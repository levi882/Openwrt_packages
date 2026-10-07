//! A retained, signed APK cache. Only recovery commands use it; system feeds
//! continue to point at their original online repositories.
use crate::archive::Plan;
use crate::engine::Jobs;
use crate::settings::Options;
use crate::util::{
    atomic_copy, atomic_write, digest_file, mkdir, random_hex, read_json, save_json, tail,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

const STATE: &str = "etc/overlay-restore-bootstrap/local-feed.json";
const SCHEDULE: &str = "etc/overlay-restore-bootstrap/local-feed-sync.json";

#[derive(Deserialize, Serialize)]
struct Schedule {
    directory: String,
    armed_at: u64,
    last_success: u64,
    next_attempt: u64,
    last_error: String,
}

impl Schedule {
    fn due(&self, interval: u64) -> u64 {
        self.armed_at
            .max(self.last_success)
            .saturating_add(interval)
            .max(self.next_attempt)
    }
}

#[derive(Deserialize, Serialize)]
struct Manifest {
    format: u32,
    created: u64,
    arch: String,
    myfeed: String,
    packages: Vec<String>,
    files: BTreeMap<String, String>,
    warnings: Vec<String>,
}

pub struct Cache {
    pub repositories: PathBuf,
    pub cache: PathBuf,
}

pub fn valid_directory(value: &str) -> Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    if !(value.starts_with("/mnt/") || value.starts_with("/media/"))
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '$')
        || Path::new(value)
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
        || value.split('/').any(|part| {
            matches!(
                part,
                "." | ".." | "upper" | "work" | ".overlay-restore-clean"
            )
        })
    {
        bail!(
            "The local feed must be a directory on a mounted disk under /mnt or /media (without spaces)"
        );
    }
    Ok(())
}

