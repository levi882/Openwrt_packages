use super::*;
use crate::archive::{inspect_backup, merge_fstab, safe_name};
use crate::engine::{Jobs, MAX_LOG_BYTES, MAX_TASKS};
use crate::settings::{Options, defaults, from_uci, validate};
use crate::util::{Runner, atomic_write, read_json, save_json};
use flate2::{Compression, write::GzEncoder};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Cursor;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::{TempDir, tempdir};

fn write(path: &Path, data: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn backup(filename: &Path, members: &[(&str, &[u8])], mode: u32) {
    let compressed = GzEncoder::new(File::create(filename).unwrap(), Compression::fast());
    let mut tar = tar::Builder::new(compressed);
    for (name, contents) in members {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(mode);
        header.set_entry_type(tar::EntryType::Regular);
        if name.len() <= 100 {
            header.as_mut_bytes()[..100].fill(0);
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            tar.append(&header, Cursor::new(contents)).unwrap();
        } else {
            tar.append_data(&mut header, name, Cursor::new(contents))
                .unwrap();
        }
    }
    tar.into_inner().unwrap().finish().unwrap();
}

fn link_backup(filename: &Path, target: &str) {
    let mut tar = tar::Builder::new(GzEncoder::new(
        File::create(filename).unwrap(),
        Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(6);
    header.set_mode(0o644);
    tar.append_data(&mut header, "etc/config/system", Cursor::new(b"system"))
        .unwrap();
    let mut link = tar::Header::new_gnu();
    link.set_size(0);
    link.set_mode(0o777);
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_link_name(target).unwrap();
    tar.append_data(&mut link, "root/link", std::io::empty())
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap();
}

fn minimal_options() -> Options {
    let mut options = Options::defaults();
    options.install_packages = vec!["curl".to_owned()];
    options.myfeed_packages.clear();
    options.optional_packages.clear();
    options.remove_packages.clear();
    options.iptv_enable = false;
    options.reboot = false;
    options
}

fn seed_local_feed(fixture: &mut Fixture) -> PathBuf {
    fixture.options.local_feed_dir = "/mnt/storage/restore-feed".into();
    fixture.options.restore_source = "local".into();
    let base = fixture.root.join("mnt/storage/restore-feed");
    let snapshot = "a".repeat(32);
    let store = base.join("snapshots").join(&snapshot);
    write(&store.join("cache/curl-test.apk"), b"fixture package");
    write(
        &store.join("repositories.list"),
        format!(
            "{}\n@myfeed {}\n",
            fixture.options.myfeed_repo, fixture.options.myfeed_repo
        ),
    );
    let manifest = json!({"format": 1, "created": 1, "arch": "x86_64", "myfeed": fixture.options.myfeed_repo,
        "packages": ["curl", "overlay-restore@myfeed>=0.2.0-r16", "luci-app-overlay-restore@myfeed>=0.2.0-r19", "luci-base"],
        "warnings": [], "files": {"curl-test.apk": util::digest_file(&store.join("cache/curl-test.apk")).unwrap(),
            "../repositories.list": util::digest_file(&store.join("repositories.list")).unwrap()}});
    save_json(&store.join("manifest.json"), &manifest).unwrap();
    write(&base.join("current"), snapshot);
    store
}

fn mock_profile(fixture: &Fixture, options: &Options) {
    let mut text = String::from("overlay_restore.main=restore\n");
    for (key, value) in serde_json::to_value(options).unwrap().as_object().unwrap() {
        let value = if let Some(values) = value.as_array() {
            values
                .iter()
                .map(|v| util::quote(v.as_str().unwrap()))
                .collect::<Vec<_>>()
                .join(" ")
        } else if let Some(value) = value.as_str() {
            util::quote(value)
        } else {
            value.to_string()
        };
        text.push_str(&format!("overlay_restore.main.{key}={value}\n"));
    }
    fixture.runner.0.lock().unwrap().uci_show = Some(text);
}

#[test]
fn selecting_online_recovery_keeps_the_cache_directory_without_using_it() {
    let mut fixture = Fixture::new();
    seed_local_feed(&mut fixture);
    fixture.options.restore_source = "online".into();
    fs::remove_file(fixture.root.join("mnt/storage/restore-feed/current")).unwrap();
    let id = fixture.applied();
    let plan: archive::Plan =
        read_json(&fixture.jobs.path(&id).unwrap().join("plan.json")).unwrap();
    assert!(plan.local_feed_snapshot.is_empty());
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
    assert!(
        !fixture
            .root
            .join("etc/overlay-restore-bootstrap/local-feed-sync.json")
            .exists()
    );
    assert!(
        fixture
            .runner
            .0
            .lock()
            .unwrap()
            .commands
            .iter()
            .filter(|a| a[0] == "apk")
            .all(|a| !a.iter().any(|v| v == "--no-network"))
    );
}

#[test]
fn source_selection_preserves_existing_profiles_and_old_frozen_jobs() {
    let old =
        "overlay_restore.main=restore\noverlay_restore.main.local_feed_dir='/mnt/disk/feed'\n";
    assert!(from_uci(old, None).unwrap().uses_local_feed());
    assert!(
        !from_uci(
            &(old.to_owned() + "overlay_restore.main.restore_source='online'\n"),
            None
        )
        .unwrap()
        .uses_local_feed()
    );
    let mut json = serde_json::to_value(Options::defaults()).unwrap();
    json["local_feed_dir"] = json!("/mnt/disk/feed");
    json.as_object_mut().unwrap().remove("restore_source");
    json.as_object_mut().unwrap().remove("local_feed_sync");
    let frozen: Options = serde_json::from_value(json).unwrap();
    assert!(frozen.uses_local_feed());
    assert!(frozen.feed_sync_seconds().is_none());
    let mut invalid = defaults();
    invalid["restore_source"] = json!("local");
    assert!(validate(&invalid).is_err());
    invalid["local_feed_dir"] = json!("/mnt/disk/feed");
    invalid["local_feed_sync"] = json!("every-second");
    assert!(validate(&invalid).is_err());
}

#[test]
fn weekly_cache_sync_waits_for_success_and_survives_task_cleanup() {
    let mut fixture = Fixture::new();
    seed_local_feed(&mut fixture);
    mock_profile(&fixture, &fixture.options);
    let id = fixture.applied();
    let sync = local_feed::status(&fixture.jobs, &fixture.options).unwrap()["sync"].clone();
    assert_eq!(sync["status"], "waiting_recovery");
    local_feed::tick(&fixture.jobs, u64::MAX).unwrap();
    assert!(
        !fixture
            .root
            .join("etc/overlay-restore-bootstrap/local-feed.json")
            .exists()
    );
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
    let sync = local_feed::status(&fixture.jobs, &fixture.options).unwrap()["sync"].clone();
    assert_eq!(sync["status"], "enabled");
    let due = sync["next_sync"].as_u64().unwrap();
    let schedule: serde_json::Value = read_json(
        &fixture
            .root
            .join("etc/overlay-restore-bootstrap/local-feed-sync.json"),
    )
    .unwrap();
    assert_eq!(due, schedule["armed_at"].as_u64().unwrap() + 7 * 86400);
    fixture.jobs.remove(&id, &id).unwrap();
    local_feed::tick(&fixture.jobs, due - 1).unwrap();
    assert!(
        !fixture
            .root
            .join("etc/overlay-restore-bootstrap/local-feed.json")
            .exists()
    );
    let before = fixture.runner.0.lock().unwrap().commands.len();
    local_feed::tick(&fixture.jobs, due).unwrap();
    local_feed::tick(&fixture.jobs, due).unwrap();
    let state: serde_json::Value = read_json(
        &fixture
            .root
            .join("etc/overlay-restore-bootstrap/local-feed.json"),
    )
    .unwrap();
    assert_eq!(state["status"], "queued");
    assert_eq!(state["automatic"], true);
    assert!(
        fixture.runner.0.lock().unwrap().commands[before..]
            .iter()
            .all(|a| a[0] != "apk")
    );
}

#[test]
fn scheduled_sync_pauses_when_disabled_or_recovery_is_pending() {
    let mut fixture = Fixture::new();
    seed_local_feed(&mut fixture);
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    let due = local_feed::status(&fixture.jobs, &fixture.options).unwrap()["sync"]["next_sync"]
        .as_u64()
        .unwrap();
    let mut disabled = fixture.options.clone();
    disabled.local_feed_sync = "off".into();
    mock_profile(&fixture, &disabled);
    local_feed::tick(&fixture.jobs, due).unwrap();
    assert!(
        !fixture
            .root
            .join("etc/overlay-restore-bootstrap/local-feed.json")
            .exists()
    );
    mock_profile(&fixture, &fixture.options);
    let mut state = fixture.jobs.load(&id).unwrap();
    state["status"] = json!("failed_packages");
    fixture.jobs.save(&mut state).unwrap();
    local_feed::tick(&fixture.jobs, due).unwrap();
    assert!(
        !fixture
            .root
            .join("etc/overlay-restore-bootstrap/local-feed.json")
            .exists()
    );
    state["status"] = json!("complete");
    fixture.jobs.save(&mut state).unwrap();
    local_feed::tick(&fixture.jobs, due).unwrap();
    state["status"] = json!("queued");
    fixture.jobs.save(&mut state).unwrap();
    local_feed::work(&fixture.jobs).unwrap();
    assert_eq!(
        local_feed::status(&fixture.jobs, &fixture.options).unwrap()["status"],
        "queued"
    );
    assert!(
        !fixture
            .runner
            .0
            .lock()
            .unwrap()
            .commands
            .iter()
            .any(|a| a.iter().any(|v| v == "cache"))
    );
}

#[test]
fn failed_scheduled_sync_retains_cache_and_retries_after_an_hour() {
    let mut fixture = Fixture::new();
    seed_local_feed(&mut fixture);
    fixture.applied();
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    mock_profile(&fixture, &fixture.options);
    let schedule = fixture
        .root
        .join("etc/overlay-restore-bootstrap/local-feed-sync.json");
    let mut state: serde_json::Value = read_json(&schedule).unwrap();
    state["armed_at"] = json!(1);
    save_json(&schedule, &state).unwrap();
    local_feed::tick(&fixture.jobs, util::now()).unwrap();
    local_feed::work(&fixture.jobs).unwrap();
    assert_eq!(
        local_feed::status(&fixture.jobs, &fixture.options).unwrap()["status"],
        "failed"
    );
    let state: serde_json::Value = read_json(&schedule).unwrap();
    assert!(state["next_attempt"].as_u64().unwrap() >= util::now() + 3595);
    assert!(!state["last_error"].as_str().unwrap().is_empty());
    local_feed::tick(&fixture.jobs, util::now()).unwrap();
    assert_eq!(
        local_feed::status(&fixture.jobs, &fixture.options).unwrap()["status"],
        "failed"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("mnt/storage/restore-feed/current")).unwrap(),
        "a".repeat(32)
    );
    assert!(
        local_feed::select(
            &fixture.jobs,
            &fixture.options,
            &fixture.options.myfeed_repo
        )
        .is_ok()
    );
}

#[test]
fn local_recovery_is_offline_and_preserves_the_online_feed_and_frozen_snapshot() {
    let mut fixture = Fixture::new();
    let store = seed_local_feed(&mut fixture);
    let repo = fixture.root.join("etc/apk/repositories.d/00-myfeed.list");
    let original = format!("# CF\n{}\n", fixture.options.myfeed_repo);
    write(&repo, &original);
    let id = fixture.applied();
    // Updating the current pointer cannot move a previously inspected task.
    write(
        &fixture.root.join("mnt/storage/restore-feed/current"),
        "b".repeat(32),
    );
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
    assert_eq!(fs::read_to_string(repo).unwrap(), original);
    let commands = &fixture.runner.0.lock().unwrap().commands;
    assert!(commands.iter().any(|args| args.iter().any(|a| a == "add")));
    for args in commands.iter().filter(|args| args[0] == "apk") {
        assert!(args.iter().any(|a| a == "--no-network"));
        assert!(
            args.iter()
                .any(|a| a == &store.join("cache").to_string_lossy())
        );
        assert!(
            !args
                .iter()
                .any(|a| a == "--allow-untrusted" || a == "--force-broken-world")
        );
    }
}

#[test]
fn incomplete_or_changed_local_feed_blocks_migration_before_configuration_changes() {
    let mut fixture = Fixture::new();
    let store = seed_local_feed(&mut fixture);
    let id = fixture.ready();
    write(&store.join("cache/curl-test.apk"), b"tampered");
    let system = fixture.root.join("etc/config/system");
    write(&system, b"original");
    fixture.jobs.apply(&id, &id, None).unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "failed_prepare");
    assert_eq!(fs::read(system).unwrap(), b"original");
    assert!(
        fixture
            .runner
            .0
            .lock()
            .unwrap()
            .commands
            .iter()
            .all(|a| !a.iter().any(|s| s == "add"))
    );
}

#[test]
fn local_feed_with_an_older_recovery_backend_requires_refresh() {
    let mut fixture = Fixture::new();
    let store = seed_local_feed(&mut fixture);
    let manifest_path = store.join("manifest.json");
    let mut manifest: serde_json::Value = read_json(&manifest_path).unwrap();
    for package in manifest["packages"].as_array_mut().unwrap() {
        if package.as_str().unwrap().starts_with("overlay-restore@") {
            *package = json!("overlay-restore@myfeed>=0.2.0-r13");
        }
    }
    save_json(&manifest_path, &manifest).unwrap();
    let system = fixture.root.join("etc/config/system");
    write(&system, b"original");
    let task = fixture
        .jobs
        .prepare(&fixture.backup, &fixture.options)
        .unwrap();
    let id = task["id"].as_str().unwrap();
    fixture.jobs.worker(true).unwrap();
    let failed = fixture.jobs.load(id).unwrap();
    assert_eq!(failed["status"], "failed_validation");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("recovery tool requirements")
    );
    assert_eq!(fs::read(system).unwrap(), b"original");
}

