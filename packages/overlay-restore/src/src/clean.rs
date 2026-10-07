//! Prepare a fresh upper from the running firmware, then exchange trees only
//! after procd has stopped services and pivoted to RAM. The old tree is retained.
use crate::archive::Plan;
use crate::engine::Jobs;
use crate::extroot::{Activation, Target};
use crate::settings::Options;
use crate::util::{
    atomic_copy, atomic_write, digest_file, mkdir, now, random_hex, read_json, require_space,
    save_json, sync_parent,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const STORE: &str = ".overlay-restore-clean";
const MARKER: &str = "etc/overlay-restore/clean-id";
const RAM_MANIFEST: &str = "/tmp/overlay-restore-stage.json";
const RAM_COMMAND: &str = "/usr/sbin/overlay-restore clean-stage /tmp/overlay-restore-stage.json";
const DEVICE_MOUNT: &str = "/overlay-restore-device";
const ORIGIN_MOUNT: &str = "/overlay-restore-origin";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LoopDevice {
    pub backing: String,
    pub identity: String,
    pub offset: u64,
    pub size_limit: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Overlay {
    pub device: String,
    pub identity: String,
    pub filesystem: String,
    pub mountpoint: String,
    pub firmware: String,
    pub loop_device: Option<LoopDevice>,
}

impl Overlay {
    fn same_device(&self, other: &Self) -> bool {
        let mut expected = self.clone();
        expected.firmware.clone_from(&other.firmware);
        expected == *other
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Mount {
    pub device: String,
    pub identity: String,
    pub point: String,
    pub root: String,
    pub filesystem: String,
    pub options: String,
    pub super_options: String,
}

fn unescape(text: &str) -> Result<String> {
    let mut bytes = Vec::new();
    let mut source = text.bytes();
    while let Some(byte) = source.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        let a = source.next().context("Incomplete mount escape")?;
        let b = source.next().context("Incomplete mount escape")?;
        let c = source.next().context("Incomplete mount escape")?;
        if ![a, b, c].iter().all(|digit| (b'0'..=b'7').contains(digit)) {
            bail!("Invalid mount escape");
        }
        let value =
            u8::try_from(u16::from(a - b'0') * 64 + u16::from(b - b'0') * 8 + u16::from(c - b'0'))?;
        if value == 0 {
            bail!("Invalid mount path");
        }
        bytes.push(value);
    }
    Ok(String::from_utf8(bytes)?)
}

pub(crate) fn mounts(text: &str) -> Result<Vec<Mount>> {
    text.lines()
        .map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let separator = fields
                .iter()
                .position(|field| *field == "-")
                .context("Invalid mount table")?;
            if separator < 6 || fields.len() < separator + 4 {
                bail!("Invalid mount table");
            }
            Ok(Mount {
                identity: fields[2].into(),
                root: unescape(fields[3])?,
                point: unescape(fields[4])?,
                options: fields[5].into(),
                filesystem: fields[separator + 1].into(),
                device: unescape(fields[separator + 2])?,
                super_options: fields[separator + 3].into(),
            })
        })
        .collect()
}

fn option<'a>(options: &'a str, name: &str) -> Option<&'a str> {
    options.split(',').find_map(|part| part.strip_prefix(name))
}

fn layout(entries: &[Mount]) -> Result<&Mount> {
    let root = entries
        .iter()
        .find(|entry| entry.point == "/")
        .context("Root mount not found")?;
    let rom = entries
        .iter()
        .find(|entry| entry.point == "/rom")
        .context("Clean recovery requires a read-only /rom firmware")?;
    if root.filesystem != "overlay"
        || rom.filesystem != "squashfs"
        || !rom.options.split(',').any(|part| part == "ro")
        || option(&root.super_options, "upperdir=") != Some("/overlay/upper")
        || option(&root.super_options, "workdir=") != Some("/overlay/work")
        || !matches!(option(&root.super_options, "lowerdir="), Some("/" | "/rom"))
    {
        bail!(
            "Clean recovery supports squashfs firmware with /overlay/upper and /overlay/work; flat or custom root layouts are not changed"
        );
    }
    let overlay = entries
        .iter()
        .find(|entry| entry.point == "/overlay")
        .context("Persistent overlay is not mounted")?;
    if !matches!(overlay.filesystem.as_str(), "ext4" | "f2fs")
        || overlay.root != "/"
        || !overlay.options.split(',').any(|part| part == "rw")
        || !overlay.device.starts_with("/dev/")
    {
        bail!("Clean recovery requires a writable ext4/f2fs block filesystem mounted at /overlay");
    }
    for entry in entries {
        if entry.point.starts_with("/overlay/upper/")
            || entry.point.starts_with("/overlay/work/")
            || (entry.filesystem == "overlay"
                && entry.point != "/"
                && (option(&entry.super_options, "upperdir=") == Some("/overlay/upper")
                    || option(&entry.super_options, "workdir=") == Some("/overlay/work")))
        {
            bail!(
                "Nested mounts or another overlay using the system upper/work must be detached before clean recovery"
            );
        }
    }
    Ok(overlay)
}

pub(crate) fn regular_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || path.canonicalize()? != path {
        bail!(
            "Overlay directory must not be a symlink or alias: {}",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn block_identity(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)?;
    if !metadata.file_type().is_block_device() {
        bail!("Overlay source is not a block device: {}", path.display());
    }
    Ok(format!(
        "{}:{}",
        libc::major(metadata.rdev()),
        libc::minor(metadata.rdev())
    ))
}

pub fn discover(root: &Path) -> Result<Overlay> {
    let entries = mounts(&fs::read_to_string(root.join("proc/self/mountinfo"))?)?;
    let mount = layout(&entries)?;
    for name in ["overlay", "overlay/upper", "overlay/work"] {
        regular_dir(&root.join(name))?;
    }
    let device = root.join(mount.device.trim_start_matches('/'));
    if block_identity(&device)? != mount.identity {
        bail!("Overlay device changed during inspection");
    }
    let name = device
        .file_name()
        .context("Overlay device has no name")?
        .to_string_lossy();
    let loop_device = if name.strip_prefix("loop").is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        let directory = root.join("sys/block").join(name.as_ref()).join("loop");
        let backing = fs::read_to_string(directory.join("backing_file"))?;
        let backing = backing.trim();
        let backing = format!(
            "/dev/{}",
            backing
                .strip_prefix("/dev/")
                .unwrap_or(backing.trim_start_matches('/'))
        );
        let path = root.join(backing.trim_start_matches('/'));
        Some(LoopDevice {
            identity: block_identity(&path)
                .context("File-backed loop overlays are not supported")?,
            backing,
            offset: fs::read_to_string(directory.join("offset"))?
                .trim()
                .parse()?,
            size_limit: fs::read_to_string(directory.join("sizelimit"))?
                .trim()
                .parse()?,
        })
    } else {
        None
    };
    Ok(Overlay {
        device: mount.device.clone(),
        identity: mount.identity.clone(),
        filesystem: mount.filesystem.clone(),
        mountpoint: "/overlay".into(),
        firmware: digest_file(&root.join("rom/etc/openwrt_release"))?,
        loop_device,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Journal {
    id: String,
    nonce: String,
    overlay: Overlay,
    phase: String,
    #[serde(default)]
    original_lan: String,
    #[serde(default)]
    activation: Option<Activation>,
    #[serde(default)]
    payload_on_overlay: bool,
}

#[derive(Deserialize, Serialize)]
struct Transition {
    id: String,
    nonce: String,
    overlay: Overlay,
    rollback: bool,
    #[serde(default)]
    activation: Option<Activation>,
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn directory(overlay: &Path, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        bail!("Invalid clean recovery task ID");
    }
    regular_dir(overlay)?;
    let store = overlay.join(STORE);
    if store.exists() || store.symlink_metadata().is_ok() {
        regular_dir(&store)?;
    }
    let path = store.join(id);
    if path.exists() || path.symlink_metadata().is_ok() {
        regular_dir(&path)?;
    }
    Ok(path)
}

fn summary(journal: &Journal) -> Value {
    json!({
        "retained": true, "phase": journal.phase, "device": journal.overlay.device,
        "filesystem": journal.overlay.filesystem,
        "original_lan": journal.original_lan,
        "activates_extroot": journal.activation.is_some(),
        "directory": format!("/overlay/{STORE}/{}", journal.id),
    })
}

fn copy_tree(source: &Path, destination: &Path, links: bool) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.is_file() {
        mkdir(destination.parent().context("Missing parent")?, 0o700)?;
        if !links {
            let mut source_file = crate::util::open_regular(source)?;
            let mut target = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(metadata.permissions().mode() & 0o777)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(destination)?;
            std::io::copy(&mut source_file, &mut target)?;
        } else if fs::hard_link(source, destination).is_err() {
            atomic_copy(destination, source, metadata.permissions().mode() & 0o777)?;
        }
    } else if metadata.is_dir() {
        mkdir(destination, metadata.permissions().mode() & 0o777)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &destination.join(entry.file_name()), links)?;
        }
    } else if metadata.file_type().is_symlink() && !links {
        mkdir(destination.parent().context("Missing parent")?, 0o700)?;
        symlink(fs::read_link(source)?, destination)?;
    } else {
        bail!("Unexpected file in recovery records: {}", source.display());
    }
    Ok(())
}