fn directory(jobs: &Jobs, value: &str, create: bool) -> Result<PathBuf> {
    valid_directory(value)?;
    if value.is_empty() {
        bail!("Configure a local feed directory first");
    }
    let path = jobs.root.join(value.trim_start_matches('/'));
    let mut existing = jobs.root.clone();
    for part in Path::new(value).components().filter_map(|c| {
        if let Component::Normal(p) = c {
            Some(p)
        } else {
            None
        }
    }) {
        existing.push(part);
        match existing.symlink_metadata() {
            Ok(meta) if meta.is_dir() => (),
            Ok(_) => bail!("The local feed path contains a symlink or a non-directory"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    // Refuse an absent disk's leftover /mnt directory on the internal overlay.
    if jobs.root == Path::new("/") {
        let mounts = crate::clean::mounts(&fs::read_to_string("/proc/self/mountinfo")?)?;
        let mount = mounts
            .iter()
            .filter(|m| Path::new(value).starts_with(&m.point))
            .max_by_key(|m| m.point.len())
            .context("Local feed disk is not mounted")?;
        if !mount.device.starts_with("/dev/")
            || !matches!(
                mount.filesystem.as_str(),
                "ext4" | "f2fs" | "btrfs" | "xfs" | "vfat" | "exfat" | "ntfs3"
            )
        {
            bail!("The local feed directory is not on a mounted persistent disk");
        }
    }
    if create {
        mkdir(&path, 0o700)?;
    }
    if !path.is_dir() {
        bail!("Local feed directory is unavailable; mount its disk first");
    }
    Ok(path)
}

fn required(jobs: &Jobs, options: &Options) -> Result<Vec<String>> {
    let mut packages: Vec<String> = options
        .install_packages
        .iter()
        .cloned()
        .chain(
            options
                .myfeed_packages
                .iter()
                .map(|p| format!("{p}@myfeed")),
        )
        .chain([
            "overlay-restore@myfeed>=0.2.0-r16".into(),
            "luci-app-overlay-restore@myfeed>=0.2.0-r19".into(),
        ])
        .collect();
    for package in crate::luci::packages(&jobs.root, options)? {
        if !packages.contains(&package) {
            packages.push(package);
        }
    }
    Ok(packages)
}

fn repositories(jobs: &Jobs, myfeed: &str) -> Result<String> {
    let mut inputs = Vec::new();
    let main = jobs.root.join("etc/apk/repositories");
    if main.is_file() {
        inputs.push(fs::read_to_string(main)?);
    }
    let mut seen = HashSet::new();
    for relative in ["etc/apk/repositories.d", "lib/apk/repositories.d"] {
        let path = jobs.root.join(relative);
        if !path.is_dir() {
            continue;
        }
        let mut files = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        files.sort_by_key(|f| f.file_name());
        for file in files {
            if file.path().extension().is_some_and(|e| e == "list") && seen.insert(file.file_name())
            {
                inputs.push(fs::read_to_string(file.path())?);
            }
        }
    }
    let mut output = Vec::new();
    for line in inputs.iter().flat_map(|text| text.lines()) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<_> = line.split_whitespace().collect();
        let url = match parts.as_slice() {
            [url] => *url,
            [tag, url] if tag.starts_with('@') => *url,
            _ => bail!(
                "Local feed preparation currently requires explicit HTTPS packages.adb repository addresses"
            ),
        };
        crate::settings::valid_url(url, false)?;
        if !url.ends_with(".adb") {
            bail!("Local feed preparation requires packages.adb repository addresses");
        }
        if url != myfeed && !output.iter().any(|s| s == line) {
            output.push(line.to_owned());
        }
    }
    output.push(myfeed.to_owned());
    output.push(format!("@myfeed {myfeed}"));
    Ok(output.join("\n") + "\n")
}

pub fn queue(jobs: &Jobs, options: &Options) -> Result<Value> {
    queue_with_origin(jobs, options, false)
}

fn queue_with_origin(jobs: &Jobs, options: &Options, automatic: bool) -> Result<Value> {
    options.validate()?;
    let _jobs = jobs.lock("jobs", false)?;
    jobs.require_idle()?;
    let _lock = jobs.lock("local-feed", false)?;
    if read_json::<Value>(&jobs.root.join(STATE))
        .is_ok_and(|s| s["status"] == "queued" || s["status"] == "downloading")
    {
        bail!("Local feed preparation is already running");
    }
    directory(jobs, &options.local_feed_dir, true)?;
    let myfeed = jobs.feed_url(options)?;
    let repositories = repositories(jobs, &myfeed)?;
    let state = json!({"status": "queued", "directory": options.local_feed_dir, "options": options, "myfeed": myfeed, "repositories": repositories, "automatic": automatic, "error": "", "warnings": []});
    save_json(&jobs.root.join(STATE), &state)?;
    Ok(json!({"status": "queued", "directory": options.local_feed_dir}))
}

pub fn work(jobs: &Jobs) -> Result<()> {
    let path = jobs.root.join(STATE);
    let Ok(mut state) = read_json::<Value>(&path) else {
        return Ok(());
    };
    if state["status"] != "queued" && state["status"] != "downloading" {
        return Ok(());
    }
    // A recovery queued after a sync request has priority. Once downloading,
    // hold the jobs lock so no recovery can start midway through the refresh.
    let Ok(_jobs) = jobs.lock("jobs", false) else {
        return Ok(());
    };
    if jobs.require_idle().is_err() || state["automatic"] == true && has_pending_recovery(jobs)? {
        return Ok(());
    }
    let _lock = jobs.lock("local-feed", false)?;
    state["status"] = json!("downloading");
    save_json(&path, &state)?;
    let result = download(jobs, &state);
    match result {
        Ok((snapshot, warnings)) => {
            state["status"] = json!("ready");
            state["snapshot"] = json!(snapshot);
            state["warnings"] = json!(warnings);
            state["error"] = json!("");
        }
        Err(error) => {
            state["status"] = json!("failed");
            state["error"] = json!(error.to_string());
        }
    }
    if let Ok(mut schedule) = read_json::<Schedule>(&jobs.root.join(SCHEDULE))
        && state["directory"] == schedule.directory
    {
        if state["status"] == "ready" {
            schedule.last_success = crate::util::now();
            schedule.next_attempt = 0;
            schedule.last_error.clear();
        } else {
            schedule.next_attempt = crate::util::now().saturating_add(3600);
            schedule.last_error = state["error"].as_str().unwrap_or("").into();
        }
        save_json(&jobs.root.join(SCHEDULE), &schedule)?;
    }
    // The previous ready snapshot is retained on every failure.
    save_json(&path, &state)
}

fn download(jobs: &Jobs, state: &Value) -> Result<(String, Vec<String>)> {
    let options: Options = serde_json::from_value(state["options"].clone())?;
    options.validate()?;
    let base = directory(jobs, &options.local_feed_dir, false)?;
    let snapshot = random_hex(16)?;
    let snapshots = base.join("snapshots");
    if snapshots
        .symlink_metadata()
        .is_ok_and(|meta| !meta.is_dir())
    {
        bail!("The local snapshot directory cannot be a symbolic link");
    }
    mkdir(&snapshots, 0o700)?;
    let store = snapshots.join(&snapshot);
    fs::create_dir(&store)?;
    let root = store.join("solver");
    mkdir(&root.join("cache"), 0o700)?;
    atomic_write(&root.join("etc/apk/world"), b"", 0o600)?;
    atomic_write(&root.join("lib/apk/db/installed"), b"", 0o600)?;
    atomic_write(&root.join("etc/apk/arch"), b"x86_64\n", 0o644)?;
    for relative in [
        "rom/etc/apk/keys",
        "rom/lib/apk/keys",
        "etc/apk/keys",
        "lib/apk/keys",
    ] {
        let source = jobs.root.join(relative);
        if source.is_dir() {
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    atomic_copy(
                        &root.join("etc/apk/keys").join(entry.file_name()),
                        &entry.path(),
                        0o644,
                    )?;
                }
            }
        }
    }
    trusted_key(jobs, &root.join("etc/apk/keys/overlay-restore-myfeed.pem"))?;
    let repo = store.join("repositories.list");
    atomic_write(
        &repo,
        state["repositories"]
            .as_str()
            .context("Missing repositories")?,
        0o600,
    )?;
    let prefix = vec![
        "apk".to_owned(),
        "--root".into(),
        root.to_string_lossy().into_owned(),
        "--cache-dir".into(),
        root.join("cache").to_string_lossy().into_owned(),
        "--repositories-file".into(),
        repo.to_string_lossy().into_owned(),
        "--wait".into(),
        "30".into(),
        "--timeout".into(),
        "20".into(),
        "--no-logfile".into(),
    ];
    let run = |extra: &[String], timeout| -> Result<()> {
        let mut args = prefix.clone();
        args.extend_from_slice(extra);
        let (code, output) = jobs.run(
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            None,
            timeout,
        )?;
        if code != 0 {
            bail!(
                "Unable to prepare the signed local feed: {}",
                tail(&output, 6000)
            );
        }
        Ok(())
    };
    run(&["update".into()], 180)?;
    let mut packages = required(jobs, &options)?;
    let mut args = vec!["cache".into(), "download".into(), "--available".into()];
    args.extend(packages.clone());
    run(&args, 900)?;
    let mut warnings = Vec::new();
    for package in &options.optional_packages {
        let dependency = format!("{package}@myfeed");
        let mut args = vec!["cache".into(), "download".into(), "--available".into()];
        args.extend(packages.clone());
        args.push(dependency.clone());
        match run(&args, 180) {
            Ok(()) => packages.push(dependency),
            Err(_) => warnings.push(format!("Optional package was not cached: {package}")),
        }
    }
    fs::rename(root.join("cache"), store.join("cache"))?;
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(store.join("cache"))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            bail!("Unexpected local cache entry");
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Invalid cache filename"))?;
        files.insert(name, digest_file(&entry.path())?);
    }
    if !files.keys().any(|n| n.ends_with(".apk")) {
        bail!("APK did not download any packages");
    }
    files.insert("../repositories.list".into(), digest_file(&repo)?);
    let manifest = Manifest {
        format: 1,
        created: crate::util::now(),
        arch: "x86_64".into(),
        myfeed: state["myfeed"]
            .as_str()
            .context("Missing myfeed address")?
            .into(),
        packages,
        files,
        warnings: warnings.clone(),
    };
    save_json(&store.join("manifest.json"), &manifest)?;
    let checksums = manifest
        .files
        .iter()
        .map(|(name, hash)| {
            let relative = if name == "../repositories.list" {
                "repositories.list".to_owned()
            } else {
                format!("cache/{name}")
            };
            format!("{hash}  {relative}\n")
        })
        .collect::<String>();
    atomic_write(&store.join("checksums.sha256"), checksums, 0o600)?;
    // Downloading never installs software. Remove only this generated solver root.
    fs::remove_dir_all(&root)?;
    atomic_write(&base.join("current"), &snapshot, 0o600)?;
    Ok((snapshot, warnings))
}