#[test]
fn local_feed_selection_requires_cached_packages_and_rejects_directory_aliases() {
    let mut fixture = Fixture::new();
    let store = seed_local_feed(&mut fixture);
    let feed = fixture.options.myfeed_repo.clone();
    fixture.options.install_packages.push("bash".into());
    assert!(local_feed::select(&fixture.jobs, &fixture.options, &feed).is_err());
    fixture.options.install_packages.pop();
    fs::rename(store.join("cache"), store.join("original-cache")).unwrap();
    symlink("original-cache", store.join("cache")).unwrap();
    assert!(local_feed::select(&fixture.jobs, &fixture.options, &feed).is_err());
    for directory in [
        "/etc/feed",
        "/tmp/feed",
        "/mnt/disk/../feed",
        "/mnt/disk/upper/feed",
        "/mnt/disk/$feed",
        "/mnt/disk/my feed",
    ] {
        assert!(
            local_feed::valid_directory(directory).is_err(),
            "{directory}"
        );
    }
}

#[test]
fn failed_local_feed_refresh_retains_the_last_ready_snapshot() {
    let mut fixture = Fixture::new();
    seed_local_feed(&mut fixture);
    write(
        &fixture
            .root
            .join("etc/overlay-restore-bootstrap/myfeed.pem"),
        b"trusted key",
    );
    local_feed::queue(&fixture.jobs, &fixture.options).unwrap();
    // The mock downloader produces no packages, just like an incomplete cache.
    local_feed::work(&fixture.jobs).unwrap();
    assert_eq!(
        local_feed::status(&fixture.jobs, &fixture.options).unwrap()["status"],
        "failed"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("mnt/storage/restore-feed/current")).unwrap(),
        "a".repeat(32)
    );
    assert_eq!(
        local_feed::select(
            &fixture.jobs,
            &fixture.options,
            &fixture.options.myfeed_repo
        )
        .unwrap(),
        "a".repeat(32)
    );
}

#[test]
fn restored_fstab_keeps_the_local_feed_disk_and_drops_conflicting_old_mounts() {
    let current = "config mount 'feed'\n option target '/mnt/disk'\n option uuid 'disk-current'\n option enabled '1'\nconfig mount 'extroot'\n option target '/overlay'\n option uuid 'extroot-current'\n";
    let old = "config mount 'feed'\n option target '/mnt/old-name'\n option uuid 'disk-old'\nconfig mount 'alias'\n option target '/mnt/alias'\n option uuid 'disk-current'\nconfig mount 'other'\n option target '/mnt/data'\n option uuid 'other-disk'\nconfig mount 'root'\n option target '/overlay'\n option uuid 'extroot-old'\n";
    let merged =
        archive::merge_recovery_fstab(old, current, false, "/mnt/disk/restore-feed").unwrap();
    assert!(
        merged.contains("/mnt/disk")
            && merged.contains("other-disk")
            && merged.contains("extroot-old")
    );
    assert!(
        !merged.contains("disk-old")
            && !merged.contains("/mnt/alias")
            && !merged.contains("extroot-current")
    );
}

#[derive(Default)]
struct MockState {
    uci_show: Option<String>,
    installed: HashSet<String>,
    versions: BTreeMap<String, String>,
    available_versions: BTreeMap<String, String>,
    failed: HashSet<String>,
    dependents: BTreeMap<String, Vec<String>>,
    dependencies: BTreeMap<String, Vec<String>>,
    failed_probes: HashSet<String>,
    remove_exit_code: i32,
    commands: Vec<Vec<String>>,
    settings: BTreeMap<String, String>,
}

#[derive(Default)]
struct MockRunner(Mutex<MockState>);
impl Runner for MockRunner {
    fn execute(&self, arguments: &[String], _: Duration) -> (i32, String) {
        let mut state = self.0.lock().unwrap();
        state.commands.push(arguments.to_vec());
        if arguments[0] == "apk" {
            let package = arguments
                .last()
                .unwrap()
                .split('@')
                .next()
                .unwrap()
                .to_owned();
            if arguments.iter().any(|argument| argument == "info") {
                if state.failed_probes.contains(&package) {
                    return (1, "Command timed out".to_owned());
                }
                return (
                    i32::from(!state.installed.contains(&package)),
                    String::new(),
                );
            }
            if let Some(position) = arguments.iter().position(|argument| argument == "list") {
                let selected: Vec<_> = arguments[position + 1..]
                    .iter()
                    .filter(|argument| !argument.starts_with('-'))
                    .collect();
                let output = selected
                    .iter()
                    .filter(|package| state.installed.contains(package.as_str()))
                    .map(|package| {
                        format!(
                            "{}-{} noarch [installed]\n",
                            package,
                            state
                                .versions
                                .get(package.as_str())
                                .map(String::as_str)
                                .unwrap_or("1.0")
                        )
                    })
                    .collect::<String>();
                return (0, output);
            }
            if let Some(position) = arguments.iter().position(|argument| argument == "add") {
                let requested: Vec<_> = arguments[position + 1..]
                    .iter()
                    .filter(|argument| !argument.starts_with('-'))
                    .map(|argument| {
                        argument
                            .split(['@', '<', '>', '=', '~'])
                            .next()
                            .unwrap()
                            .to_owned()
                    })
                    .collect();
                if requested
                    .iter()
                    .any(|package| state.failed.contains(package))
                {
                    return (1, "repository download failed".to_owned());
                }
                let mut selected = requested.clone();
                for package in &requested {
                    selected.extend(state.dependencies.get(package).cloned().unwrap_or_default());
                }
                for name in selected {
                    if (!state.installed.contains(&name)
                        || arguments.iter().any(|argument| argument == "--upgrade"))
                        && let Some(version) = state.available_versions.get(&name).cloned()
                    {
                        state.versions.insert(name.clone(), version);
                    }
                    state.installed.insert(name);
                }
            } else if let Some(position) = arguments.iter().position(|argument| argument == "del") {
                let requested: HashSet<_> = arguments[position + 1..].iter().cloned().collect();
                let mut removable = requested.clone();
                loop {
                    let blocked: Vec<_> = removable
                        .iter()
                        .filter(|package| {
                            state.dependents.get(*package).is_some_and(|dependents| {
                                dependents.iter().any(|dependent| {
                                    state.installed.contains(dependent)
                                        && !removable.contains(dependent)
                                })
                            })
                        })
                        .cloned()
                        .collect();
                    if blocked.is_empty() {
                        break;
                    }
                    for package in blocked {
                        removable.remove(&package);
                    }
                }
                for package in &removable {
                    state.installed.remove(package);
                }
                let retained: Vec<_> = requested.difference(&removable).cloned().collect();
                return (
                    state.remove_exit_code,
                    if retained.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "Packages retained due to dependencies: {}",
                            retained.join(" ")
                        )
                    },
                );
            }
        }
        if arguments[0] == "uci" {
            if arguments.get(2).is_some_and(|argument| argument == "show") {
                return (
                    0,
                    state
                        .uci_show
                        .clone()
                        .unwrap_or_else(|| "overlay_restore.main=restore\n".to_owned()),
                );
            }
            if arguments.get(2).is_some_and(|argument| argument == "get") {
                return (
                    0,
                    state
                        .settings
                        .get(arguments.last().unwrap())
                        .cloned()
                        .unwrap_or_default(),
                );
            }
        }
        (0, String::new())
    }
}

