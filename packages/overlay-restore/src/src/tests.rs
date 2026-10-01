use super::*;
use crate::archive::{inspect_backup, merge_fstab, safe_name};
use crate::engine::Jobs;
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

#[derive(Default)]
struct MockState {
    installed: HashSet<String>,
    failed: HashSet<String>,
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
                return (
                    i32::from(!state.installed.contains(&package)),
                    String::new(),
                );
            }
            if arguments.iter().any(|argument| argument == "add") {
                if state.failed.contains(&package) {
                    return (1, "repository download failed".to_owned());
                }
                state.installed.insert(package);
            } else if arguments.iter().any(|argument| argument == "del") {
                state.installed.remove(&package);
            }
        }
        if arguments[0] == "uci" {
            if arguments.get(2).is_some_and(|argument| argument == "show") {
                return (0, "overlay_restore.main=restore\n".to_owned());
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
fn traversal_absolute_control_and_backslash_names_are_rejected() {
    for name in [
        "../../etc/shadow",
        "/etc/shadow",
        "root/../etc/shadow",
        "root\\escape",
        "root/line\n",
    ] {
        assert!(safe_name(name).is_err(), "{name:?}");
    }
    for name in [
        "../etc/config/system",
        "/etc/config/system",
        "root/../etc/config/system",
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
    link_backup(&fixture.backup, "../../outside");
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