pub fn trusted_key(jobs: &Jobs, target: &Path) -> Result<()> {
    for relative in [
        "etc/apk/keys/myfeed.pem",
        "etc/overlay-restore-bootstrap/myfeed.pem",
        "etc/apk/keys/overlay-restore-bootstrap.pem",
    ] {
        let source = jobs.root.join(relative);
        if source.symlink_metadata().is_ok_and(|meta| meta.is_file()) {
            return atomic_copy(target, &source, 0o644);
        }
    }
    bail!("The trusted myfeed signing key is missing; install the current recovery package first")
}

fn load(jobs: &Jobs, options: &Options, snapshot: &str, verify: bool) -> Result<(Cache, Manifest)> {
    if snapshot.len() != 32
        || !snapshot
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        bail!("Invalid local feed snapshot");
    }
    let base = directory(jobs, &options.local_feed_dir, false)?;
    let store = base.join("snapshots").join(snapshot);
    for path in [base.join("snapshots"), store.clone(), store.join("cache")] {
        if !path.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
            bail!("Local feed directories must exist and cannot be symbolic links");
        }
    }
    let manifest: Manifest = read_json(&store.join("manifest.json"))?;
    if manifest.format != 1 || manifest.arch != "x86_64" {
        bail!("Unsupported local feed snapshot");
    }
    if verify {
        for (name, hash) in &manifest.files {
            let file = if name == "../repositories.list" {
                store.join("repositories.list")
            } else {
                if Path::new(name).components().count() != 1 || name.starts_with('.') {
                    bail!("Invalid local cache filename");
                }
                store.join("cache").join(name)
            };
            if digest_file(&file)? != *hash {
                bail!("Local feed file changed or is incomplete: {name}");
            }
        }
    }
    Ok((
        Cache {
            repositories: store.join("repositories.list"),
            cache: store.join("cache"),
        },
        manifest,
    ))
}