struct Fixture {
    _temporary: TempDir,
    root: PathBuf,
    jobs: Jobs,
    runner: Arc<MockRunner>,
    options: Options,
    backup: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempdir().unwrap();
        let root = temporary.path().to_owned();
        let runner = Arc::new(MockRunner::default());
        let mut jobs = Jobs::fixture(&root, runner.clone()).unwrap();
        jobs.boot_id = Some("before".to_owned());
        let filename = root.join("backup.tar.gz");
        backup(
            &filename,
            &[(
                "etc/config/system",
                b"config system\n option hostname 'restored'\n",
            )],
            0o644,
        );
        Self {
            _temporary: temporary,
            root,
            jobs,
            runner,
            options: minimal_options(),
            backup: filename,
        }
    }
    fn ready(&self) -> String {
        let state = self.jobs.prepare(&self.backup, &self.options).unwrap();
        let id = state["id"].as_str().unwrap().to_owned();
        self.jobs.worker(true).unwrap();
        assert_eq!(self.jobs.load(&id).unwrap()["status"], "ready");
        id
    }
    fn applied(&self) -> String {
        let id = self.ready();
        self.jobs.apply(&id, &id, None).unwrap();
        self.jobs.worker(true).unwrap();
        assert_eq!(self.jobs.load(&id).unwrap()["status"], "awaiting_reboot");
        id
    }
}

fn history_record(fixture: &Fixture, index: usize, status: &str) -> String {
    let id = format!("{index:032x}");
    save_json(
        &fixture.jobs.path(&id).unwrap().join("state.json"),
        &json!({
            "id": id, "status": status, "created": index, "updated": index,
            "completed_files": [], "packages": {}, "boot_attempts": 0
        }),
    )
    .unwrap();
    id
}

#[test]
fn cleanup_removes_finished_task_data_and_preserves_pending_recovery() {
    let fixture = Fixture::new();
    let backup_before = fs::read(&fixture.backup).unwrap();
    let current = fixture.root.join("etc/config/system");
    write(&current, "current configuration");
    let statuses = [
        "complete",
        "complete_with_warnings",
        "failed_validation",
        "ready",
        "failed_prepare",
        "failed_apply",
        "failed_packages",
        "validating",
        "queued",
        "preparing_packages",
        "applying",
        "awaiting_reboot",
        "installing",
    ];
    let ids: Vec<_> = statuses
        .iter()
        .enumerate()
        .map(|(index, status)| {
            let id = history_record(&fixture, index, status);
            let directory = fixture.jobs.path(&id).unwrap();
            write(
                &directory.join("originals/etc/config/system"),
                "saved original",
            );
            write(&directory.join("payload/etc/config/system"), "pending data");
            write(&directory.join("task.log"), "diagnostics");
            write(
                &fixture.jobs.temporary.join(&id).join("backup.tar.gz"),
                "temporary copy",
            );
            id
        })
        .collect();
    let result = fixture.jobs.cleanup().unwrap();
    assert_eq!(result["removed"].as_array().unwrap().len(), 3);
    for (index, id) in ids.iter().enumerate() {
        assert_eq!(fixture.jobs.path(id).unwrap().exists(), index >= 3);
        assert_eq!(fixture.jobs.temporary.join(id).exists(), index >= 3);
        if index >= 4 {
            assert!(fixture.jobs.remove(id, id).is_err());
        }
    }
    assert_eq!(fs::read(&fixture.backup).unwrap(), backup_before);
    assert_eq!(
        fs::read_to_string(current).unwrap(),
        "current configuration"
    );
}

#[test]
fn unused_preview_removal_requires_confirmation_and_only_deletes_task_copies() {
    let fixture = Fixture::new();
    let backup_before = fs::read(&fixture.backup).unwrap();
    let id = fixture.ready();
    assert!(fixture.jobs.remove(&id, "wrong").is_err());
    assert!(fixture.jobs.remove("../outside", "../outside").is_err());
    assert!(fixture.jobs.path(&id).unwrap().exists());
    fixture.jobs.remove(&id, &id).unwrap();
    assert!(!fixture.jobs.path(&id).unwrap().exists());
    assert!(!fixture.jobs.temporary.join(&id).exists());
    assert_eq!(fs::read(&fixture.backup).unwrap(), backup_before);
    assert!(crate::rpc::call(&fixture.jobs, "remove", &json!({"id": id})).is_err());
    assert!(crate::rpc::call(&fixture.jobs, "cleanup", &json!({"path": "/"})).is_err());
}

#[test]
fn cleanup_skips_task_writers_and_validator_locks() {
    let fixture = Fixture::new();
    let id = history_record(&fixture, 1, "complete");
    let writer = fixture.jobs.lock(&format!("task-{id}"), false).unwrap();
    assert_eq!(fixture.jobs.cleanup().unwrap()["skipped_busy"], 1);
    assert!(fixture.jobs.remove(&id, &id).is_err());
    drop(writer);
    let validator = fixture.jobs.lock(&format!("validate-{id}"), false).unwrap();
    assert_eq!(fixture.jobs.cleanup().unwrap()["skipped_busy"], 1);
    drop(validator);
    assert_eq!(fixture.jobs.cleanup().unwrap()["removed"], json!([id]));
}

#[test]
fn task_limit_requires_manual_cleanup_without_pruning_existing_records() {
    let fixture = Fixture::new();
    for index in 0..MAX_TASKS {
        history_record(&fixture, index, "complete");
    }
    let list = crate::rpc::list(&fixture.jobs).unwrap();
    assert_eq!(list["tasks"].as_array().unwrap().len(), 10);
    assert_eq!(list["total_tasks"], MAX_TASKS);
    assert_eq!(list["max_tasks"], MAX_TASKS);
    assert!(
        fixture
            .jobs
            .prepare(&fixture.backup, &fixture.options)
            .unwrap_err()
            .to_string()
            .contains("task limit")
    );
    assert_eq!(fixture.jobs.states().unwrap().len(), MAX_TASKS);
    history_record(&fixture, MAX_TASKS, "complete");
    assert!(
        fixture
            .jobs
            .prepare(&fixture.backup, &fixture.options)
            .is_err()
    );
    assert_eq!(fixture.jobs.states().unwrap().len(), MAX_TASKS + 1);
    fixture.jobs.cleanup().unwrap();
    assert!(
        fixture
            .jobs
            .prepare(&fixture.backup, &fixture.options)
            .is_ok()
    );
}

#[test]
fn cleanup_rejects_directory_aliases_and_usage_does_not_follow_them() {
    let fixture = Fixture::new();
    let id = history_record(&fixture, 1, "complete");
    let external = fixture.root.join("outside");
    write(&external.join("marker"), vec![b'x'; 65536]);
    let before = fixture.jobs.usage().unwrap();
    symlink(&external, fixture.jobs.path(&id).unwrap().join("external")).unwrap();
    symlink(&external, fixture.jobs.temporary.join(&id)).unwrap();
    let after = fixture.jobs.usage().unwrap();
    assert_eq!(before["task_bytes"], after["task_bytes"]);
    assert_eq!(after["temporary_bytes"], 0);
    assert!(fixture.jobs.remove(&id, &id).is_err());
    assert!(fixture.jobs.path(&id).unwrap().exists());
    assert_eq!(fs::read(external.join("marker")).unwrap().len(), 65536);
    fs::remove_file(fixture.jobs.temporary.join(&id)).unwrap();
    let other_id = format!("{:032x}", 2);
    save_json(
        &external.join("state.json"),
        &json!({"id": other_id, "status": "complete"}),
    )
    .unwrap();
    symlink(&external, fixture.jobs.path(&other_id).unwrap()).unwrap();
    assert!(fixture.jobs.remove(&other_id, &other_id).is_err());
    assert!(external.join("state.json").exists());
}

#[test]
fn log_limit_retains_recent_unicode_output_and_serializes_writers() {
    let fixture = Fixture::new();
    let id = history_record(&fixture, 1, "complete");
    fixture.jobs.log(&id, &"旧日志\n".repeat(400000)).unwrap();
    std::thread::scope(|scope| {
        for writer in 0..4 {
            let fixture = &fixture;
            let id = &id;
            scope.spawn(move || {
                for index in 0..30 {
                    fixture
                        .jobs
                        .log(
                            id,
                            &format!("writer-{writer}-{index}:{}", "测".repeat(6000)),
                        )
                        .unwrap();
                }
            });
        }
    });
    fixture.jobs.log(&id, "latest diagnostic").unwrap();
    let path = fixture.jobs.path(&id).unwrap().join("task.log");
    let contents = fs::read_to_string(&path).unwrap();
    assert!(contents.len() as u64 <= MAX_LOG_BYTES);
    assert!(contents.ends_with("latest diagnostic\n"));
    for line in contents.lines().filter(|line| line.contains("writer-")) {
        assert_eq!(line.matches("writer-").count(), 1);
        assert_eq!(line.matches('测').count(), 6000);
    }
    assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
}

#[test]
fn worker_bounds_legacy_logs_without_removing_history() {
    let fixture = Fixture::new();
    let id = history_record(&fixture, 1, "complete");
    let log = fixture.jobs.path(&id).unwrap().join("task.log");
    write(&log, "old record\n".repeat(300000));
    fixture.jobs.worker(true).unwrap();
    assert!(log.metadata().unwrap().len() <= MAX_LOG_BYTES);
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
    assert!(fs::read_to_string(log).unwrap().contains("1 MiB limit"));
}

#[test]
fn log_rotation_rejects_symlinks_and_special_files() {
    let fixture = Fixture::new();
    let id = history_record(&fixture, 1, "complete");
    let marker = fixture.root.join("marker");
    write(&marker, "original");
    let path = fixture.jobs.path(&id).unwrap().join("task.log");
    symlink(&marker, &path).unwrap();
    assert!(fixture.jobs.log(&id, "overwrite").is_err());
    assert_eq!(fs::read_to_string(&marker).unwrap(), "original");
    fs::remove_file(&path).unwrap();
    let name = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    // SAFETY: name points to a NUL-terminated path in this test's private directory.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(fixture.jobs.log(&id, "special file").is_err());
}

#[test]
fn overlay_layout_prunes_packaged_and_runtime_files() {
    let fixture = Fixture::new();
    backup(
        &fixture.backup,
        &[
            ("overlay/upper/etc/config/system", b"config system\n"),
            ("overlay/upper/root/note", b"custom"),
            ("overlay/upper/usr/bin/old", b"program"),
            ("overlay/upper/usr/bin/custom", b"user program"),
            ("overlay/upper/lib/apk/packages/old.list", b"/usr/bin/old\n"),
            ("overlay/upper/etc/apk/keys/myfeed.pem", b"old key"),
            (
                "overlay/upper/etc/overlay-restore-bootstrap/run",
                b"old bootstrap",
            ),
            (
                "overlay/upper/etc/overlay-restore-bootstrap/myfeed.pem",
                b"old bootstrap key",
            ),
            ("overlay/upper/lib/modules/old/kernel.ko", b"module"),
            (
                "overlay/upper/www/luci-static/resources/view/system/overlay-restore.js",
                b"old UI",
            ),
            ("overlay/upper/etc/config/.wh.deleted", b""),
        ],
        0o644,
    );
    let plan = inspect_backup(
        &fixture.backup,
        &fixture.options,
        &fixture.root.join("payload"),
    )
    .unwrap();
    assert_eq!(plan.layout, "overlay");
    assert!(plan.metadata_available);
    assert_eq!(
        plan.files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        ["etc/config/system", "root/note", "usr/bin/custom"]
    );
}