struct Reserve(PathBuf);
impl Reserve {
    fn create(base: &Path) -> Result<Self> {
        let path = base.join("reserve");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        let reserve = Self(path);
        // Release this space on success or error, leaving room to record a
        // failed APK transaction even if its inactive root filled the disk.
        for _ in 0..128 {
            file.write_all(&[0; 65536])?;
        }
        file.sync_all()?;
        Ok(reserve)
    }
}
impl Drop for Reserve {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn sync_filesystem(path: &Path) -> Result<()> {
    let file = File::open(path)?;
    // SAFETY: SYS_syncfs takes a held-open descriptor and flushes only its
    // filesystem; it does not dereference any user pointers.
    if unsafe { libc::syscall(libc::SYS_syncfs, file.as_raw_fd()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn copy_current(jobs: &Jobs, upper: &Path, relative: &str) -> Result<()> {
    let source = jobs.root.join(relative);
    match fs::symlink_metadata(&source) {
        Ok(metadata) if metadata.is_file() => atomic_copy(
            &upper.join(relative),
            &source,
            metadata.permissions().mode() & 0o777,
        ),
        Ok(_) => bail!("Bootstrap configuration is not a regular file: {relative}"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn bootstrap(jobs: &Jobs, id: &str, upper: &Path, options: &Options) -> Result<()> {
    // Resolve packages against the *firmware* database, never the old upper's
    // database. --no-scripts keeps package hooks from running in the live root.
    for relative in ["lib/apk", "etc/apk"] {
        copy_tree(
            &jobs.root.join("rom").join(relative),
            &upper.join(relative),
            false,
        )?;
    }
    // Keep /var volatile after activation, without an absolute symlink that
    // could escape the inactive root while APK writes its temporary files.
    if jobs
        .root
        .join("rom/var")
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        mkdir(&upper.join("tmp"), 0o1777)?;
        symlink("tmp", upper.join("var"))?;
    }
    let repository = upper.join("etc/apk/repositories.d/99-overlay-restore.list");
    atomic_write(
        &repository,
        format!("@myfeed {}\n", options.myfeed_repo).as_bytes(),
        0o600,
    )?;
    let key = upper.join("etc/apk/keys/overlay-restore-myfeed.pem");
    if crate::local_feed::trusted_key(jobs, &key).is_err() {
        if options.uses_local_feed() {
            bail!("The trusted myfeed key is missing for local clean recovery");
        }
        let key_path = key.to_str().context("Invalid key path")?;
        let (code, output) = jobs.run(
            &[
                "uclient-fetch",
                "-q",
                "-O",
                key_path,
                &options.myfeed_key_url,
            ],
            Some(id),
            120,
        )?;
        if code != 0 {
            bail!("Unable to prepare trusted myfeed key: {output}");
        }
    }
    let local_cache = crate::local_feed::task(jobs, id, true)?;
    if local_cache.is_some() {
        jobs.log(id, "Preparing the clean environment from the signed local feed without repository network access.")?;
    }
    let run_apk = |arguments: &[&str], timeout| -> Result<(i32, String)> {
        if let Some(cache) = &local_cache {
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
        } else {
            jobs.run(arguments, Some(id), timeout)
        }
    };
    let target = upper.to_str().context("Invalid bootstrap path")?;
    let (code, output) = run_apk(
        &[
            "apk",
            "--root",
            target,
            "--no-logfile",
            "--wait",
            "30",
            "update",
        ],
        180,
    )?;
    if code != 0 {
        bail!("Repositories unavailable for the clean environment: {output}");
    }
    let luci_packages = crate::luci::packages(&jobs.root, options)?;
    let mut arguments = vec![
        "apk",
        "--root",
        target,
        "--no-logfile",
        "--wait",
        "30",
        "--no-scripts",
        "--no-commit-hooks",
        "add",
        "--upgrade",
        "--latest",
        "overlay-restore@myfeed>=0.2.0-r16",
        "luci-app-overlay-restore@myfeed>=0.2.0-r19",
    ];
    arguments.extend(luci_packages.iter().map(String::as_str));
    for (name, argument) in [("smartdns", "smartdns@myfeed"), ("nikki", "nikki@myfeed")] {
        if options
            .myfeed_packages
            .iter()
            .any(|package| package == name)
        {
            arguments.push(argument);
        }
    }
    let (code, output) = run_apk(&arguments, 600)?;
    if code != 0 {
        bail!(
            "Unable to install recovery and DNS/proxy packages into the clean environment: {output}"
        );
    }
    let mut verification = vec!["apk", "--root", target, "list", "--installed"];
    verification.extend(luci_packages.iter().map(String::as_str));
    let (code, output) = run_apk(&verification, 60)?;
    if code != 0 {
        bail!("Unable to verify LuCI packages in the clean environment: {output}");
    }
    crate::luci::verify_versions(&output, &luci_packages)?;
    fs::remove_file(repository)?;
    // Keep the feed available for normal installs and for the bootstrap world
    // constraints. Recovery restores both entries after its transaction.
    atomic_write(
        &upper.join("etc/apk/repositories.d/00-myfeed.list"),
        format!("{}\n@myfeed {}\n", options.myfeed_repo, options.myfeed_repo).as_bytes(),
        0o600,
    )?;
    for relative in [
        "usr/sbin/overlay-restore",
        "usr/libexec/overlay-restore-switch",
        "lib/upgrade/zz-overlay-restore.sh",
        "etc/init.d/overlay-restore",
    ] {
        if !upper.join(relative).is_file() {
            bail!("Clean environment is missing {relative}");
        }
    }
    for relative in [
        "etc/config/network",
        "etc/config/firewall",
        "etc/config/dhcp",
        "etc/config/wireless",
        "etc/config/fstab",
        "etc/config/overlay_restore",
        "etc/passwd",
        "etc/shadow",
        "etc/group",
        "etc/board.json",
    ] {
        copy_current(jobs, upper, relative)?;
    }
    let dropbear = jobs.root.join("etc/dropbear");
    if dropbear.is_dir() {
        for entry in fs::read_dir(dropbear)? {
            let entry = entry?;
            if entry.file_name() == "authorized_keys"
                || entry.file_name().to_string_lossy().ends_with("_host_key")
            {
                copy_current(
                    jobs,
                    upper,
                    &format!("etc/dropbear/{}", entry.file_name().to_string_lossy()),
                )?;
            }
        }
    }
    copy_current(jobs, upper, "root/.ssh/authorized_keys")?;
    mkdir(&upper.join("etc/rc.d"), 0o755)?;
    for name in ["overlay-restore", "smartdns", "nikki"] {
        let init = upper.join("etc/init.d").join(name);
        if !init.is_file() {
            continue;
        }
        let start = fs::read_to_string(init)?
            .lines()
            .find_map(|line| {
                line.strip_prefix("START=")
                    .and_then(|number| number.parse::<u8>().ok())
            })
            .filter(|number| *number > 0 && *number < 100)
            .context("Bootstrap init script has no valid START")?;
        let link = upper.join("etc/rc.d").join(format!("S{start:02}{name}"));
        if !link.symlink_metadata().is_ok() {
            symlink(format!("../init.d/{name}"), link)?;
        }
    }
    Ok(())
}

fn new_destination(upper: &Path, relative: &str) -> Result<PathBuf> {
    if crate::archive::safe_name(relative)? != relative || relative.is_empty() {
        bail!("Invalid clean recovery destination");
    }
    let path = upper.join(relative);
    for parent in path.ancestors().skip(1) {
        if parent == upper {
            break;
        }
        if parent.symlink_metadata().is_ok() {
            regular_dir(parent)?;
        }
    }
    Ok(path)
}

fn inspected_target(
    jobs: &Jobs,
    options: &Options,
    plan: &Plan,
) -> Result<(Overlay, Option<Activation>)> {
    let (overlay, activation) = crate::extroot::select(jobs, &options.overlay_device)?;
    let inspected: Overlay = serde_json::from_value(plan.settings["overlay_target"].clone())?;
    if overlay != inspected {
        bail!("Overlay or firmware changed after inspection; inspect the backup again");
    }
    if let Some(activation) = &activation {
        let origin: Overlay = serde_json::from_value(plan.settings["overlay_origin"].clone())?;
        if activation.origin != origin || plan.settings["extroot_uuid"] != activation.uuid {
            bail!("Extroot selection changed after inspection; inspect the backup again");
        }
    }
    Ok((overlay, activation))
}

fn retained_directory(jobs: &Jobs, target: &Path, id: &str) -> Result<PathBuf> {
    for state in jobs.states()? {
        if state["id"] != id && !Jobs::can_cleanup(&state) && state["status"] != "ready" {
            bail!("Finish other recovery tasks before rebuilding overlay");
        }
    }
    let base = directory(target, id)?;
    let store = base.parent().context("Missing clean store")?;
    if store.exists()
        && fs::read_dir(store)?.any(|entry| entry.is_ok_and(|entry| entry.path() != base))
    {
        bail!(
            "An earlier overlay is still retained; roll it back or explicitly discard it before another clean recovery"
        );
    }
    Ok(base)
}

pub fn stage_payload(
    jobs: &Jobs,
    id: &str,
    options: &Options,
    plan: &Plan,
    payload: &Path,
) -> Result<Value> {
    let task = jobs.path(id)?;
    let (overlay, mut activation) = inspected_target(jobs, options, plan)?;
    let target = Target::open(jobs, &overlay, id)?;
    let base = retained_directory(jobs, &target.path, id)?;
    if base.exists() {
        bail!("Discard the incomplete clean environment before inspecting the backup again");
    }
    require_space(
        &target.path,
        2 * plan.selected_bytes + 24 * 1024 * 1024,
        &format!("staged files and a clean overlay on {}", overlay.device),
    )?;
    if let Some(activation) = &mut activation {
        activation.original_extroot_uuid = crate::extroot::boot_marker(&target.path)?;
    }
    mkdir(&base, 0o700)?;
    let mut journal = Journal {
        id: id.into(),
        nonce: random_hex(16)?,
        overlay,
        activation,
        phase: "staging".into(),
        original_lan: fs::read_to_string(jobs.root.join("etc/config/network"))
            .ok()
            .and_then(|text| crate::archive::lan_address(&text).ok())
            .unwrap_or_default(),
        payload_on_overlay: true,
    };
    save_json(&base.join("journal.json"), &journal)?;
    save_json(&task.join("clean-overlay.json"), &journal)?;
    let mut state = jobs.load(id)?;
    state["clean_overlay"] = summary(&journal);
    state["status"] = json!("cleaning_overlay");
    jobs.save(&mut state)?;
    let staged = (|| -> Result<()> {
        mkdir(&base.join("payload"), 0o700)?;
        for entry in &plan.files {
            atomic_copy(
                &new_destination(&base.join("payload"), &entry.path)?,
                &payload.join(&entry.path),
                entry.mode,
            )?;
        }
        journal.phase = "staged".into();
        save_json(&base.join("journal.json"), &journal)?;
        save_json(&task.join("clean-overlay.json"), &journal)?;
        sync_filesystem(&target.path)?;
        jobs.log(
            id,
            &format!(
                "Recovery files persisted on {}; the running overlay holds only task records.",
                journal.overlay.device
            ),
        )?;
        Ok(())
    })();
    if let Err(error) = staged {
        state["status"] = json!("failed_clean");
        state["error"] = json!(error.to_string());
        jobs.log(
            id,
            &format!("Unable to persist clean recovery files; running overlay retained: {error}"),
        )?;
        jobs.save(&mut state)?;
        return Err(error);
    }
    Ok(summary(&journal))
}

fn prepare(jobs: &Jobs, id: &str) -> Result<Journal> {
    let task = jobs.path(id)?;
    let plan: Plan = read_json(&task.join("plan.json"))?;
    let mut options: Options = read_json(&task.join("options.json"))?;
    options.myfeed_repo = plan.myfeed_repo.clone();
    options.validate()?;
    let (overlay, mut activation) = inspected_target(jobs, &options, &plan)?;
    let target = Target::open(jobs, &overlay, id)?;
    if let Some(activation) = &mut activation {
        activation.original_extroot_uuid = crate::extroot::boot_marker(&target.path)?;
    }
    let base = retained_directory(jobs, &target.path, id)?;
    let mut journal = if base.join("journal.json").is_file() {
        let journal: Journal = read_json(&base.join("journal.json"))?;
        if journal.id != id || journal.overlay != overlay {
            bail!("Clean recovery journal changed");
        }
        if journal.phase == "prepared"
            || (journal.phase == "switched" && journal.activation.is_some())
        {
            return Ok(journal);
        }
        let cached: Journal = read_json(&task.join("clean-overlay.json"))?;
        if journal.phase != "staged"
            || !journal.payload_on_overlay
            || !valid_id(&journal.nonce)
            || cached.nonce != journal.nonce
            || journal.activation != activation
        {
            bail!("Discard the incomplete clean environment before retrying");
        }
        journal
    } else {
        Journal {
            id: id.into(),
            nonce: random_hex(16)?,
            overlay,
            activation,
            phase: "building".into(),
            original_lan: fs::read_to_string(jobs.root.join("etc/config/network"))
                .ok()
                .and_then(|text| crate::archive::lan_address(&text).ok())
                .unwrap_or_default(),
            payload_on_overlay: false,
        }
    };
    require_space(
        &target.path,
        plan.selected_bytes * if journal.payload_on_overlay { 1 } else { 2 } + 24 * 1024 * 1024,
        &format!("a clean overlay on {}", journal.overlay.device),
    )?;
    if journal.activation.is_some() {
        for name in ["upper", "work"] {
            let path = target.path.join(name);
            if !path.exists() {
                mkdir(&path, 0o755)?;
            }
            regular_dir(&path)?;
        }
    }
    mkdir(&base, 0o700)?;
    journal.phase = "building".into();
    save_json(&base.join("journal.json"), &journal)?;
    save_json(&task.join("clean-overlay.json"), &journal)?;
    let mut state = jobs.load(id)?;
    state["clean_overlay"] = summary(&journal);
    state["status"] = json!("cleaning_overlay");
    jobs.save(&mut state)?;
    let reserve = Reserve::create(&base)?;
    jobs.log(id, "Preparing a clean overlay from the current firmware; the running upper and external data directories are retained.")?;
    let upper = base.join("alternate-upper");
    mkdir(&upper, 0o755)?;
    mkdir(&base.join("alternate-work"), 0o755)?;
    bootstrap(jobs, id, &upper, &options)?;
    let payload = if journal.payload_on_overlay {
        base.join("payload")
    } else {
        task.join("payload")
    };
    for entry in &plan.files {
        let source = payload.join(&entry.path);
        if digest_file(&source)? != entry.sha256 {
            bail!("Persistent payload changed: {}", entry.path);
        }
        atomic_copy(&new_destination(&upper, &entry.path)?, &source, entry.mode)?;
    }
    if let Some(activation) = &journal.activation {
        let fstab = upper.join("etc/config/fstab");
        let current = fs::read_to_string(&fstab).unwrap_or_default();
        atomic_write(
            &fstab,
            crate::extroot::configured_fstab(&current, &activation.uuid)?.as_bytes(),
            0o600,
        )?;
    }
    // Share immutable history on the same filesystem; extroot activation
    // copies records that still live on the original filesystem.
    copy_tree(
        &jobs.directory,
        &upper.join("etc/overlay-restore/jobs"),
        true,
    )?;
    if journal.payload_on_overlay {
        // The payload is already on this filesystem, so history can share its
        // immutable files without allocating another copy in the new upper.
        copy_tree(
            &payload,
            &upper
                .join("etc/overlay-restore/jobs")
                .join(id)
                .join("payload"),
            true,
        )?;
    }
    for previous in jobs.states()? {
        if previous["id"] != id && previous["status"] == "ready" {
            let other = previous["id"].as_str().context("Task has no ID")?;
            let path = upper
                .join("etc/overlay-restore/jobs")
                .join(other)
                .join("state.json");
            let mut expired = previous;
            expired["status"] = json!("failed_validation");
            expired["error"] =
                json!("Preview expired during clean recovery; inspect the backup again");
            save_json(&path, &expired)?;
        }
    }
    atomic_write(&upper.join(MARKER), id.as_bytes(), 0o600)?;
    journal.phase = "prepared".into();
    save_json(&base.join("journal.json"), &journal)?;
    save_json(&task.join("clean-overlay.json"), &journal)?;
    state = jobs.load(id)?;
    state["status"] = json!("awaiting_reboot");
    state["completed_files"] = json!(
        plan.files
            .iter()
            .map(|entry| &entry.path)
            .collect::<Vec<_>>()
    );
    state["clean_overlay"] = summary(&journal);
    save_json(
        &upper
            .join("etc/overlay-restore/jobs")
            .join(id)
            .join("state.json"),
        &state,
    )?;
    // The old root records the pending transition, so an interruption before
    // the atomic upper exchange can resume safely from the old environment.
    state["status"] = json!("cleaning_overlay");
    state["completed_files"] = json!([]);
    jobs.save(&mut state)?;
    jobs.log(
        id,
        "Clean environment prepared; stopping services and switching from RAM before reboot.",
    )?;
    drop(reserve);
    sync_filesystem(&target.path)?;
    Ok(journal)
}

fn transition(jobs: &Jobs, journal: &Journal, rollback: bool) -> Result<()> {
    let manifest = Transition {
        id: journal.id.clone(),
        nonce: journal.nonce.clone(),
        overlay: journal.overlay.clone(),
        rollback,
        activation: journal.activation.clone(),
    };
    save_json(
        &jobs.root.join(RAM_MANIFEST.trim_start_matches('/')),
        &manifest,
    )?;
    let (code, output) = jobs.run(
        &["/usr/libexec/validate_firmware_image", RAM_MANIFEST],
        None,
        30,
    )?;
    let validation: Value = serde_json::from_str(&output)
        .context("OpenWrt RAM transition validation is unavailable")?;
    if code != 0 || validation["forceable"] != true {
        bail!("This firmware cannot start the controlled RAM transition");
    }
    let request = json!({
        "prefix": format!("/tmp/overlay-restore-ramfs-{}", journal.id), "path": RAM_MANIFEST,
        "command": RAM_COMMAND, "force": true, "options": {},
    })
    .to_string();
    let mut state = jobs.load(&journal.id)?;
    state["status"] = json!(if rollback {
        "awaiting_rollback_boot"
    } else {
        "awaiting_clean_boot"
    });
    state["transition_boot"] = json!(jobs.current_boot()?);
    jobs.save(&mut state)?;
    let (code, output) = jobs.run(
        &["/usr/libexec/overlay-restore-switch", &journal.id, &request],
        None,
        30,
    )?;
    if code != 0 {
        bail!("Unable to enter RAM for overlay switching: {output}");
    }
    Ok(())
}

pub fn prepare_and_switch(jobs: &Jobs, id: &str) -> Result<()> {
    let _jobs = jobs.lock("jobs", false)?;
    if let Err(error) = prepare(jobs, id).and_then(|journal| transition(jobs, &journal, false)) {
        let mut state = jobs.load(id)?;
        state["status"] = json!("failed_clean");
        state["error"] = json!(error.to_string());
        jobs.log(
            id,
            &format!("Clean overlay preparation failed; running overlay retained: {error}"),
        )?;
        jobs.save(&mut state)?;
    }
    Ok(())
}

fn read_journal(jobs: &Jobs, id: &str) -> Result<(Target, PathBuf, Journal)> {
    let task = jobs.path(id)?;
    let current = discover(&jobs.root)?;
    let cached: Option<Journal> = read_json(&task.join("clean-overlay.json")).ok();
    let overlay = if let Some(cached) = &cached {
        if cached.id != id || !valid_id(&cached.nonce) {
            bail!("Invalid retained overlay record");
        }
        if !cached.overlay.same_device(&current) {
            let activation = cached
                .activation
                .as_ref()
                .context("Retained overlay device changed")?;
            if !activation.origin.same_device(&current) {
                bail!("The original system overlay changed");
            }
            crate::extroot::verify_uuid(jobs, &cached.overlay.device, &activation.uuid)?;
        }
        cached.overlay.clone()
    } else {
        current
    };
    let target = Target::open(jobs, &overlay, id)?;
    let base = directory(&target.path, id)?;
    let journal: Journal = read_json(&base.join("journal.json"))?;
    if journal.id != id || !journal.overlay.same_device(&overlay) || !valid_id(&journal.nonce) {
        bail!("Retained overlay does not match the current device");
    }
    if let Some(cached) = cached
        && (cached.nonce != journal.nonce || cached.activation != journal.activation)
    {
        bail!("Retained overlay identity changed");
    }
    Ok((target, base, journal))
}

pub fn decorate(jobs: &Jobs, state: &mut Value) -> Result<()> {
    state["can_rollback_overlay"] = json!(false);
    state["can_discard_overlay"] = json!(false);
    if state["clean_overlay"]["retained"] != true {
        return Ok(());
    }
    let id = state["id"].as_str().context("Task has no ID")?.to_owned();
    // Status polling never mounts an inactive disk. The private cached journal
    // locates it; a confirmed action reopens the actual filesystem and nonce.
    if let Ok(cached) = read_json::<Journal>(&jobs.path(&id)?.join("clean-overlay.json"))
        && let Some(activation) = &cached.activation
        && discover(&jobs.root).is_ok_and(|current| activation.origin.same_device(&current))
    {
        state["clean_overlay"]["directory"] =
            json!(format!("{}:/{STORE}/{id}", cached.overlay.device));
        state["can_discard_overlay"] = json!(
            ["rolled_back", "failed_clean"].contains(&state["status"].as_str().unwrap_or(""))
        );
        return Ok(());
    }
    let (_target, base, journal) = match read_journal(jobs, &id) {
        Ok(value) => value,
        Err(error) => {
            state["clean_overlay"]["unavailable"] = json!(error.to_string());
            return Ok(());
        }
    };
    state["clean_overlay"] = summary(&journal);
    let same_firmware =
        digest_file(&jobs.root.join("rom/etc/openwrt_release"))? == journal.overlay.firmware;
    if !same_firmware {
        state["clean_overlay"]["unavailable"] = json!(
            "Firmware changed since this recovery; the retained environment can be discarded but cannot be activated automatically."
        );
    }
    let marker = jobs.root.join("overlay/upper").join(MARKER);
    let switched = fs::read_to_string(marker).is_ok_and(|value| value == id);
    state["can_rollback_overlay"] = json!(
        switched
            && same_firmware
            && ["complete", "complete_with_warnings", "failed_packages"]
                .contains(&state["status"].as_str().unwrap_or(""))
    );
    state["can_discard_overlay"] = json!(
        [
            "complete",
            "complete_with_warnings",
            "rolled_back",
            "failed_clean"
        ]
        .contains(&state["status"].as_str().unwrap_or(""))
            && base.is_dir()
    );
    Ok(())
}

pub fn finalize(jobs: &Jobs, id: &str) -> Result<()> {
    let mut state = jobs.load(id)?;
    if state["clean_overlay"]["retained"] != true {
        return Ok(());
    }
    let (_target, base, mut journal) = read_journal(jobs, id)?;
    if digest_file(&jobs.root.join("rom/etc/openwrt_release"))? != journal.overlay.firmware {
        bail!("Firmware changed during clean recovery; package operations were stopped");
    }
    if fs::read_to_string(jobs.root.join("overlay/upper").join(MARKER))? != id {
        bail!("Clean overlay was not activated; package operations were stopped");
    }
    journal.phase = "switched".into();
    save_json(&base.join("journal.json"), &journal)?;
    save_json(&jobs.path(id)?.join("clean-overlay.json"), &journal)?;
    state["clean_overlay"] = summary(&journal);
    jobs.save(&mut state)
}

pub fn interrupted(jobs: &Jobs, id: &str) -> Result<()> {
    let mut state = jobs.load(id)?;
    if state["transition_boot"].as_str() == Some(jobs.current_boot()?.as_str()) {
        return Ok(());
    }
    let rollback = state["status"] == "awaiting_rollback_boot";
    state["status"] = if rollback {
        state["before_rollback"].clone()
    } else {
        json!("failed_clean")
    };
    state["error"] = json!(
        "Overlay transition was interrupted before activation; the running environment was retained. Retry manually."
    );
    jobs.log(
        id,
        "RAM transition was interrupted; no automatic repeat will be attempted.",
    )?;
    jobs.save(&mut state)
}

pub fn rollback(jobs: &Jobs, id: &str, confirmation: &str) -> Result<Value> {
    if id != confirmation {
        bail!("Overlay rollback must be explicitly confirmed");
    }
    let _jobs = jobs.lock("jobs", false)?;
    let _task = jobs.lock(&format!("task-{id}"), false)?;
    jobs.require_idle()?;
    let mut state = jobs.load(id)?;
    decorate(jobs, &mut state)?;
    if state["can_rollback_overlay"] != true {
        bail!("This task has no available overlay rollback");
    }
    state["before_rollback"] = state["status"].clone();
    state["status"] = json!("rolling_back");
    jobs.save(&mut state)?;
    Ok(state)
}

pub fn switch_back(jobs: &Jobs, id: &str) -> Result<()> {
    let result =
        read_journal(jobs, id).and_then(|(_, _, journal)| transition(jobs, &journal, true));
    if let Err(error) = result {
        let mut state = jobs.load(id)?;
        state["status"] = state["before_rollback"].clone();
        state["error"] = json!(format!("Unable to start overlay rollback: {error}"));
        jobs.log(
            id,
            &format!("Rollback did not start; current environment retained: {error}"),
        )?;
        jobs.save(&mut state)?;
    }
    Ok(())
}

fn has_snapshot_mount(entries: &[Mount], identity: &str, id: &str) -> bool {
    let relative = format!("/{STORE}/{id}");
    entries.iter().any(|entry| {
        (entry.identity == identity
            && (entry.root == relative || entry.root.starts_with(&format!("{relative}/"))))
            || entries
                .iter()
                .filter(|alias| alias.identity == identity && alias.root == "/")
                .any(|alias| {
                    let prefix = format!("{}{relative}", alias.point.trim_end_matches('/'));
                    entry.point == prefix || entry.point.starts_with(&format!("{prefix}/"))
                })
    })
}

pub fn discard(jobs: &Jobs, id: &str, confirmation: &str) -> Result<Value> {
    if id != confirmation {
        bail!("Discarding the retained overlay must be explicitly confirmed");
    }
    let _jobs = jobs.lock("jobs", false)?;
    let _task = jobs.lock(&format!("task-{id}"), false)?;
    jobs.require_idle()?;
    let mut state = jobs.load(id)?;
    decorate(jobs, &mut state)?;
    if state["can_discard_overlay"] != true {
        bail!("Finish recovery or roll back before discarding the alternate overlay");
    }
    let (_target, base, journal) = read_journal(jobs, id)?;
    let entries = mounts(&fs::read_to_string(jobs.root.join("proc/self/mountinfo"))?)?;
    if has_snapshot_mount(&entries, &journal.overlay.identity, id) {
        bail!("The retained overlay contains a mount; unmount it before discarding");
    }
    // base is a verified directory under /overlay/.overlay-restore-clean/<id>.
    // Rust's remove_dir_all does not follow symbolic links in the old tree.
    fs::remove_dir_all(&base)?;
    sync_parent(&base)?;
    state["clean_overlay"]["retained"] = json!(false);
    jobs.log(id, "The inactive alternate overlay was explicitly discarded; the active upper/work and other external data were retained.")?;
    jobs.save(&mut state)?;
    Ok(state)
}

fn exchange(first: &Path, second: &Path) -> Result<()> {
    regular_dir(first)?;
    regular_dir(second)?;
    let first_c = CString::new(first.as_os_str().as_bytes())?;
    let second_c = CString::new(second.as_os_str().as_bytes())?;
    // SAFETY: both paths are valid nul-terminated strings; RENAME_EXCHANGE
    // atomically swaps two existing directories on the same filesystem.
    if unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            first_c.as_ptr(),
            libc::AT_FDCWD,
            second_c.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    sync_parent(first)?;
    sync_parent(second)
}

fn swap(overlay: &Path, journal: &mut Journal, rollback: bool) -> Result<()> {
    let base = directory(overlay, &journal.id)?;
    let upper = overlay.join("upper");
    let alternate = base.join("alternate-upper");
    for path in [
        &upper,
        &alternate,
        &overlay.join("work"),
        &base.join("alternate-work"),
    ] {
        regular_dir(path)?;
    }
    let switched = fs::read_to_string(upper.join(MARKER)).is_ok_and(|value| value == journal.id);
    if switched == rollback {
        if !rollback && fs::read_to_string(alternate.join(MARKER))? != journal.id {
            bail!("Prepared upper does not belong to the confirmed task");
        }
        // Work is scratch space. Exchange it first; the atomic upper exchange
        // is the commit point, so a power loss never leaves /upper absent.
        exchange(&overlay.join("work"), &base.join("alternate-work"))?;
        if rollback && journal.activation.is_none() {
            let path = alternate
                .join("etc/overlay-restore/jobs")
                .join(&journal.id)
                .join("state.json");
            let mut state: Value = read_json(&path)?;
            state["status"] = json!("rolled_back");
            state["error"] = json!("");
            state["updated"] = json!(now());
            journal.phase = "rolled_back".into();
            state["clean_overlay"] = summary(journal);
            save_json(&path, &state)?;
        }
        exchange(&upper, &alternate)?;
    }
    journal.phase = if rollback { "rolled_back" } else { "switched" }.into();
    save_json(&base.join("journal.json"), journal)
}

// Linux's LOOP_CONFIGURE atomically supplies the backing descriptor and offset.
// Keep the returned descriptor open until after mounting: AUTOCLEAR devices
// otherwise detach as soon as the last opener exits.
#[repr(C)]
struct LoopInfo {
    device: u64,
    inode: u64,
    rdevice: u64,
    offset: u64,
    size_limit: u64,
    number: u32,
    encrypt_type: u32,
    encrypt_key_size: u32,
    flags: u32,
    file_name: [u8; 64],
    crypt_name: [u8; 64],
    key: [u8; 32],
    init: [u64; 2],
}

impl Default for LoopInfo {
    fn default() -> Self {
        Self {
            device: 0,
            inode: 0,
            rdevice: 0,
            offset: 0,
            size_limit: 0,
            number: 0,
            encrypt_type: 0,
            encrypt_key_size: 0,
            flags: 0,
            file_name: [0; 64],
            crypt_name: [0; 64],
            key: [0; 32],
            init: [0; 2],
        }
    }
}

#[repr(C)]
struct LoopConfig {
    fd: u32,
    block_size: u32,
    info: LoopInfo,
    reserved: [u64; 8],
}

pub(crate) fn restore_loop(overlay: &Overlay) -> Result<Option<File>> {
    let Some(specification) = &overlay.loop_device else {
        return Ok(None);
    };
    let backing = Path::new(&specification.backing);
    if block_identity(backing)? != specification.identity {
        bail!("Loop backing device changed");
    }
    let source = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(backing)?;
    let loop_file = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(&overlay.device)?;
    let mut existing = LoopInfo::default();
    // SAFETY: the ioctl writes a Linux loop_info64 into the correctly sized
    // repr(C) structure. File descriptors are held open for the whole call.
    let result = unsafe { libc::ioctl(loop_file.as_raw_fd(), 0x4C05, &mut existing) };
    if result == 0 {
        if existing.offset != specification.offset
            || existing.size_limit != specification.size_limit
            || existing.rdevice != source.metadata()?.rdev()
        {
            bail!("Loop device is already in use by another backing device");
        }
    } else {
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENXIO) {
            return Err(std::io::Error::last_os_error().into());
        }
        let configuration = LoopConfig {
            fd: source.as_raw_fd() as u32,
            block_size: 0,
            info: LoopInfo {
                offset: specification.offset,
                size_limit: specification.size_limit,
                flags: 4,
                ..LoopInfo::default()
            },
            reserved: [0; 8],
        };
        // SAFETY: LOOP_CONFIGURE reads the repr(C) Linux loop_config structure
        // and its valid backing descriptor; no referenced memory outlives it.
        if unsafe { libc::ioctl(loop_file.as_raw_fd(), 0x4C0A, &configuration) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(Some(loop_file))
}

fn update_origin_files(origin: &Path, journal: &Journal, rollback: bool) -> Result<()> {
    let activation = journal
        .activation
        .as_ref()
        .context("No extroot activation")?;
    regular_dir(&origin.join("upper"))?;
    let fstab = origin.join("upper/etc/config/fstab");
    if rollback {
        // Commit the old-system result before disabling extroot, so a power
        // loss after changing fstab boots the retained original environment.
        let state_path = origin
            .join("upper/etc/overlay-restore/jobs")
            .join(&journal.id)
            .join("state.json");
        let mut state: Value = read_json(&state_path)?;
        state["status"] = json!("rolled_back");
        state["error"] = json!("");
        state["updated"] = json!(now());
        state["clean_overlay"] = summary(journal);
        state["clean_overlay"]["phase"] = json!("rolled_back");
        save_json(&state_path, &state)?;
        atomic_write(&fstab, activation.original_fstab.as_bytes(), 0o600)?;
    } else {
        atomic_write(
            &fstab,
            crate::extroot::configured_fstab(&activation.original_fstab, &activation.uuid)?
                .as_bytes(),
            0o600,
        )?;
    }
    sync_filesystem(origin)
}

fn update_origin(journal: &Journal, rollback: bool) -> Result<()> {
    let activation = journal
        .activation
        .as_ref()
        .context("No extroot activation")?;
    if activation.origin.identity == journal.overlay.identity {
        bail!("Original and selected overlay must be different devices");
    }
    let _loop_file = restore_loop(&activation.origin)?;
    if block_identity(Path::new(&activation.origin.device))? != activation.origin.identity {
        bail!("Original overlay block device changed");
    }
    let target = Path::new(ORIGIN_MOUNT);
    mkdir(target, 0o700)?;
    crate::extroot::mount_device(
        Path::new(&activation.origin.device),
        target,
        &activation.origin.filesystem,
    )?;
    let result = update_origin_files(target, journal, rollback);
    let target_c = CString::new(ORIGIN_MOUNT)?;
    // SAFETY: target_c refers to the original-system mount owned above.
    unsafe {
        libc::umount(target_c.as_ptr());
    }
    result
}

pub fn run_stage(path: &Path) -> Result<()> {
    if path != Path::new(RAM_MANIFEST) || unsafe { libc::geteuid() } != 0 {
        bail!("Clean stage is private to the prepared RAM transition");
    }
    let entries = mounts(&fs::read_to_string("/proc/self/mountinfo")?)?;
    let mut root_stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    let root = CString::new("/")?;
    // SAFETY: root is a valid C string and statfs writes the correctly sized
    // structure. Its contents are inspected only after a successful call.
    if unsafe { libc::statfs(root.as_ptr(), root_stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful statfs initialized every field of root_stat.
    let root_stat = unsafe { root_stat.assume_init() };
    if entries.iter().any(|entry| entry.filesystem == "overlay")
        || i128::from(root_stat.f_type) != i128::from(libc::TMPFS_MAGIC)
        || fs::read_to_string("/proc/1/comm")?.trim() != "upgraded"
    {
        bail!("Refusing to exchange an overlay while the live root is mounted");
    }
    let transition: Transition = read_json(path)?;
    if !valid_id(&transition.id)
        || !valid_id(&transition.nonce)
        || transition.overlay.mountpoint != "/overlay"
        || !matches!(transition.overlay.filesystem.as_str(), "ext4" | "f2fs")
        || !transition.overlay.device.starts_with("/dev/")
    {
        bail!("Invalid RAM transition manifest");
    }
    if let Some(activation) = &transition.activation {
        let output = std::process::Command::new("/sbin/block")
            .args(["info", &transition.overlay.device])
            .output()?;
        if !output.status.success()
            || !crate::extroot::matches_uuid(
                &String::from_utf8(output.stdout)?,
                &transition.overlay.device,
                &activation.uuid,
            )?
        {
            bail!("Selected external filesystem UUID changed before activation");
        }
    }
    let _loop_file = restore_loop(&transition.overlay)?;
    if block_identity(Path::new(&transition.overlay.device))? != transition.overlay.identity {
        bail!("Overlay block device changed before switching");
    }
    mkdir(Path::new(DEVICE_MOUNT), 0o700)?;
    let source = CString::new(transition.overlay.device.as_str())?;
    let target = CString::new(DEVICE_MOUNT)?;
    let filesystem = CString::new(transition.overlay.filesystem.as_str())?;
    // SAFETY: mount reads nul-terminated source/target/type strings. No mount
    // options pointer is supplied; the block device and filesystem were checked.
    if unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            filesystem.as_ptr(),
            libc::MS_NOATIME | libc::MS_NOSUID | libc::MS_NODEV,
            std::ptr::null(),
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let result = (|| {
        let base = directory(Path::new(DEVICE_MOUNT), &transition.id)?;
        let mut journal: Journal = read_json(&base.join("journal.json"))?;
        if journal.id != transition.id
            || journal.nonce != transition.nonce
            || journal.overlay != transition.overlay
            || journal.activation != transition.activation
        {
            bail!("Mounted filesystem is not the one prepared for this recovery");
        }
        if journal.activation.is_some() && transition.rollback {
            update_origin(&journal, true)?;
        }
        swap(Path::new(DEVICE_MOUNT), &mut journal, transition.rollback)?;
        if let Some(activation) = &journal.activation {
            crate::extroot::update_boot_marker(
                Path::new(DEVICE_MOUNT),
                &activation.original_extroot_uuid,
                transition.rollback,
            )?;
            sync_filesystem(Path::new(DEVICE_MOUNT))?;
        }
        if journal.activation.is_some() && !transition.rollback {
            update_origin(&journal, false)?;
        }
        Ok(())
    })();
    // SAFETY: sync takes no arguments, and target remains a valid C string.
    unsafe {
        libc::sync();
        libc::umount(target.as_ptr());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const INTERNAL: &str = "14 24 254:2 / /rom ro,relatime - squashfs /dev/root ro\n\
21 24 7:0 / /overlay rw,noatime - ext4 /dev/loop0 rw\n\
24 1 0:21 / / rw,noatime - overlay overlayfs:/overlay rw,lowerdir=/,upperdir=/overlay/upper,workdir=/overlay/work";
    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn journal() -> Journal {
        Journal {
            id: ID.into(),
            nonce: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            phase: "prepared".into(),
            original_lan: "192.168.1.1".into(),
            activation: None,
            payload_on_overlay: false,
            overlay: Overlay {
                device: "/dev/loop0".into(),
                identity: "7:0".into(),
                filesystem: "ext4".into(),
                mountpoint: "/overlay".into(),
                firmware: "firmware".into(),
                loop_device: None,
            },
        }
    }

    fn trees(overlay: &Path) -> PathBuf {
        mkdir(&overlay.join("upper/root"), 0o755).unwrap();
        mkdir(&overlay.join("work"), 0o755).unwrap();
        let base = directory(overlay, ID).unwrap();
        mkdir(&base.join("alternate-upper/root"), 0o755).unwrap();
        mkdir(&base.join("alternate-work"), 0o755).unwrap();
        atomic_write(&overlay.join("upper/root/old-program"), b"old", 0o644).unwrap();
        atomic_write(
            &base.join("alternate-upper/root/restored"),
            b"restored",
            0o644,
        )
        .unwrap();
        atomic_write(
            &base.join("alternate-upper").join(MARKER),
            ID.as_bytes(),
            0o600,
        )
        .unwrap();
        atomic_write(&overlay.join("outside-data"), b"external data", 0o644).unwrap();
        atomic_write(
            &overlay
                .join("upper/etc/overlay-restore/jobs")
                .join(ID)
                .join("state.json"),
            br#"{"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"cleaning_overlay"}"#,
            0o600,
        )
        .unwrap();
        base
    }

    #[test]
    fn recognizes_internal_loop_and_external_ext4_f2fs() {
        let entries = mounts(INTERNAL).unwrap();
        assert_eq!(layout(&entries).unwrap().device, "/dev/loop0");
        for filesystem in ["ext4", "f2fs"] {
            let external = INTERNAL
                .replace("7:0", "8:1")
                .replace("/dev/loop0", "/dev/sda1")
                .replace("ext4", filesystem);
            let entries = mounts(&external).unwrap();
            let overlay = layout(&entries).unwrap();
            assert_eq!(overlay.device, "/dev/sda1");
            assert_eq!(overlay.filesystem, filesystem);
        }
    }

    #[test]
    fn rejects_flat_tmpfs_readonly_and_custom_roots() {
        for text in [
            INTERNAL.replace("- overlay overlayfs:/overlay", "- ext4 /dev/vda2"),
            INTERNAL.replace("- ext4 /dev/loop0", "- tmpfs tmpfs"),
            INTERNAL.replace("/overlay rw,noatime", "/overlay ro,noatime"),
            INTERNAL.replace("upperdir=/overlay/upper", "upperdir=/mnt/custom/upper"),
            INTERNAL.replace("lowerdir=/,", "lowerdir=/lower:/other,"),
            INTERNAL.replace("/rom ro,relatime", "/rom rw,relatime"),
        ] {
            assert!(layout(&mounts(&text).unwrap()).is_err(), "{text}");
        }
    }

    #[test]
    fn rejects_nested_mounts_and_shared_upper_but_allows_containers_to_stop() {
        for entry in [
            "\n31 24 8:2 / /overlay/upper/root/storage rw - ext4 /dev/sdb2 rw",
            "\n31 24 0:33 / /opt/container rw - overlay overlay rw,upperdir=/overlay/upper,workdir=/opt/work",
        ] {
            assert!(layout(&mounts(&format!("{INTERNAL}{entry}")).unwrap()).is_err());
        }
        let container = format!(
            "{INTERNAL}\n31 24 0:33 / /opt/container rw - overlay overlay rw,upperdir=/opt/upper,workdir=/opt/work"
        );
        assert!(layout(&mounts(&container).unwrap()).is_ok());
    }

    #[test]
    fn mount_escapes_do_not_accept_nul_invalid_octal_or_overflow() {
        assert_eq!(unescape(r"/mnt/a\040b\134c").unwrap(), "/mnt/a b\\c");
        for path in [r"/mnt/\000", r"/mnt/\777", r"/mnt/\abc", "/mnt/\\"] {
            assert!(unescape(path).is_err(), "{path}");
        }
    }

    #[test]
    fn clean_recovery_requires_reboot_and_preserved_extroot() {
        let mut options = Options::defaults();
        options.clean_overlay = true;
        assert!(options.validate().is_ok());
        options.reboot = false;
        assert!(options.validate().is_err());
        options.reboot = true;
        options.keep_current_extroot = false;
        assert!(options.validate().is_err());
    }

    #[test]
    fn old_options_remain_plain_configuration_migrations() {
        let mut options = serde_json::to_value(Options::defaults()).unwrap();
        options.as_object_mut().unwrap().remove("clean_overlay");
        let options: Options = serde_json::from_value(options).unwrap();
        assert!(!options.clean_overlay);
    }

    #[test]
    fn legacy_journals_keep_the_original_payload_location() {
        let mut value = serde_json::to_value(journal()).unwrap();
        value.as_object_mut().unwrap().remove("payload_on_overlay");
        let loaded: Journal = serde_json::from_value(value).unwrap();
        assert!(!loaded.payload_on_overlay);
    }

    #[test]
    fn removing_staging_keeps_immutable_payload_history() {
        let temporary = tempdir().unwrap();
        let payload = temporary.path().join("staging/payload");
        let history = temporary
            .path()
            .join("upper/etc/overlay-restore/jobs")
            .join(ID)
            .join("payload");
        atomic_write(&payload.join("root/data"), b"backup data", 0o600).unwrap();
        copy_tree(&payload, &history, true).unwrap();
        assert_eq!(
            fs::metadata(payload.join("root/data")).unwrap().ino(),
            fs::metadata(history.join("root/data")).unwrap().ino()
        );
        fs::remove_dir_all(temporary.path().join("staging")).unwrap();
        assert_eq!(fs::read(history.join("root/data")).unwrap(), b"backup data");
        assert_eq!(
            fs::metadata(history.join("root/data")).unwrap().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn firmware_changes_allow_discarding_but_device_changes_are_rejected() {
        let original = journal().overlay;
        let mut changed = original.clone();
        changed.firmware = "new-firmware".into();
        assert!(original.same_device(&changed));
        assert_ne!(original, changed);
        changed.device = "/dev/sda1".into();
        assert!(!original.same_device(&changed));
        changed = original.clone();
        changed.identity = "7:1".into();
        assert!(!original.same_device(&changed));
    }

    #[test]
    fn exchange_preserves_old_tree_external_data_and_is_idempotent() {
        let temporary = tempdir().unwrap();
        let overlay = temporary.path();
        let base = trees(overlay);
        let mut journal = journal();
        swap(overlay, &mut journal, false).unwrap();
        assert_eq!(
            fs::read(overlay.join("upper/root/restored")).unwrap(),
            b"restored"
        );
        assert!(!overlay.join("upper/root/old-program").exists());
        assert_eq!(
            fs::read(base.join("alternate-upper/root/old-program")).unwrap(),
            b"old"
        );
        let inode = fs::metadata(overlay.join("upper")).unwrap().ino();
        swap(overlay, &mut journal, false).unwrap();
        assert_eq!(fs::metadata(overlay.join("upper")).unwrap().ino(), inode);
        assert_eq!(
            fs::read(overlay.join("outside-data")).unwrap(),
            b"external data"
        );
    }

    #[test]
    fn interrupted_commit_is_detected_without_exchanging_again() {
        let temporary = tempdir().unwrap();
        let overlay = temporary.path();
        let base = trees(overlay);
        exchange(&overlay.join("work"), &base.join("alternate-work")).unwrap();
        exchange(&overlay.join("upper"), &base.join("alternate-upper")).unwrap();
        let inode = fs::metadata(overlay.join("upper")).unwrap().ino();
        let mut journal = journal();
        swap(overlay, &mut journal, false).unwrap();
        assert_eq!(journal.phase, "switched");
        assert_eq!(fs::metadata(overlay.join("upper")).unwrap().ino(), inode);
    }

    #[test]
    fn rollback_retains_restored_tree_and_does_not_requeue_cleaning() {
        let temporary = tempdir().unwrap();
        let overlay = temporary.path();
        let base = trees(overlay);
        let mut journal = journal();
        swap(overlay, &mut journal, false).unwrap();
        swap(overlay, &mut journal, true).unwrap();
        let state: Value = read_json(
            &overlay
                .join("upper/etc/overlay-restore/jobs")
                .join(ID)
                .join("state.json"),
        )
        .unwrap();
        assert_eq!(state["status"], "rolled_back");
        assert_eq!(state["clean_overlay"]["retained"], true);
        assert_eq!(
            fs::read(base.join("alternate-upper/root/restored")).unwrap(),
            b"restored"
        );
        assert_eq!(
            fs::read(overlay.join("upper/root/old-program")).unwrap(),
            b"old"
        );
        let inode = fs::metadata(overlay.join("upper")).unwrap().ino();
        swap(overlay, &mut journal, true).unwrap();
        assert_eq!(fs::metadata(overlay.join("upper")).unwrap().ino(), inode);
    }

    #[test]
    fn extroot_activation_and_rollback_restore_the_original_fstab_and_state() {
        let temporary = tempdir().unwrap();
        let origin = temporary.path().join("origin");
        let external = temporary.path().join("external");
        mkdir(&external, 0o755).unwrap();
        let base = trees(&external);
        mkdir(&origin.join("upper/etc/config"), 0o755).unwrap();
        let state_path = origin
            .join("upper/etc/overlay-restore/jobs")
            .join(ID)
            .join("state.json");
        save_json(
            &state_path,
            &json!({"id": ID, "status": "awaiting_clean_boot"}),
        )
        .unwrap();
        let original_fstab = "config mount 'data'\n option target '/mnt/data'\n";
        let mut journal = journal();
        journal.activation = Some(Activation {
            origin: journal.overlay.clone(),
            uuid: "abcd-1234".into(),
            original_fstab: original_fstab.into(),
            original_extroot_uuid: None,
        });
        // The old external tree has no record of this new restore task.
        fs::remove_file(
            external
                .join("upper/etc/overlay-restore/jobs")
                .join(ID)
                .join("state.json"),
        )
        .unwrap();
        swap(&external, &mut journal, false).unwrap();
        update_origin_files(&origin, &journal, false).unwrap();
        assert!(
            fs::read_to_string(origin.join("upper/etc/config/fstab"))
                .unwrap()
                .contains("uuid 'abcd-1234'")
        );
        update_origin_files(&origin, &journal, true).unwrap();
        assert_eq!(
            fs::read_to_string(origin.join("upper/etc/config/fstab")).unwrap(),
            original_fstab
        );
        let state: Value = read_json(&state_path).unwrap();
        assert_eq!(state["status"], "rolled_back");
        swap(&external, &mut journal, true).unwrap();
        assert_eq!(
            fs::read(base.join("alternate-upper/root/restored")).unwrap(),
            b"restored"
        );
        assert_eq!(
            fs::read(external.join("upper/root/old-program")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn aliased_store_and_trees_are_rejected_before_changes() {
        let temporary = tempdir().unwrap();
        let overlay = temporary.path();
        let base = trees(overlay);
        let outside = overlay.join("outside");
        mkdir(&outside, 0o755).unwrap();
        let original = base.join("alternate-work");
        fs::remove_dir(&original).unwrap();
        symlink(&outside, &original).unwrap();
        let old_inode = fs::metadata(overlay.join("work")).unwrap().ino();
        assert!(swap(overlay, &mut journal(), false).is_err());
        assert_eq!(fs::metadata(overlay.join("work")).unwrap().ino(), old_inode);
        for id in [
            "../upper",
            "/upper",
            "bad",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ] {
            assert!(directory(overlay, id).is_err());
        }
    }

    #[test]
    fn snapshot_mounts_through_disk_aliases_are_protected() {
        let prefix = format!("/{STORE}/{ID}");
        let mounts = format!(
            "{INTERNAL}\n30 24 7:0 / /mnt/disk rw - ext4 /dev/loop0 rw\n\
31 30 8:2 / /mnt/disk{prefix}/alternate-upper/root/data rw - ext4 /dev/sdb1 rw"
        );
        assert!(has_snapshot_mount(
            &super::mounts(&mounts).unwrap(),
            "7:0",
            ID
        ));
        let mounts = format!(
            "{INTERNAL}\n31 24 7:0 {prefix}/alternate-upper /mnt/old rw - ext4 /dev/loop0 rw"
        );
        assert!(has_snapshot_mount(
            &super::mounts(&mounts).unwrap(),
            "7:0",
            ID
        ));
        assert!(!has_snapshot_mount(
            &super::mounts(INTERNAL).unwrap(),
            "7:0",
            ID
        ));
    }

    #[test]
    fn destinations_cannot_escape_through_installed_symlinks() {
        let temporary = tempdir().unwrap();
        let upper = temporary.path().join("upper");
        mkdir(&upper, 0o755).unwrap();
        symlink(temporary.path(), upper.join("etc")).unwrap();
        for path in ["../outside", "/outside", "etc/config/network"] {
            assert!(new_destination(&upper, path).is_err());
        }
    }

    #[test]
    fn history_links_are_immutable_under_atomic_updates() {
        let temporary = tempdir().unwrap();
        let source = temporary.path().join("old");
        let copy = temporary.path().join("new");
        mkdir(&source, 0o700).unwrap();
        atomic_write(&source.join("state.json"), b"old", 0o600).unwrap();
        copy_tree(&source, &copy, true).unwrap();
        atomic_write(&copy.join("state.json"), b"new", 0o600).unwrap();
        assert_eq!(fs::read(source.join("state.json")).unwrap(), b"old");
        assert_eq!(fs::read(copy.join("state.json")).unwrap(), b"new");
    }

    #[test]
    fn preparation_reserve_releases_space_on_error() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("reserve");
        let failed = (|| -> Result<()> {
            let _reserve = Reserve::create(temporary.path())?;
            assert_eq!(fs::metadata(&path)?.len(), 8 * 1024 * 1024);
            bail!("Simulated failed package transaction");
        })();
        assert!(failed.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn retained_overlay_blocks_history_deletion_and_live_stage_is_private() {
        assert!(!Jobs::can_cleanup(
            &json!({"status":"complete","clean_overlay":{"retained":true}})
        ));
        assert!(Jobs::can_cleanup(
            &json!({"status":"rolled_back","clean_overlay":{"retained":false}})
        ));
        assert!(run_stage(Path::new("/tmp/not-the-private-manifest")).is_err());
        assert_eq!(std::mem::size_of::<LoopInfo>(), 232);
        assert_eq!(std::mem::size_of::<LoopConfig>(), 304);
    }
}