pub fn select(jobs: &Jobs, options: &Options, myfeed: &str) -> Result<String> {
    if !options.uses_local_feed() {
        return Ok(String::new());
    }
    let base = directory(jobs, &options.local_feed_dir, false)?;
    let snapshot = fs::read_to_string(base.join("current"))
        .context("Prepare the local feed before inspecting a backup")?;
    let (_, manifest) = load(jobs, options, &snapshot, true)?;
    if manifest.myfeed != myfeed {
        bail!("The local feed was prepared for a different myfeed address");
    }
    if required(jobs, options)?
        .iter()
        .any(|p| !manifest.packages.contains(p))
    {
        bail!(
            "The local feed no longer meets the software or recovery tool requirements; prepare it again"
        );
    }
    Ok(snapshot)
}

pub fn task(jobs: &Jobs, id: &str, verify: bool) -> Result<Option<Cache>> {
    let options: Options = read_json(&jobs.path(id)?.join("options.json"))?;
    if !options.uses_local_feed() {
        return Ok(None);
    }
    let plan: Plan = read_json(&jobs.path(id)?.join("plan.json"))?;
    let (cache, manifest) = load(jobs, &options, &plan.local_feed_snapshot, verify)?;
    if manifest.myfeed != plan.myfeed_repo {
        bail!("The local feed does not match this recovery plan");
    }
    Ok(Some(cache))
}

pub fn run(jobs: &Jobs, id: &str, arguments: &[&str], timeout: u64) -> Result<(i32, String)> {
    let Some(cache) = task(jobs, id, false)? else {
        return jobs.run(arguments, Some(id), timeout);
    };
    let mut args = vec![
        "apk".to_owned(),
        "--no-network".into(),
        "--cache-dir".into(),
        cache.cache.to_string_lossy().into_owned(),
        "--repositories-file".into(),
        cache.repositories.to_string_lossy().into_owned(),
    ];
    args.extend(arguments.iter().skip(1).map(|s| (*s).to_owned()));
    jobs.run(
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        Some(id),
        timeout,
    )
}