#[test]
fn old_apk_database_identifies_program_ownership() {
    let fixture = Fixture::new();
    backup(
        &fixture.backup,
        &[
            ("upper/etc/config/system", b"system"),
            ("upper/usr/bin/old", b"binary"),
            ("upper/lib/apk/db/installed", b"P:old\nF:usr/bin\nR:old\n"),
        ],
        0o644,
    );
    let plan = inspect_backup(
        &fixture.backup,
        &fixture.options,
        &fixture.root.join("payload"),
    )
    .unwrap();
    assert_eq!(plan.layout, "upper");
    assert!(plan.metadata_available);
    assert_eq!(plan.files.len(), 1);
}

#[test]
fn opkg_file_lists_identify_program_ownership() {
    let fixture = Fixture::new();
    backup(
        &fixture.backup,
        &[
            ("upper/etc/config/system", b"system"),
            ("upper/usr/bin/old", b"binary"),
            ("upper/usr/lib/opkg/info/old.list", b"/usr/bin/old\n"),
        ],
        0o644,
    );
    let plan = inspect_backup(
        &fixture.backup,
        &fixture.options,
        &fixture.root.join("payload"),
    )
    .unwrap();
    assert!(plan.metadata_available);
    assert_eq!(plan.files.len(), 1);
}

#[test]
fn sysupgrade_and_credential_network_options() {
    let fixture = Fixture::new();
    let mut options = fixture.options.clone();
    options.restore_credentials = false;
    options.keep_network = true;
    backup(
        &fixture.backup,
        &[
            ("etc/config/system", b"system"),
            ("etc/config/network", b"network"),
            ("etc/shadow", b"secret"),
            ("root/.ssh/authorized_keys", b"key"),
        ],
        0o644,
    );
    let plan = inspect_backup(&fixture.backup, &options, &fixture.root.join("payload")).unwrap();
    assert_eq!(plan.layout, "sysupgrade");
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].path, "etc/config/system");
}

#[test]
fn unix_backslash_names_survive_inspection_and_migration() {
    let fixture = Fixture::new();
    let members: &[(&str, &[u8])] = &[
        ("overlay/upper/etc/config/system", b"config system\n"),
        ("overlay/upper/root/ \\", b"literal backslash"),
        ("overlay/upper/root/dir\\name/note", b"backslash directory"),
        ("overlay/upper/root/dir/name/note", b"ordinary directories"),
        ("overlay/upper/root/\\../note", b"literal parent name"),
    ];
    backup(&fixture.backup, members, 0o640);
    let current = fixture.root.join("etc/config/system");
    write(&current, "current configuration");
    let id = fixture.ready();
    assert_eq!(
        fs::read_to_string(&current).unwrap(),
        "current configuration"
    );
    let task = fixture.jobs.public(&id, true).unwrap();
    assert_eq!(task["plan"]["file_count"], members.len());
    for (name, _) in members {
        let path = name.strip_prefix("overlay/upper/").unwrap();
        assert!(
            task["plan"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["path"] == path && entry["archive_path"] == *name)
        );
    }
    fixture.jobs.apply(&id, &id, None).unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "awaiting_reboot");
    for (name, contents) in members {
        let path = fixture
            .root
            .join(name.strip_prefix("overlay/upper/").unwrap());
        assert_eq!(fs::read(&path).unwrap(), *contents);
        assert_eq!(path.metadata().unwrap().mode() & 0o777, 0o640);
    }
}

#[test]
fn unix_backslash_link_targets_are_skipped() {
    for target in ["dir\\name/note", "\\../note", "/tmp/back\\slash"] {
        let fixture = Fixture::new();
        link_backup(&fixture.backup, target);
        let plan = inspect_backup(
            &fixture.backup,
            &fixture.options,
            &fixture.root.join("payload"),
        )
        .unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.skipped["links and directories"], 1);
        assert!(!fixture.root.join("payload/root/link").exists());
    }
}

#[test]
fn traversal_absolute_and_control_names_are_rejected() {
    for name in [
        "../../etc/shadow",
        "/etc/shadow",
        "root/../etc/shadow",
        "root/dir\\name/../etc/shadow",
        "root/line\n",
        "root/tab\t",
        "root/nul\0",
    ] {
        assert!(safe_name(name).is_err(), "{name:?}");
    }
    for name in [
        "../etc/config/system",
        "/etc/config/system",
        "root/../etc/config/system",
        "root/dir\\name/../etc/config/system",
        "root/line\n",
    ] {
        let fixture = Fixture::new();
        backup(
            &fixture.backup,
            &[("etc/config/system", b"system"), (name, b"bad")],
            0o644,
        );
        assert!(
            inspect_backup(
                &fixture.backup,
                &fixture.options,
                &fixture.root.join("payload")
            )
            .is_err()
        );
    }
}

#[test]
fn duplicate_paths_and_file_parent_conflicts_are_rejected() {
    for names in [
        ["etc/config/system", "./etc/config/system"],
        ["etc/config/system", "etc//config/system"],
        ["etc/config/system", "etc/config/system/child"],
    ] {
        let fixture = Fixture::new();
        backup(
            &fixture.backup,
            &[(names[0], b"a"), (names[1], b"b")],
            0o644,
        );
        assert!(
            inspect_backup(
                &fixture.backup,
                &fixture.options,
                &fixture.root.join("payload")
            )
            .is_err()
        );
    }
}

#[test]
fn links_are_skipped_and_escaping_links_are_rejected() {
    let fixture = Fixture::new();
    link_backup(&fixture.backup, "/tmp/outside");
    let plan = inspect_backup(
        &fixture.backup,
        &fixture.options,
        &fixture.root.join("payload"),
    )
    .unwrap();
    assert_eq!(plan.files.len(), 1);
    assert!(!fixture.root.join("payload/root/link").exists());
    fs::remove_dir_all(fixture.root.join("payload")).unwrap();
    for target in ["../../outside", "dir\\name/../../../outside"] {
        link_backup(&fixture.backup, target);
        assert!(
            inspect_backup(
                &fixture.backup,
                &fixture.options,
                &fixture.root.join("payload")
            )
            .is_err()
        );
    }
}

#[test]
fn gzip_crc_corruption_is_rejected() {
    let fixture = Fixture::new();
    let mut bytes = fs::read(&fixture.backup).unwrap();
    let index = bytes.len() - 8;
    bytes[index] ^= 0xff;
    fs::write(&fixture.backup, bytes).unwrap();
    assert!(
        inspect_backup(
            &fixture.backup,
            &fixture.options,
            &fixture.root.join("payload")
        )
        .is_err()
    );
}

#[test]
fn expanded_size_limit_includes_unselected_members_and_trailers() {
    let mut fixture = Fixture::new();
    fixture.options.max_expanded_mb = 8;
    backup(
        &fixture.backup,
        &[
            ("etc/config/system", b"system"),
            ("tmp/skipped", &vec![0; 9 * 1024 * 1024]),
        ],
        0o644,
    );
    assert!(
        inspect_backup(
            &fixture.backup,
            &fixture.options,
            &fixture.root.join("payload")
        )
        .is_err()
    );
}

#[test]
fn oversized_extension_headers_are_rejected() {
    let fixture = Fixture::new();
    let mut tar = tar::Builder::new(GzEncoder::new(
        File::create(&fixture.backup).unwrap(),
        Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(65537);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::GNULongName);
    tar.append_data(&mut header, "././@LongLink", Cursor::new(vec![b'a'; 65537]))
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    assert!(
        inspect_backup(
            &fixture.backup,
            &fixture.options,
            &fixture.root.join("payload")
        )
        .is_err()
    );
}

#[test]
fn gnu_long_names_are_supported_within_limits() {
    let fixture = Fixture::new();
    let name = "root/".to_owned() + &"long-name-".repeat(15);
    backup(
        &fixture.backup,
        &[("etc/config/system", b"system"), (&name, b"long path")],
        0o644,
    );
    let plan = inspect_backup(
        &fixture.backup,
        &fixture.options,
        &fixture.root.join("payload"),
    )
    .unwrap();
    assert!(plan.files.iter().any(|entry| entry.path == name));
    assert_eq!(
        fs::read(fixture.root.join("payload").join(name)).unwrap(),
        b"long path"
    );
}

#[test]
fn gnu_long_unicode_names_override_truncated_header_fields() {
    let fixture = Fixture::new();
    // The 100-byte GNU fallback ends partway through a three-byte character.
    let name = format!("root/{}配置文件", "a".repeat(93));
    let mut tar = tar::Builder::new(GzEncoder::new(
        File::create(&fixture.backup).unwrap(),
        Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(6);
    header.set_mode(0o644);
    tar.append_data(&mut header, "etc/config/system", Cursor::new(b"system"))
        .unwrap();
    let complete_name = [name.as_bytes(), &[0]].concat();
    let mut extension = tar::Header::new_gnu();
    extension.set_size(complete_name.len() as u64);
    extension.set_mode(0o644);
    extension.set_entry_type(tar::EntryType::GNULongName);
    tar.append_data(&mut extension, "././@LongLink", Cursor::new(complete_name))
        .unwrap();
    let contents = b"long unicode path";
    header.set_size(contents.len() as u64);
    header.as_mut_bytes()[..100].copy_from_slice(&name.as_bytes()[..100]);
    assert!(std::str::from_utf8(&header.path_bytes()).is_err());
    header.set_cksum();
    tar.append(&header, Cursor::new(contents)).unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    let plan = inspect_backup(
        &fixture.backup,
        &fixture.options,
        &fixture.root.join("payload"),
    )
    .unwrap();
    assert!(plan.files.iter().any(|entry| entry.path == name));
    assert_eq!(
        fs::read(fixture.root.join("payload").join(name)).unwrap(),
        b"long unicode path"
    );
}

#[test]
fn pax_names_override_invalid_utf8_fallbacks_without_bypassing_validation() {
    let fixture = Fixture::new();
    for (path, succeeds) in [
        (Some("etc/config/配置"), true),
        (Some("../outside"), false),
        (None, false),
    ] {
        let mut tar = tar::Builder::new(GzEncoder::new(
            File::create(&fixture.backup).unwrap(),
            Compression::fast(),
        ));
        if let Some(path) = path {
            let record = pax_record("path", path);
            let mut header = tar::Header::new_ustar();
            header.set_size(record.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::XHeader);
            tar.append_data(&mut header, "PaxHeaders/file", Cursor::new(record))
                .unwrap();
        }
        let mut header = tar::Header::new_ustar();
        header.set_size(6);
        header.set_mode(0o644);
        header.as_mut_bytes()[0] = 0xff;
        header.set_cksum();
        tar.append(&header, Cursor::new(b"system")).unwrap();
        if succeeds {
            let records = [
                pax_record("path", "root/link"),
                pax_record("linkpath", "../etc/config/配置"),
            ]
            .concat();
            let mut extension = tar::Header::new_ustar();
            extension.set_size(records.len() as u64);
            extension.set_mode(0o644);
            extension.set_entry_type(tar::EntryType::XHeader);
            tar.append_data(&mut extension, "PaxHeaders/link", Cursor::new(records))
                .unwrap();
            let mut link = tar::Header::new_ustar();
            link.set_size(0);
            link.set_mode(0o777);
            link.set_entry_type(tar::EntryType::Symlink);
            link.as_mut_bytes()[0] = 0xff;
            link.as_mut_bytes()[157] = 0xff;
            link.set_cksum();
            tar.append(&link, std::io::empty()).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let staging = fixture.root.join(if succeeds {
            "valid-payload"
        } else {
            "invalid-payload"
        });
        let result = inspect_backup(&fixture.backup, &fixture.options, &staging);
        if succeeds {
            let plan = result.unwrap();
            assert_eq!(plan.files.len(), 1);
            assert_eq!(plan.files[0].path, "etc/config/配置");
            assert_eq!(
                fs::read(staging.join("etc/config/配置")).unwrap(),
                b"system"
            );
        } else {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(if path.is_some() {
                "Parent traversal"
            } else {
                "Invalid archive path encoding"
            }));
            assert!(!staging.exists());
        }
    }
}

fn pax_backup(filename: &Path, attributes: &[Vec<u8>]) {
    let mut tar = tar::Builder::new(GzEncoder::new(
        File::create(filename).unwrap(),
        Compression::fast(),
    ));
    for contents in attributes {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::XHeader);
        tar.append_data(&mut header, "PaxHeaders/file", Cursor::new(contents))
            .unwrap();
    }
    let mut header = tar::Header::new_gnu();
    header.set_size(6);
    header.set_mode(0o644);
    tar.append_data(&mut header, "etc/config/system", Cursor::new(b"system"))
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap();
}

fn pax_record(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut length = body.len() + 1;
    loop {
        let result = format!("{length}{body}");
        if result.len() == length {
            return result.into_bytes();
        }
        length = result.len();
    }
}

#[test]
fn pax_paths_support_unicode_and_reject_parent_traversal() {
    let fixture = Fixture::new();
    pax_backup(&fixture.backup, &[pax_record("path", "etc/config/配置")]);
    let plan = inspect_backup(
        &fixture.backup,
        &fixture.options,
        &fixture.root.join("payload"),
    )
    .unwrap();
    assert_eq!(plan.files[0].path, "etc/config/配置");
    fs::remove_dir_all(fixture.root.join("payload")).unwrap();
    pax_backup(&fixture.backup, &[pax_record("path", "../outside")]);
    assert!(
        inspect_backup(
            &fixture.backup,
            &fixture.options,
            &fixture.root.join("payload")
        )
        .is_err()
    );
}

#[test]
fn repeated_pax_headers_cannot_accumulate_unbounded_metadata() {
    let fixture = Fixture::new();
    let value = "a".repeat(40000);
    pax_backup(
        &fixture.backup,
        &[
            pax_record("ignored-one", &value),
            pax_record("ignored-two", &value),
        ],
    );
    let error = inspect_backup(
        &fixture.backup,
        &fixture.options,
        &fixture.root.join("payload"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("Combined archive extension"));
}

#[test]
fn fifo_upload_is_rejected_without_blocking() {
    let fixture = Fixture::new();
    let path = fixture.root.join("fifo");
    let filename = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    // SAFETY: filename is NUL terminated and points inside this test's private directory.
    assert_eq!(unsafe { libc::mkfifo(filename.as_ptr(), 0o600) }, 0);
    assert!(fixture.jobs.prepare(&path, &fixture.options).is_err());
}

#[test]
fn current_extroot_is_preserved_and_other_mounts_are_restored() {
    let backup = "config mount 'disk'\n option target '/mnt/data'\n\nconfig mount 'old'\n option target '/'\n option uuid 'OLD'\n";
    let current = "config mount 'new'\n option target '/overlay'\n option uuid 'NEW'\n";
    let merged = merge_fstab(backup, current).unwrap();
    assert!(merged.contains("'/mnt/data'"));
    assert!(merged.contains("'NEW'"));
    assert!(!merged.contains("'OLD'"));
}

#[test]
fn inspection_never_changes_live_configuration_or_exposes_tokens() {
    let mut fixture = Fixture::new();
    fixture.options.iptv_refresh_token = "PRIVATE_TOKEN".to_owned();
    let file = fixture.root.join("etc/config/system");
    write(&file, "original");
    let id = fixture.ready();
    assert_eq!(fs::read_to_string(file).unwrap(), "original");
    let public = fixture.jobs.public(&id, true).unwrap().to_string();
    assert!(!public.contains("PRIVATE_TOKEN"));
    assert!(!public.contains("iptv_refresh_token"));
    assert!(fixture.jobs.apply(&id, "wrong confirmation", None).is_err());
}

#[test]
fn concurrent_validators_publish_the_plan_once() {
    let fixture = Fixture::new();
    let state = fixture
        .jobs
        .prepare(&fixture.backup, &fixture.options)
        .unwrap();
    let id = state["id"].as_str().unwrap();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| fixture.jobs.validate(id));
        let second = scope.spawn(|| fixture.jobs.validate(id));
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    });
    assert_eq!(fixture.jobs.load(id).unwrap()["status"], "ready");
    let log = fs::read_to_string(fixture.jobs.path(id).unwrap().join("task.log")).unwrap();
    assert_eq!(log.matches("Validation complete:").count(), 1);
}

#[test]
fn archive_mode_is_applied_and_current_owner_is_preserved() {
    let fixture = Fixture::new();
    let destination = fixture.root.join("etc/config/system");
    write(&destination, "original");
    // SAFETY: geteuid has no preconditions; chown receives a valid CString below.
    if unsafe { libc::geteuid() } == 0 {
        let path = std::ffi::CString::new(destination.to_str().unwrap()).unwrap();
        // SAFETY: path is valid and NUL terminated.
        assert_eq!(unsafe { libc::chown(path.as_ptr(), 65534, 65534) }, 0);
    }
    let previous = destination.metadata().unwrap();
    backup(
        &fixture.backup,
        &[("etc/config/system", b"restored")],
        0o6660,
    );
    fixture.applied();
    let metadata = destination.metadata().unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o660);
    assert_eq!(
        (metadata.uid(), metadata.gid()),
        (previous.uid(), previous.gid())
    );
}

#[test]
fn migration_waits_for_actual_reboot_and_is_idempotent() {
    let mut fixture = Fixture::new();
    let id = fixture.applied();
    assert!(!fixture.runner.0.lock().unwrap().installed.contains("curl"));
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "awaiting_reboot");
    fixture.jobs.boot_id = Some("after".to_owned());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
    assert!(fixture.runner.0.lock().unwrap().installed.contains("curl"));
    let commands = fixture.runner.0.lock().unwrap().commands.len();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.runner.0.lock().unwrap().commands.len(), commands);
}

#[test]
fn package_retry_preserves_configuration_edited_after_restore() {
    let mut fixture = Fixture::new();
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".to_owned());
    fixture
        .runner
        .0
        .lock()
        .unwrap()
        .failed
        .insert("curl".to_owned());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "failed_packages");
    let destination = fixture.root.join("etc/config/system");
    fs::write(&destination, "edited after restoration").unwrap();
    fixture.runner.0.lock().unwrap().failed.clear();
    fixture.jobs.retry(&id).unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
    assert_eq!(
        fs::read_to_string(destination).unwrap(),
        "edited after restoration"
    );
    assert!(
        !fixture
            .runner
            .0
            .lock()
            .unwrap()
            .commands
            .iter()
            .flatten()
            .any(|argument| argument == "--force-broken-world")
    );
}

#[test]
fn selected_applications_translations_and_themes_are_removed_together() {
    let mut fixture = Fixture::new();
    fixture.options.remove_packages = [
        "luci-theme-argon",
        "luci-app-argon-config",
        "luci-app-nlbwmon",
        "luci-i18n-argon-config-zh-cn",
        "luci-i18n-nlbwmon-zh-cn",
    ]
    .map(str::to_owned)
    .to_vec();
    {
        let mut state = fixture.runner.0.lock().unwrap();
        state
            .installed
            .extend(fixture.options.remove_packages.clone());
        state.installed.insert("luci-base".to_owned());
        state.dependents = BTreeMap::from([
            (
                "luci-theme-argon".to_owned(),
                vec!["luci-app-argon-config".to_owned()],
            ),
            (
                "luci-app-argon-config".to_owned(),
                vec!["luci-i18n-argon-config-zh-cn".to_owned()],
            ),
            (
                "luci-app-nlbwmon".to_owned(),
                vec!["luci-i18n-nlbwmon-zh-cn".to_owned()],
            ),
        ]);
    }
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".to_owned());
    fixture.jobs.worker(true).unwrap();
    let task = fixture.jobs.load(&id).unwrap();
    assert_eq!(task["status"], "complete");
    let state = fixture.runner.0.lock().unwrap();
    for package in &fixture.options.remove_packages {
        assert!(!state.installed.contains(package));
        assert_eq!(task["packages"][package]["status"], "removed");
    }
    assert!(state.installed.contains("luci-base"));
    assert_eq!(
        state
            .commands
            .iter()
            .filter(|arguments| arguments.iter().any(|argument| argument == "del"))
            .count(),
        1
    );
}

#[test]
fn removal_retains_unselected_dependents_and_reports_each_package_result() {
    let mut fixture = Fixture::new();
    fixture.options.remove_packages = [
        "luci-app-ddns",
        "luci-app-nlbwmon",
        "luci-i18n-nlbwmon-zh-cn",
        "luci-app-not-installed",
    ]
    .map(str::to_owned)
    .to_vec();
    {
        let mut state = fixture.runner.0.lock().unwrap();
        state
            .installed
            .extend(fixture.options.remove_packages[..3].iter().cloned());
        state.installed.insert("luci-app-other-plugin".to_owned());
        state.dependents = BTreeMap::from([
            (
                "luci-app-ddns".to_owned(),
                vec!["luci-app-other-plugin".to_owned()],
            ),
            (
                "luci-app-nlbwmon".to_owned(),
                vec!["luci-i18n-nlbwmon-zh-cn".to_owned()],
            ),
        ]);
        state.remove_exit_code = 1;
    }
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".to_owned());
    fixture.jobs.worker(true).unwrap();
    let task = fixture.jobs.load(&id).unwrap();
    assert_eq!(task["status"], "failed_packages");
    assert_eq!(task["error"], "luci-app-ddns");
    assert_eq!(task["packages"]["luci-app-ddns"]["status"], "failed");
    assert!(
        task["packages"]["luci-app-ddns"]["message"]
            .as_str()
            .unwrap()
            .contains("dependencies")
    );
    for package in &fixture.options.remove_packages[1..] {
        assert_eq!(task["packages"][package]["status"], "removed");
    }
    let state = fixture.runner.0.lock().unwrap();
    assert!(state.installed.contains("luci-app-other-plugin"));
    assert!(state.installed.contains("luci-app-ddns"));
    let removal = state
        .commands
        .iter()
        .find(|arguments| arguments.iter().any(|argument| argument == "del"))
        .unwrap();
    assert_eq!(&removal[4..], &fixture.options.remove_packages[..3]);
    assert!(
        !removal
            .iter()
            .any(|argument| argument.starts_with("--force") || argument == "--rdepends")
    );
}