pub fn status(jobs: &Jobs, options: &Options) -> Result<Value> {
    let sync = sync_status(jobs, options);
    if options.local_feed_dir.is_empty() {
        return Ok(json!({"status": "disabled", "sync": sync}));
    }
    let state = read_json::<Value>(&jobs.root.join(STATE)).unwrap_or(json!({}));
    let active = state["directory"] == options.local_feed_dir;
    let mut output = json!({"status": if active {state["status"].as_str().unwrap_or("missing")} else {"missing"}, "directory": options.local_feed_dir, "error": if active {state["error"].clone()} else {json!("")}, "warnings": if active {state["warnings"].clone()} else {json!([])}});
    let mut available = false;
    if let Ok(base) = directory(jobs, &options.local_feed_dir, false)
        && let Ok(snapshot) = fs::read_to_string(base.join("current"))
        && let Ok((_, manifest)) = load(jobs, options, &snapshot, false)
    {
        available = true;
        output["snapshot"] = json!(snapshot);
        output["created"] = json!(manifest.created);
        output["packages"] = json!(manifest.packages.len());
        output["cached_files"] = json!(manifest.files.len().saturating_sub(1));
        if output["status"] == "missing" {
            output["status"] = json!("ready");
        }
        if output["status"] == "ready"
            && (manifest.myfeed != jobs.feed_url(options)?
                || required(jobs, options)?
                    .iter()
                    .any(|p| !manifest.packages.contains(p)))
        {
            output["status"] = json!("stale");
        }
    }
    if !available && output["status"] == "ready" {
        output["status"] = json!("missing");
    }
    output["sync"] = sync;
    Ok(output)
}

/// Arm maintenance only after software recovery succeeded. The retained state
/// survives reboot, sysupgrade, and removal of old recovery task records.
pub fn arm_after_recovery(jobs: &Jobs, id: &str, options: &Options) -> Result<()> {
    if !options.uses_local_feed() {
        return Ok(());
    }
    let state = jobs.load(id)?;
    if !["complete", "complete_with_warnings"].contains(&state["status"].as_str().unwrap_or("")) {
        return Ok(());
    }
    let _lock = jobs.lock("local-feed", false)?;
    let schedule = Schedule {
        directory: options.local_feed_dir.clone(),
        armed_at: crate::util::now(),
        last_success: status(jobs, options)?["created"].as_u64().unwrap_or(0),
        next_attempt: 0,
        last_error: String::new(),
    };
    save_json(&jobs.root.join(SCHEDULE), &schedule)?;
    jobs.log(id, "Local software recovery succeeded; periodic signed-feed cache synchronization can now run when enabled.")
}

fn sync_status(jobs: &Jobs, options: &Options) -> Value {
    let schedule = read_json::<Schedule>(&jobs.root.join(SCHEDULE));
    let status = if options.local_feed_dir.is_empty() || options.feed_sync_seconds().is_none() {
        "disabled"
    } else if schedule
        .as_ref()
        .is_ok_and(|s| s.directory == options.local_feed_dir)
    {
        "enabled"
    } else {
        "waiting_recovery"
    };
    let mut value = json!({"status": status, "interval": options.local_feed_sync});
    if let Ok(schedule) = schedule
        && schedule.directory == options.local_feed_dir
    {
        value["last_success"] = json!(schedule.last_success);
        value["last_error"] = json!(schedule.last_error);
        if status == "enabled" {
            value["next_sync"] = json!(schedule.due(options.feed_sync_seconds().unwrap()));
        }
    }
    value
}

/// Called at most once a minute by the existing procd worker. It queues a
/// normal signed cache download, never an installation or package upgrade.
pub fn tick(jobs: &Jobs, now: u64) -> Result<()> {
    let path = jobs.root.join(SCHEDULE);
    let Ok(mut schedule) = read_json::<Schedule>(&path) else {
        return Ok(());
    };
    let options = jobs.read_settings(None)?;
    let Some(interval) = options.feed_sync_seconds() else {
        return Ok(());
    };
    if options.local_feed_dir != schedule.directory || now < schedule.due(interval) {
        return Ok(());
    }
    // Failed recoveries can resume later and must remain fully offline too.
    if has_pending_recovery(jobs)?
        || read_json::<Value>(&jobs.root.join(STATE))
            .is_ok_and(|s| s["status"] == "queued" || s["status"] == "downloading")
    {
        return Ok(());
    }
    if let Err(error) = queue_with_origin(jobs, &options, true) {
        // A missing disk or temporary feed failure does not hammer the router.
        schedule.next_attempt = now.saturating_add(3600);
        schedule.last_error = error.to_string();
        save_json(&path, &schedule)?;
    }
    Ok(())
}

fn has_pending_recovery(jobs: &Jobs) -> Result<bool> {
    Ok(jobs.states()?.iter().any(|s| {
        !matches!(
            s["status"].as_str(),
            Some(
                "ready"
                    | "complete"
                    | "complete_with_warnings"
                    | "failed_validation"
                    | "rolled_back"
            )
        )
    }))
}