#[test]
fn later_dependency_installation_clears_an_earlier_download_failure() {
    let mut fixture = Fixture::new();
    fixture.options.myfeed_packages = ["luci-app-homebox", "luci-i18n-homebox-zh-cn"]
        .map(str::to_owned)
        .to_vec();
    write(
        &fixture.root.join("etc/apk/keys/myfeed.pem"),
        "fixture public key",
    );
    {
        let mut state = fixture.runner.0.lock().unwrap();
        state.failed.insert("luci-app-homebox".to_owned());
        state.dependencies.insert(
            "luci-i18n-homebox-zh-cn".to_owned(),
            vec!["luci-app-homebox".to_owned()],
        );
    }
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".to_owned());
    fixture.jobs.worker(true).unwrap();
    let task = fixture.jobs.load(&id).unwrap();
    assert_eq!(task["status"], "complete");
    assert_eq!(task["error"], "");
    assert_eq!(task["packages"]["luci-app-homebox"]["status"], "installed");
    assert!(
        fixture
            .runner
            .0
            .lock()
            .unwrap()
            .commands
            .iter()
            .any(|arguments| arguments
                .last()
                .is_some_and(|argument| argument == "luci-app-homebox@myfeed"))
    );
}

#[test]
fn failed_tagged_install_does_not_accept_an_existing_package_as_success() {
    let mut fixture = Fixture::new();
    fixture.options.myfeed_packages = vec!["luci-app-homebox".to_owned()];
    write(
        &fixture.root.join("etc/apk/keys/myfeed.pem"),
        "fixture public key",
    );
    {
        let mut state = fixture.runner.0.lock().unwrap();
        state.installed.insert("luci-app-homebox".to_owned());
        state.failed.insert("luci-app-homebox".to_owned());
    }
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".to_owned());
    fixture.jobs.worker(true).unwrap();
    let task = fixture.jobs.load(&id).unwrap();
    assert_eq!(task["status"], "failed_packages");
    assert_eq!(task["error"], "luci-app-homebox");
    assert_eq!(task["packages"]["luci-app-homebox"]["status"], "failed");
    assert!(
        fixture
            .runner
            .0
            .lock()
            .unwrap()
            .installed
            .contains("luci-app-homebox")
    );
}

#[test]
fn recovery_upgrades_existing_selected_packages_and_their_dependencies() {
    let mut fixture = Fixture::new();
    fixture.options.myfeed_packages = ["nikki", "luci-app-nikki"].map(str::to_owned).to_vec();
    write(
        &fixture.root.join("etc/apk/keys/myfeed.pem"),
        "fixture public key",
    );
    {
        let mut state = fixture.runner.0.lock().unwrap();
        for (name, old, latest) in [
            ("curl", "old", "new"),
            ("nikki", "2026.04.08-r1", "2026.04.08-r8"),
            ("luci-app-nikki", "1.26.1-r1", "1.26.2-r4"),
            ("mihomo-meta", "1.19.31", "1.19.32"),
            ("unselected-plugin", "old", "new"),
        ] {
            state.installed.insert(name.into());
            state.versions.insert(name.into(), old.into());
            state.available_versions.insert(name.into(), latest.into());
        }
        state
            .dependencies
            .insert("nikki".into(), vec!["mihomo-meta".into()]);
    }
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
    let state = fixture.runner.0.lock().unwrap();
    for name in ["curl", "nikki", "luci-app-nikki", "mihomo-meta"] {
        assert_eq!(state.versions[name], state.available_versions[name]);
    }
    assert_eq!(state.versions["unselected-plugin"], "old");
}

#[test]
fn package_retry_does_not_repeat_completed_upgrades() {
    let mut fixture = Fixture::new();
    fixture.options.myfeed_packages = ["nikki", "luci-app-nikki"].map(str::to_owned).to_vec();
    write(
        &fixture.root.join("etc/apk/keys/myfeed.pem"),
        "fixture public key",
    );
    {
        let mut state = fixture.runner.0.lock().unwrap();
        state.installed.insert("nikki".into());
        state.failed.insert("luci-app-nikki".into());
    }
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "failed_packages");
    fixture.runner.0.lock().unwrap().failed.clear();
    fixture.jobs.retry(&id).unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
    let state = fixture.runner.0.lock().unwrap();
    let attempts = |package: &str| {
        state
            .commands
            .iter()
            .filter(|args| {
                args.iter().any(|argument| argument == "add")
                    && args.last().is_some_and(|argument| argument == package)
            })
            .count()
    };
    assert_eq!(attempts("nikki@myfeed"), 1);
    assert_eq!(attempts("luci-app-nikki@myfeed"), 2);
}

fn seed_luci_core(fixture: &Fixture) {
    let database = crate::luci::CORE
        .iter()
        .map(|name| format!("P:{name}\nV:old\n\n"))
        .collect::<String>();
    write(&fixture.root.join("rom/lib/apk/db/installed"), database);
    let mut state = fixture.runner.0.lock().unwrap();
    for name in crate::luci::CORE {
        state.installed.insert(name.into());
        state.versions.insert(name.into(), "old".into());
        state.available_versions.insert(name.into(), "new".into());
    }
}

#[test]
fn application_dependencies_cannot_leave_old_luci_core_views_installed() {
    let mut fixture = Fixture::new();
    seed_luci_core(&fixture);
    fixture.options.install_packages = vec!["curl".into()];
    fixture
        .runner
        .0
        .lock()
        .unwrap()
        .dependencies
        .insert("curl".into(), vec!["luci-base".into()]);
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    let task = fixture.jobs.load(&id).unwrap();
    assert_eq!(task["status"], "complete");
    let state = fixture.runner.0.lock().unwrap();
    for name in crate::luci::CORE {
        assert_eq!(state.versions[name], "new");
        assert_eq!(task["packages"][name]["status"], "installed");
    }
    let installs: Vec<_> = state
        .commands
        .iter()
        .filter(|args| args.iter().any(|arg| arg == "add"))
        .collect();
    let group = installs.last().unwrap();
    assert!(
        crate::luci::CORE
            .iter()
            .all(|name| group.iter().any(|arg| arg == name))
    );
    assert!(group.iter().any(|arg| arg == "--latest"));
}

#[test]
fn incoherent_luci_versions_do_not_report_success_and_can_retry() {
    let mut fixture = Fixture::new();
    seed_luci_core(&fixture);
    fixture
        .runner
        .0
        .lock()
        .unwrap()
        .available_versions
        .insert("luci-mod-status".into(), "different-source-version".into());
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".into());
    fixture.jobs.worker(true).unwrap();
    let task = fixture.jobs.load(&id).unwrap();
    assert_eq!(task["status"], "failed_packages");
    assert!(
        task["error"]
            .as_str()
            .unwrap()
            .contains("LuCI core versions do not match")
    );
    assert_eq!(task["packages"]["luci-mod-status"]["status"], "failed");
    fixture
        .runner
        .0
        .lock()
        .unwrap()
        .available_versions
        .insert("luci-mod-status".into(), "new".into());
    fixture.jobs.retry(&id).unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "complete");
}

#[test]
fn local_cache_without_existing_luci_views_requires_refresh() {
    let mut fixture = Fixture::new();
    seed_local_feed(&mut fixture);
    seed_luci_core(&fixture);
    assert_eq!(
        local_feed::status(&fixture.jobs, &fixture.options).unwrap()["status"],
        "stale"
    );
    assert!(
        local_feed::select(
            &fixture.jobs,
            &fixture.options,
            &fixture.options.myfeed_repo
        )
        .is_err()
    );
}

#[test]
fn local_feed_download_includes_core_views_from_firmware() {
    let mut fixture = Fixture::new();
    seed_local_feed(&mut fixture);
    seed_luci_core(&fixture);
    write(
        &fixture
            .root
            .join("etc/overlay-restore-bootstrap/myfeed.pem"),
        "fixture trusted key",
    );
    local_feed::queue(&fixture.jobs, &fixture.options).unwrap();
    local_feed::work(&fixture.jobs).unwrap();
    let state = fixture.runner.0.lock().unwrap();
    let download = state
        .commands
        .iter()
        .find(|args| args.iter().any(|arg| arg == "download"))
        .unwrap();
    assert!(
        crate::luci::CORE
            .iter()
            .all(|name| download.iter().any(|arg| arg == name))
    );
}

#[test]
fn luci_group_keeps_only_present_or_requested_views() {
    let fixture = Fixture::new();
    write(
        &fixture.root.join("lib/apk/db/installed"),
        "P:luci-mod-status\nV:old\n\nP:luci-app-nikki\nV:old\n",
    );
    write(
        &fixture
            .root
            .join("rom/lib/apk/packages/luci-mod-network.list"),
        "firmware list",
    );
    let mut options = fixture.options.clone();
    options.install_packages.push("luci-mod-system".into());
    assert_eq!(
        crate::luci::packages(&fixture.root, &options).unwrap(),
        [
            "luci-base",
            "luci-mod-status",
            "luci-mod-network",
            "luci-mod-system"
        ]
    );
}

#[test]
fn missing_luci_core_packages_are_rejected_during_verification() {
    assert!(
        crate::luci::verify_versions(
            "luci-base-new noarch [installed]\n",
            &["luci-base".into(), "luci-mod-status".into()]
        )
        .is_err()
    );
}

#[test]
fn later_dependencies_do_not_hide_a_package_that_was_reinstalled_after_removal() {
    let mut fixture = Fixture::new();
    fixture.options.remove_packages = vec!["luci-app-ddns".to_owned()];
    {
        let mut state = fixture.runner.0.lock().unwrap();
        state.installed.insert("luci-app-ddns".to_owned());
        state
            .dependencies
            .insert("curl".to_owned(), vec!["luci-app-ddns".to_owned()]);
    }
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".to_owned());
    fixture.jobs.worker(true).unwrap();
    let task = fixture.jobs.load(&id).unwrap();
    assert_eq!(task["status"], "failed_packages");
    assert_eq!(task["error"], "luci-app-ddns");
    assert_eq!(task["packages"]["luci-app-ddns"]["status"], "failed");
    assert!(
        fixture
            .runner
            .0
            .lock()
            .unwrap()
            .installed
            .contains("luci-app-ddns")
    );
}

#[test]
fn a_failed_package_probe_does_not_report_successful_removal() {
    let mut fixture = Fixture::new();
    fixture.options.remove_packages = vec!["luci-app-ddns".to_owned()];
    let id = fixture.applied();
    fixture.jobs.boot_id = Some("after".to_owned());
    {
        let mut state = fixture.runner.0.lock().unwrap();
        state.installed.insert("luci-app-ddns".to_owned());
        state.failed_probes.insert("luci-app-ddns".to_owned());
    }
    fixture.jobs.worker(true).unwrap();
    let task = fixture.jobs.load(&id).unwrap();
    assert_eq!(task["status"], "failed_packages");
    assert!(
        task["error"]
            .as_str()
            .unwrap()
            .contains("Unable to check installed package luci-app-ddns")
    );
    assert_ne!(task["packages"]["luci-app-ddns"]["status"], "removed");
    assert!(
        fixture
            .runner
            .0
            .lock()
            .unwrap()
            .installed
            .contains("luci-app-ddns")
    );
}

#[test]
fn automatic_package_retry_is_limited_to_three_separate_boots() {
    let mut fixture = Fixture::new();
    let id = fixture.applied();
    fixture
        .runner
        .0
        .lock()
        .unwrap()
        .failed
        .insert("curl".to_owned());
    for boot in ["after-1", "after-2", "after-3"] {
        fixture.jobs.boot_id = Some(boot.to_owned());
        fixture.jobs.worker(true).unwrap();
        let attempts = fixture.jobs.load(&id).unwrap()["boot_attempts"].clone();
        fixture.jobs.worker(true).unwrap();
        assert_eq!(fixture.jobs.load(&id).unwrap()["boot_attempts"], attempts);
    }
    fixture.jobs.boot_id = Some("after-4".to_owned());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["boot_attempts"], 3);
}

#[test]
fn symlink_parent_destination_is_rejected() {
    let fixture = Fixture::new();
    symlink(fixture.root.join("tmp"), fixture.root.join("etc/config")).unwrap();
    let state = fixture
        .jobs
        .prepare(&fixture.backup, &fixture.options)
        .unwrap();
    let id = state["id"].as_str().unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(
        fixture.jobs.load(id).unwrap()["status"],
        "failed_validation"
    );
}

#[test]
fn clean_apply_rejects_no_reboot_without_mutating_the_inspected_task() {
    let fixture = Fixture::new();
    let id = fixture.ready();
    let task = fixture.jobs.path(&id).unwrap();
    let mut options = fixture.options.clone();
    options.clean_overlay = true;
    options.keep_current_extroot = true;
    options.reboot = true;
    save_json(&task.join("options.json"), &options).unwrap();
    assert!(fixture.jobs.apply(&id, &id, Some(false)).is_err());
    let unchanged: Options = read_json(&task.join("options.json")).unwrap();
    assert!(unchanged.reboot);
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "ready");
    assert!(!task.join("payload").exists());
    assert!(fixture.jobs.temporary.join(&id).join("payload").is_dir());
}

#[test]
fn insufficient_storage_reports_the_checked_location_and_capacity() {
    let temporary = tempdir().unwrap();
    crate::util::require_space(temporary.path(), 0, "recovery").unwrap();
    let error = crate::util::require_space(temporary.path(), u64::MAX, "recovery")
        .unwrap_err()
        .to_string();
    assert!(error.contains(&temporary.path().display().to_string()));
    assert!(error.contains("MiB available"));
    assert!(error.contains("MiB required"));
}

#[test]
fn reconnect_addresses_support_uci_ipaddr_lists_and_reject_hostnames() {
    for setting in [
        "option ipaddr '192.168.1.1'",
        "option ipaddr '192.168.1.1/24'",
        "list ipaddr '192.168.1.1/24'\nlist ipaddr '192.168.2.1/24'",
    ] {
        let network = format!("config interface 'lan'\n{setting}\n");
        assert_eq!(
            crate::archive::lan_address(&network).unwrap(),
            "192.168.1.1"
        );
    }
    assert_eq!(
        crate::archive::lan_address("config interface 'lan'\noption ipaddr 'example.com'\n")
            .unwrap(),
        ""
    );
}

#[test]
fn tampered_staging_and_persistent_payloads_are_rejected() {
    let fixture = Fixture::new();
    let id = fixture.ready();
    let source = fixture
        .jobs
        .temporary
        .join(&id)
        .join("payload/etc/config/system");
    fs::write(source, "tampered").unwrap();
    assert!(fixture.jobs.apply(&id, &id, None).is_err());
    let fixture = Fixture::new();
    let id = fixture.ready();
    fixture.jobs.apply(&id, &id, None).unwrap();
    fs::write(
        fixture
            .jobs
            .path(&id)
            .unwrap()
            .join("payload/etc/config/system"),
        "tampered",
    )
    .unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "failed_apply");
    assert!(!fixture.root.join("etc/config/system").exists());
}

#[test]
fn active_tasks_and_worker_lock_exclude_concurrent_operations() {
    let fixture = Fixture::new();
    fixture
        .jobs
        .prepare(&fixture.backup, &fixture.options)
        .unwrap();
    assert!(
        fixture
            .jobs
            .prepare(&fixture.backup, &fixture.options)
            .is_err()
    );
    let _lock = fixture.jobs.lock("worker", false).unwrap();
    assert!(fixture.jobs.worker(true).is_err());
}

#[test]
fn repository_journal_restores_original_content_and_existing_world_tags() {
    let fixture = Fixture::new();
    let id = fixture.ready();
    let repo = fixture.root.join("etc/apk/repositories.d/00-myfeed.list");
    let world = fixture.root.join("etc/apk/world");
    write(&repo, "@myfeed https://example.org/packages.adb\n");
    write(
        &world,
        "curl@myfeed\nexisting@myfeed\nconstrained@myfeed>=1\n",
    );
    save_json(&fixture.jobs.path(&id).unwrap().join("repository.json"), &json!({"existed": true, "original": "# comment\nhttps://example.org/packages.adb\n", "url": "https://example.org/packages.adb", "packages": ["curl", "existing", "constrained"], "tagged_before": ["existing"]})).unwrap();
    packages::restore_repository(&fixture.jobs, &id).unwrap();
    assert_eq!(
        fs::read_to_string(repo).unwrap(),
        "# comment\nhttps://example.org/packages.adb\n"
    );
    assert_eq!(
        fs::read_to_string(world).unwrap(),
        "curl\nexisting@myfeed\nconstrained>=1\n"
    );
}

#[test]
fn clean_recovery_repairs_missing_feed_tags_without_changing_world_constraints() {
    let fixture = Fixture::new();
    let id = fixture.ready();
    let mut task = fixture.jobs.load(&id).unwrap();
    task["clean_overlay"] = json!({"phase": "switched"});
    fixture.jobs.save(&mut task).unwrap();
    write(&fixture.root.join("etc/overlay-restore/clean-id"), &id);
    let repo = fixture.root.join("etc/apk/repositories.d/00-myfeed.list");
    let world = fixture.root.join("etc/apk/world");
    let original = "# keep comment\nhttps://example.org/packages.adb\n";
    let constraints = "overlay-restore@myfeed>=0.2.0-r8\nnikki@myfeed\ncurl\n";
    write(&repo, original);
    write(&world, constraints);
    let plan: crate::archive::Plan =
        read_json(&fixture.jobs.path(&id).unwrap().join("plan.json")).unwrap();
    packages::restore_repository(&fixture.jobs, &id).unwrap();
    let repaired = fs::read_to_string(&repo).unwrap();
    assert_eq!(
        repaired,
        format!("{original}@myfeed {}\n", plan.myfeed_repo)
    );
    assert_eq!(fs::read_to_string(&world).unwrap(), constraints);
    packages::restore_repository(&fixture.jobs, &id).unwrap();
    assert_eq!(fs::read_to_string(repo).unwrap(), repaired);
}

#[test]
fn clean_transaction_keeps_the_tag_definition_for_preexisting_bootstrap_pins() {
    let fixture = Fixture::new();
    let id = fixture.ready();
    let mut task = fixture.jobs.load(&id).unwrap();
    task["clean_overlay"] = json!({"phase": "switched"});
    fixture.jobs.save(&mut task).unwrap();
    write(&fixture.root.join("etc/overlay-restore/clean-id"), &id);
    let repo = fixture.root.join("etc/apk/repositories.d/00-myfeed.list");
    let world = fixture.root.join("etc/apk/world");
    write(&repo, "@myfeed https://example.org/packages.adb\n");
    write(&world, "overlay-restore@myfeed>=0.2.0-r8\ncurl@myfeed\n");
    save_json(&fixture.jobs.path(&id).unwrap().join("repository.json"), &json!({"existed": true, "original": "https://example.org/packages.adb\n", "url": "https://example.org/packages.adb", "packages": ["curl"], "tagged_before": ["overlay-restore"]})).unwrap();
    packages::restore_repository(&fixture.jobs, &id).unwrap();
    let plan: crate::archive::Plan =
        read_json(&fixture.jobs.path(&id).unwrap().join("plan.json")).unwrap();
    assert_eq!(
        fs::read_to_string(repo).unwrap(),
        format!(
            "https://example.org/packages.adb\n@myfeed {}\n",
            plan.myfeed_repo
        )
    );
    assert_eq!(
        fs::read_to_string(world).unwrap(),
        "overlay-restore@myfeed>=0.2.0-r8\ncurl\n"
    );
}

#[test]
fn ordinary_or_inactive_recovery_does_not_add_a_feed_tag() {
    let fixture = Fixture::new();
    let id = fixture.ready();
    let repo = fixture.root.join("etc/apk/repositories.d/00-myfeed.list");
    let contents = "https://example.org/packages.adb\n";
    write(&repo, contents);
    write(&fixture.root.join("etc/apk/world"), "existing@myfeed\n");
    packages::restore_repository(&fixture.jobs, &id).unwrap();
    assert_eq!(fs::read_to_string(&repo).unwrap(), contents);
    let mut task = fixture.jobs.load(&id).unwrap();
    task["clean_overlay"] = json!({"phase": "switched"});
    fixture.jobs.save(&mut task).unwrap();
    write(
        &fixture.root.join("etc/overlay-restore/clean-id"),
        "another-task",
    );
    packages::restore_repository(&fixture.jobs, &id).unwrap();
    assert_eq!(fs::read_to_string(repo).unwrap(), contents);
}

#[test]
fn failed_write_rolls_back_completed_files_and_removes_new_files() {
    let mut fixture = Fixture::new();
    backup(
        &fixture.backup,
        &[
            ("etc/config/a", b"new-a"),
            ("etc/config/b", b"new-b"),
            ("etc/config/c", b"new-c"),
        ],
        0o644,
    );
    write(&fixture.root.join("etc/config/a"), "old-a");
    write(&fixture.root.join("etc/config/c"), "old-c");
    let id = fixture.ready();
    fixture.jobs.apply(&id, &id, None).unwrap();
    fixture.jobs.fail_write = Some("etc/config/c".to_owned());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "failed_apply");
    assert_eq!(
        fs::read_to_string(fixture.root.join("etc/config/a")).unwrap(),
        "old-a"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("etc/config/c")).unwrap(),
        "old-c"
    );
    assert!(!fixture.root.join("etc/config/b").exists());
}

#[test]
fn failure_saving_original_does_not_overwrite_live_file() {
    let mut fixture = Fixture::new();
    let destination = fixture.root.join("etc/config/system");
    write(&destination, "complete original data");
    let id = fixture.ready();
    fixture.jobs.apply(&id, &id, None).unwrap();
    fixture.jobs.fail_original = true;
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "failed_apply");
    assert_eq!(
        fs::read_to_string(destination).unwrap(),
        "complete original data"
    );
}

#[test]
fn missing_network_package_failure_preserves_configuration_and_can_retry() {
    let mut fixture = Fixture::new();
    fixture.options.myfeed_packages = vec!["smartdns".to_owned()];
    write(
        &fixture.root.join("etc/apk/keys/myfeed.pem"),
        "fixture public key",
    );
    let destination = fixture.root.join("etc/config/system");
    write(&destination, "original");
    let id = fixture.ready();
    fixture
        .runner
        .0
        .lock()
        .unwrap()
        .failed
        .insert("smartdns".to_owned());
    fixture.jobs.apply(&id, &id, None).unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "failed_prepare");
    assert_eq!(fs::read_to_string(&destination).unwrap(), "original");
    assert!(
        !fixture
            .jobs
            .path(&id)
            .unwrap()
            .join("repository.json")
            .exists()
    );
    fixture.runner.0.lock().unwrap().failed.clear();
    fixture.jobs.retry(&id).unwrap();
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(&id).unwrap()["status"], "awaiting_reboot");
}

#[test]
fn rpc_rejects_invalid_backup_paths_unknown_methods_and_invalid_task_ids() {
    let fixture = Fixture::new();
    for path in [
        json!("/etc/shadow"),
        json!("backup.tar.gz"),
        json!("/../backup.tar.gz"),
        json!("/tmp/../../backup.tar.gz"),
        json!("/missing.tar.gz"),
        json!("/backup\0.tar.gz"),
        json!(null),
        json!(12),
    ] {
        assert!(rpc::call(&fixture.jobs, "prepare", &json!({"path": path})).is_err());
    }
    assert!(rpc::call(&fixture.jobs, "prepare", &json!({"command": "id"})).is_err());
    assert!(rpc::call(&fixture.jobs, "status", &json!({"id": "../outside"})).is_err());
    assert!(rpc::call(&fixture.jobs, "exec", &json!({})).is_err());
    assert!(rpc::call(&fixture.jobs, "status", &json!({"id": 12})).is_err());
}

#[test]
fn rpc_router_backup_is_retained_and_uses_an_independent_snapshot() {
    let fixture = Fixture::new();
    let source = fixture.root.join("mnt/备份/overlay backup.tgz");
    write(&source, fs::read(&fixture.backup).unwrap());
    let upload = fixture.root.join("tmp/overlay-restore-upload.tar.gz");
    write(&upload, b"unrelated upload");
    let result = rpc::call(
        &fixture.jobs,
        "prepare",
        &json!({"path": "/mnt/备份/overlay backup.tgz"}),
    )
    .unwrap();
    let id = result["id"].as_str().unwrap();
    assert_eq!(
        fs::read(&source).unwrap(),
        fs::read(&fixture.backup).unwrap()
    );
    assert_eq!(fs::read(&upload).unwrap(), b"unrelated upload");
    write(&source, b"changed after inspection");
    fixture.jobs.worker(true).unwrap();
    assert_eq!(fixture.jobs.load(id).unwrap()["status"], "ready");
}

#[test]
fn rpc_upload_still_consumes_only_the_fixed_upload_path() {
    let fixture = Fixture::new();
    let upload = fixture.root.join("tmp/overlay-restore-upload.tar.gz");
    write(&upload, fs::read(&fixture.backup).unwrap());
    let result = rpc::call(&fixture.jobs, "prepare", &json!({})).unwrap();
    assert!(!upload.exists());
    assert!(fixture.backup.exists());
    fixture.jobs.worker(true).unwrap();
    assert_eq!(
        fixture.jobs.load(result["id"].as_str().unwrap()).unwrap()["status"],
        "ready"
    );
}

#[test]
fn rpc_router_backup_rejects_links_special_files_and_oversized_sources() {
    let fixture = Fixture::new();
    let link = fixture.root.join("link.tar.gz");
    symlink(&fixture.backup, &link).unwrap();
    assert!(rpc::call(&fixture.jobs, "prepare", &json!({"path": "/link.tar.gz"})).is_err());
    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
    fs::create_dir(fixture.root.join("directory.tar.gz")).unwrap();
    assert!(
        rpc::call(
            &fixture.jobs,
            "prepare",
            &json!({"path": "/directory.tar.gz"})
        )
        .is_err()
    );
    let outside = tempdir().unwrap();
    symlink(outside.path(), fixture.root.join("escape")).unwrap();
    assert!(
        rpc::call(
            &fixture.jobs,
            "prepare",
            &json!({"path": "/escape/backup.tar.gz"})
        )
        .is_err()
    );
    let large = fixture.root.join("large.tar.gz");
    File::create(&large)
        .unwrap()
        .set_len(257 * 1024 * 1024)
        .unwrap();
    assert!(rpc::call(&fixture.jobs, "prepare", &json!({"path": "/large.tar.gz"})).is_err());
    assert!(large.exists());
    assert!(fixture.jobs.states().unwrap().is_empty());
}

#[test]
fn saved_uci_empty_lists_do_not_reinstate_defaults() {
    let options = from_uci("overlay_restore.main=restore\n", None).unwrap();
    assert!(options.install_packages.is_empty());
    assert!(options.myfeed_packages.is_empty());
    assert!(options.optional_packages.is_empty());
    assert!(options.remove_packages.is_empty());
}

#[test]
fn uci_lists_and_legacy_environment_are_compatible() {
    let environment = BTreeMap::from([
        ("RESTORE_KEEP_EXTROOT".to_owned(), "1".to_owned()),
        ("RESTORE_MYFEED_INSTALL_PACKAGES".to_owned(), "".to_owned()),
    ]);
    let options = from_uci(
        "overlay_restore.main.install_packages='curl' 'tcpdump'\n",
        Some(&environment),
    )
    .unwrap();
    assert_eq!(options.install_packages, ["curl", "tcpdump"]);
    assert!(options.myfeed_packages.is_empty());
    assert!(!options.keep_current_extroot);
}

#[test]
fn invalid_packages_core_removal_paths_ips_urls_and_quoting_are_rejected() {
    for update in [
        json!({"remove_packages": ["python3-light"]}),
        json!({"remove_packages": ["libubus"]}),
        json!({"remove_packages": ["luci-theme-bootstrap"]}),
        json!({"install_packages": ["curl;reboot"]}),
        json!({"iptv_refresh_host": "not-an-ip"}),
        json!({"iptv_repo_root": "/mnt/../../etc"}),
        json!({"myfeed_repo": "http://example.org/packages.adb"}),
        json!({"myfeed_key_url": "https://user:password@example.org/key"}),
        json!({"iptv_refresh_allow_ips": ["10.1.1.1/99"]}),
    ] {
        let mut options = defaults();
        for (key, value) in update.as_object().unwrap() {
            options[key] = value.clone();
        }
        assert!(validate(&options).is_err(), "{update}");
    }
    assert!(from_uci("overlay_restore.main.iptv_repo_root='unterminated", None).is_err());
}

#[test]
fn historical_task_records_are_read_and_large_public_plans_are_bounded() {
    let fixture = Fixture::new();
    let files: Vec<String> = (0..201)
        .map(|index| format!("etc/config/setting-{index:03}"))
        .collect();
    let members: Vec<_> = files
        .iter()
        .map(|path| (path.as_str(), &b"setting"[..]))
        .collect();
    backup(&fixture.backup, &members, 0o644);
    let id = fixture.ready();
    let mut state = fixture.jobs.load(&id).unwrap();
    state["historical_field"] = json!("preserved");
    fixture.jobs.save(&mut state).unwrap();
    let public = fixture.jobs.public(&id, true).unwrap();
    assert_eq!(public["historical_field"], "preserved");
    assert_eq!(public["plan"]["file_count"], 201);
    assert_eq!(public["plan"]["files"].as_array().unwrap().len(), 200);
    let persisted: archive::Plan =
        read_json(&fixture.jobs.path(&id).unwrap().join("plan.json")).unwrap();
    assert_eq!(persisted.files.len(), 201);
    let options: Options =
        read_json(&fixture.jobs.path(&id).unwrap().join("options.json")).unwrap();
    options.validate().unwrap();
}

#[test]
fn subprocess_timeout_also_reaps_descendants_that_hold_output_pipes() {
    let runner = util::SystemRunner;
    let started = std::time::Instant::now();
    let (code, output) = runner.execute(
        &[
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "printf started; sleep 30 &".to_owned(),
        ],
        Duration::from_secs(1),
    );
    assert_ne!(code, 0);
    assert!(output.contains("started"));
    assert!(output.contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn atomic_replacement_rejects_symlink_sources_and_targets() {
    let temporary = tempdir().unwrap();
    let real = temporary.path().join("real");
    let link = temporary.path().join("link");
    write(&real, "original");
    symlink(&real, &link).unwrap();
    assert!(atomic_write(&link, "changed", 0o600).is_err());
    assert!(util::atomic_copy(&temporary.path().join("copy"), &link, 0o600).is_err());
    assert_eq!(fs::read_to_string(real).unwrap(), "original");
}

#[test]
fn iptv_does_not_create_missing_storage_and_tokens_stay_private() {
    let fixture = Fixture::new();
    let id = fixture.ready();
    let mut options = fixture.options.clone();
    options.iptv_enable = true;
    write(&fixture.root.join("etc/init.d/iptv-refresh"), "service");
    let warnings = services::repair_services(&fixture.jobs, &id, &mut options).unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("storage is not ready"))
    );
    assert!(
        !fixture
            .root
            .join(options.iptv_repo_root.trim_start_matches('/'))
            .exists()
    );
    assert!(!fixture.root.join("etc/iptv-refresh/token").exists());
}

#[test]
fn service_configuration_uses_argument_arrays_and_quoted_files() {
    let fixture = Fixture::new();
    let id = fixture.ready();
    let mut options = fixture.options.clone();
    options.iptv_enable = true;
    options.iptv_public_url = "http://example.org/it's-safe".to_owned();
    options.validate().unwrap();
    write(&fixture.root.join("etc/init.d/iptv-refresh"), "service");
    fs::create_dir_all(
        fixture
            .root
            .join(options.iptv_repo_root.trim_start_matches('/')),
    )
    .unwrap();
    services::repair_services(&fixture.jobs, &id, &mut options).unwrap();
    let env = fs::read_to_string(
        fixture
            .root
            .join(options.iptv_repo_root.trim_start_matches('/'))
            .join("config/local/iptv_refresh.env"),
    )
    .unwrap();
    assert!(env.contains("'\\''"));
    assert_eq!(options.iptv_refresh_token.len(), 64);
    let token = fixture.root.join("etc/iptv-refresh/token");
    assert_eq!(
        token.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        !fixture
            .jobs
            .public(&id, true)
            .unwrap()
            .to_string()
            .contains(&options.iptv_refresh_token)
    );
}
