use crate::archive::{Plan, inspect_backup, lan_address, merge_fstab, safe_name};
use crate::settings::{Options, from_uci, valid_url};
use crate::util::{
    Lock, Runner, SystemRunner, atomic_copy, atomic_write, digest_file, disk_free, mkdir, now,
    open_regular, quote, random_hex, read_json, save_json, sync_parent, tail, timestamp,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const ACTIVE: &[&str] = &[
    "validating",
    "queued",
    "preparing_packages",
    "applying",
    "awaiting_reboot",
    "installing",
];

pub struct Jobs {
    pub root: PathBuf,
    pub directory: PathBuf,
    pub temporary: PathBuf,
    locks: PathBuf,
    runner: Arc<dyn Runner>,
    fixture: bool,
    pub boot_id: Option<String>,
    #[cfg(test)]
    pub fail_write: Option<String>,
    #[cfg(test)]
    pub fail_original: bool,
}

impl Jobs {
    pub fn new() -> Result<Self> {
        Self::create(Path::new("/"), Arc::new(SystemRunner), false)
    }

    fn create(root: &Path, runner: Arc<dyn Runner>, fixture: bool) -> Result<Self> {
        let root = root.canonicalize()?;
        let directory = root.join("etc/overlay-restore/jobs");
        let temporary = root.join("tmp/overlay-restore");
        let locks = root.join("var/lock");
        for directory in [&directory, &temporary, &locks] {
            mkdir(directory, 0o700)?;
        }
        Ok(Self {
            root,
            directory,
            temporary,
            locks,
            runner,
            fixture,
            boot_id: None,
            #[cfg(test)]
            fail_write: None,
            #[cfg(test)]
            fail_original: false,
        })
    }

    #[cfg(test)]
    pub fn fixture(root: &Path, runner: Arc<dyn Runner>) -> Result<Self> {
        Self::create(root, runner, true)
    }

    pub fn path(&self, id: &str) -> Result<PathBuf> {
        if id.len() != 32
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            bail!("Invalid task ID");
        }
        Ok(self.directory.join(id))
    }

    pub fn lock(&self, name: &str, blocking: bool) -> Result<Lock> {
        Lock::acquire(
            &self.locks.join(format!("overlay-restore-{name}.lock")),
            blocking,
        )
    }

    pub fn load(&self, id: &str) -> Result<Value> {
        read_json(&self.path(id)?.join("state.json")).context("Task not found or invalid")
    }

    pub fn save(&self, state: &mut Value) -> Result<()> {
        state["updated"] = json!(now());
        save_json(
            &self
                .path(state["id"].as_str().context("Task has no ID")?)?
                .join("state.json"),
            state,
        )
    }

    pub fn log(&self, id: &str, text: &str) -> Result<()> {
        let path = self.path(id)?.join("task.log");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        writeln!(file, "{} {}", timestamp(), text.trim_end())?;
        Ok(())
    }

    pub fn states(&self) -> Result<Vec<Value>> {
        let mut states = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if self.path(&name).is_ok() && entry.path().join("state.json").is_file() {
                states.push(self.load(&name)?);
            }
        }
        states.sort_by_key(|state| std::cmp::Reverse(state["created"].as_u64().unwrap_or(0)));
        Ok(states)
    }

    pub fn public(&self, id: &str, details: bool) -> Result<Value> {
        let mut state = self.load(id)?;
        let completed = state
            .as_object_mut()
            .context("Invalid task state")?
            .remove("completed_files");
        state["completed_count"] = json!(
            completed
                .as_ref()
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        );
        let task = self.path(id)?;
        if details && task.join("plan.json").is_file() {
            let plan: Plan = read_json(&task.join("plan.json"))?;
            let count = plan.files.len();
            let mut plan = serde_json::to_value(plan)?;
            plan["file_count"] = json!(count);
            plan["files"]
                .as_array_mut()
                .context("Invalid plan files")?
                .truncate(200);
            state["plan"] = plan;
        }
        if details && task.join("task.log").is_file() {
            let mut file = open_regular(&task.join("task.log"))?;
            let size = file.metadata()?.len();
            file.seek(SeekFrom::Start(size.saturating_sub(40000)))?;
            let mut contents = Vec::new();
            file.take(40000).read_to_end(&mut contents)?;
            state["log"] = json!(String::from_utf8_lossy(&contents));
        }
        Ok(state)
    }

    pub fn run(&self, arguments: &[&str], id: Option<&str>, timeout: u64) -> Result<(i32, String)> {
        if self.root != Path::new("/") && !self.fixture {
            bail!("System commands are disabled for fixture roots");
        }
        let owned: Vec<String> = arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect();
        let result = self.runner.execute(&owned, Duration::from_secs(timeout));
        if let Some(id) = id {
            self.log(
                id,
                &(arguments
                    .iter()
                    .map(|argument| quote(argument))
                    .collect::<Vec<_>>()
                    .join(" ")
                    + "\n"
                    + &tail(&result.1, 16000)),
            )?;
        }
        Ok(result)
    }

    pub fn read_settings(&self, environment: Option<&BTreeMap<String, String>>) -> Result<Options> {
        let (code, output) = self.run(&["uci", "-q", "show", "overlay_restore.main"], None, 120)?;
        if code != 0 {
            bail!("The overlay_restore UCI profile is missing or invalid");
        }
        from_uci(&output, environment)
    }

    pub fn start_worker(&self) -> Result<()> {
        let (code, output) = self.run(&["/etc/init.d/overlay-restore", "start"], None, 15)?;
        if code != 0 {
            bail!("Unable to start recovery worker: {output}");
        }
        Ok(())
    }

    pub fn current_boot(&self) -> Result<String> {
        if let Some(id) = &self.boot_id {
            return Ok(id.clone());
        }
        Ok(
            fs::read_to_string(self.root.join("proc/sys/kernel/random/boot_id"))?
                .trim()
                .to_owned(),
        )
    }

    pub fn require_idle(&self) -> Result<()> {
        if self
            .states()?
            .iter()
            .any(|state| ACTIVE.contains(&state["status"].as_str().unwrap_or("")))
        {
            bail!("Finish or retry the current recovery task first");
        }
        Ok(())
    }

    pub fn prepare(&self, filename: &Path, options: &Options) -> Result<Value> {
        options.validate()?;
        let _lock = self.lock("jobs", false)?;
        self.require_idle()?;
        let mut source = open_regular(filename)?;
        let size = source.metadata()?.len();
        if size == 0 {
            bail!("Backup must be a nonempty regular file");
        }
        if size > options.max_upload_mb * 1024 * 1024 {
            bail!("Backup exceeds the upload size limit");
        }
        let id = random_hex(16)?;
        let task = self.path(&id)?;
        let temporary = self.temporary.join(&id);
        mkdir(&task, 0o700)?;
        let result = (|| {
            mkdir(&temporary, 0o700)?;
            if disk_free(&temporary)? < size + 8 * 1024 * 1024 {
                bail!("Insufficient temporary space for the uploaded backup");
            }
            let mut target = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(temporary.join("backup.tar.gz"))?;
            let written = std::io::copy(
                &mut std::io::Read::by_ref(&mut source)
                    .take(options.max_upload_mb * 1024 * 1024 + 1),
                &mut target,
            )?;
            if written != size {
                bail!("Uploaded backup changed while being copied");
            }
            target.sync_all()?;
            let mut state = json!({"id": id, "status": "validating", "created": now(), "error": "", "warnings": [], "packages": {}, "completed_files": [], "boot_attempts": 0});
            save_json(&task.join("options.json"), options)?;
            self.save(&mut state)?;
            self.log(
                &id,
                "Backup accepted for validation; the running configuration has not been changed.",
            )?;
            Ok(state)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&temporary);
            let _ = fs::remove_dir_all(&task);
        }
        result
    }

    pub fn destination(&self, relative: &str) -> Result<PathBuf> {
        if relative.is_empty() || safe_name(relative)? != relative {
            bail!("Invalid destination path");
        }
        let path = self.root.join(relative);
        for parent in path.ancestors().skip(1) {
            if parent == self.root {
                break;
            }
            match fs::symlink_metadata(parent) {
                Ok(metadata) if metadata.is_symlink() => {
                    bail!("Destination parent is a symlink: {relative}")
                }
                Ok(metadata) if !metadata.is_dir() => {
                    bail!("Destination parent is not a directory: {relative}")
                }
                Ok(_) => (),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(error.into()),
            }
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_file() => {
                bail!("Destination is not a regular file: {relative}")
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        Ok(path)
    }

    pub fn validate(&self, id: &str) -> Result<()> {
        let task = self.path(id)?;
        let _lock = self.lock(&format!("validate-{id}"), true)?;
        let mut state = self.load(id)?;
        if state["status"] != "validating" {
            return Ok(());
        }
        let temporary = self.temporary.join(id);
        let inspected = (|| -> Result<Plan> {
            let options: Options = read_json(&task.join("options.json"))?;
            options.validate()?;
            let payload = temporary.join("payload");
            if payload.exists() {
                fs::remove_dir_all(&payload)?;
            }
            let backup = temporary.join("backup.tar.gz");
            let mut plan = inspect_backup(&backup, &options, &payload)?;
            plan.sha256 = digest_file(&backup)?;
            let fstab = payload.join("etc/config/fstab");
            if options.keep_current_extroot && fstab.is_file() {
                let current = self.root.join("etc/config/fstab");
                let current = if current.is_file() {
                    fs::read_to_string(current)?
                } else {
                    String::new()
                };
                let entry = plan
                    .files
                    .iter_mut()
                    .find(|entry| entry.path == "etc/config/fstab")
                    .context("Missing fstab entry")?;
                atomic_write(
                    &fstab,
                    merge_fstab(&fs::read_to_string(&fstab)?, &current)?,
                    entry.mode,
                )?;
                entry.size = fstab.metadata()?.len();
                entry.sha256 = digest_file(&fstab)?;
            }
            for entry in &plan.files {
                self.destination(&entry.path)?;
            }
            let network = payload.join("etc/config/network");
            if network.is_file() {
                plan.lan_ip = lan_address(&fs::read_to_string(network)?)?;
            }
            let settings = serde_json::to_value(&options)?;
            let mut public = serde_json::Map::new();
            for key in [
                "keep_current_extroot",
                "keep_network",
                "restore_credentials",
                "reboot",
                "iptv_enable",
                "install_packages",
                "myfeed_packages",
                "optional_packages",
                "remove_packages",
            ] {
                public.insert(key.to_owned(), settings[key].clone());
            }
            plan.settings = Value::Object(public);
            plan.myfeed_repo = self.feed_url(&options)?;
            plan.selected_bytes = plan.files.iter().map(|entry| entry.size).sum();
            Ok(plan)
        })();
        match inspected {
            Ok(plan) => {
                save_json(&task.join("plan.json"), &plan)?;
                state["warnings"] = json!(plan.warnings);
                state["status"] = json!("ready");
                self.log(
                    id,
                    &format!(
                        "Validation complete: {} selected files; waiting for confirmation.",
                        plan.files.len()
                    ),
                )?;
            }
            Err(error) => {
                state["status"] = json!("failed_validation");
                state["error"] = json!(error.to_string());
                self.log(id, &format!("Validation failed: {error}"))?;
                if temporary.exists() {
                    fs::remove_dir_all(temporary)?;
                }
            }
        }
        self.save(&mut state)
    }

    pub fn apply(&self, id: &str, confirmation: &str, reboot: Option<bool>) -> Result<Value> {
        if confirmation != id {
            bail!("The inspected task must be explicitly confirmed");
        }
        let _lock = self.lock("jobs", false)?;
        self.require_idle()?;
        let mut state = self.load(id)?;
        let task = self.path(id)?;
        if state["status"] != "ready" {
            bail!("Task is not ready to apply");
        }
        let payload = self.temporary.join(id).join("payload");
        if !payload.is_dir() {
            bail!("Preview expired after reboot; upload and inspect the backup again");
        }
        let plan: Plan = read_json(&task.join("plan.json"))?;
        let mut originals_bytes = 0;
        for entry in &plan.files {
            let destination = self.destination(&entry.path)?;
            if destination.exists() {
                originals_bytes += destination.metadata()?.len();
            }
            if digest_file(&payload.join(&entry.path))? != entry.sha256 {
                bail!("Staged file changed after inspection: {}", entry.path);
            }
        }
        if disk_free(&task)? < 2 * (plan.selected_bytes + originals_bytes) + 8 * 1024 * 1024 {
            bail!("Insufficient persistent storage for recovery and the original files");
        }
        let persistent = task.join("payload");
        if persistent.exists() {
            fs::remove_dir_all(&persistent)?;
        }
        let copied = (|| -> Result<()> {
            for entry in &plan.files {
                atomic_copy(
                    &persistent.join(&entry.path),
                    &payload.join(&entry.path),
                    entry.mode,
                )?;
            }
            Ok(())
        })();
        if let Err(error) = copied {
            let _ = fs::remove_dir_all(persistent);
            return Err(error);
        }
        if let Some(reboot) = reboot {
            let mut options: Options = read_json(&task.join("options.json"))?;
            options.reboot = reboot;
            save_json(&task.join("options.json"), &options)?;
        }
        state["status"] = json!("queued");
        state["apply_boot"] = json!(self.current_boot()?);
        self.save(&mut state)?;
        fs::remove_dir_all(self.temporary.join(id))?;
        Ok(state)
    }

    pub fn migrate(&self, id: &str) -> Result<()> {
        let mut state = self.load(id)?;
        let task = self.path(id)?;
        let mut plan: Plan = read_json(&task.join("plan.json"))?;
        let journal = task.join("originals.json");
        let mut originals: BTreeMap<String, Value> = if journal.exists() {
            read_json(&journal)?
        } else {
            BTreeMap::new()
        };
        state["apply_boot"] = json!(self.current_boot()?);
        if state["completed_files"]
            .as_array()
            .context("Invalid task progress")?
            .is_empty()
        {
            state["status"] = json!("preparing_packages");
            self.save(&mut state)?;
            if let Err(error) = crate::packages::prepare_network_packages(self, id) {
                state["status"] = json!("failed_prepare");
                state["error"] = json!(error.to_string());
                self.log(
                    id,
                    &format!(
                        "Network runtime preparation failed; configuration was not changed: {error}"
                    ),
                )?;
                return self.save(&mut state);
            }
        }
        state["status"] = json!("applying");
        self.save(&mut state)?;
        plan.files.sort_by_key(|entry| {
            (
                [
                    "etc/config/network",
                    "etc/config/firewall",
                    "etc/config/dhcp",
                    "etc/passwd",
                    "etc/shadow",
                ]
                .contains(&entry.path.as_str()),
                entry.path.clone(),
            )
        });
        let migrated = (|| -> Result<()> {
            for entry in &plan.files {
                let destination = self.destination(&entry.path)?;
                let completed = state["completed_files"]
                    .as_array_mut()
                    .context("Invalid progress")?;
                if completed
                    .iter()
                    .any(|path| path.as_str() == Some(&entry.path))
                {
                    if destination.is_file() && digest_file(&destination)? == entry.sha256 {
                        continue;
                    }
                    completed.retain(|path| path.as_str() != Some(&entry.path));
                }
                let source = task.join("payload").join(&entry.path);
                if digest_file(&source)? != entry.sha256 {
                    bail!("Persistent staged file changed: {}", entry.path);
                }
                if !originals.contains_key(&entry.path) {
                    let original = json!({"existed": destination.exists(), "mode": if destination.exists() { destination.metadata()?.mode() & 0o777 } else { 0 }});
                    if destination.exists() {
                        #[cfg(test)]
                        if self.fail_original {
                            bail!("Injected failure saving original");
                        }
                        atomic_copy(
                            &task.join("originals").join(&entry.path),
                            &destination,
                            original["mode"].as_u64().context("Invalid original mode")? as u32,
                        )?;
                    }
                    originals.insert(entry.path.clone(), original);
                    save_json(&journal, &originals)?;
                }
                mkdir(destination.parent().context("Invalid destination")?, 0o755)?;
                #[cfg(test)]
                if self.fail_write.as_ref() == Some(&entry.path) {
                    bail!("Injected disk write failure");
                }
                atomic_copy(&destination, &source, entry.mode)?;
                state["completed_files"]
                    .as_array_mut()
                    .context("Invalid progress")?
                    .push(json!(entry.path));
                self.save(&mut state)?;
            }
            Ok(())
        })();
        match migrated {
            Ok(()) => {
                state["status"] = json!("awaiting_reboot");
                self.log(id, "Configuration migration complete; package operations will resume after reboot.")?;
            }
            Err(error) => {
                let mut rollback_errors = Vec::new();
                for (relative, original) in originals.iter().rev() {
                    let restored = (|| -> Result<()> {
                        let destination = self.destination(relative)?;
                        if original["existed"] == true {
                            atomic_copy(
                                &destination,
                                &task.join("originals").join(relative),
                                original["mode"].as_u64().context("Invalid original mode")? as u32,
                            )?;
                        } else if destination.exists() {
                            fs::remove_file(&destination)?;
                            sync_parent(&destination)?;
                        }
                        Ok(())
                    })();
                    if let Err(error) = restored {
                        rollback_errors.push(format!("{relative}: {error}"));
                    }
                }
                state["status"] = json!("failed_apply");
                let message = if rollback_errors.is_empty() {
                    state["completed_files"] = json!([]);
                    format!("Migration failed; original files restored: {error}")
                } else {
                    format!(
                        "Migration failed: {error}; rollback incomplete: {}",
                        rollback_errors.join("; ")
                    )
                };
                state["error"] = json!(message);
                self.log(id, &message)?;
            }
        }
        self.save(&mut state)?;
        let options: Options = read_json(&task.join("options.json"))?;
        if state["status"] == "awaiting_reboot" && options.reboot {
            self.run(&["sync"], Some(id), 120)?;
            let (code, output) = self.run(&["/sbin/reboot"], Some(id), 10)?;
            if code != 0 {
                state["reboot_failed"] = json!(true);
                state["warnings"]
                    .as_array_mut()
                    .context("Invalid warnings")?
                    .push(json!(format!(
                        "Automatic reboot failed; reboot manually: {output}"
                    )));
                self.save(&mut state)?;
            }
        }
        Ok(())
    }

    pub fn feed_url(&self, options: &Options) -> Result<String> {
        let file = self.root.join("etc/apk/repositories.d/00-myfeed.list");
        if file.is_file() {
            for line in fs::read_to_string(file)?.lines() {
                let parts: Vec<_> = line.split_whitespace().collect();
                if parts.is_empty() || parts[0].starts_with('#') {
                    continue;
                }
                let url = if parts[0] == "@myfeed" && parts.len() == 2 {
                    parts[1]
                } else {
                    parts[0]
                };
                valid_url(url, false)?;
                return Ok(url.to_owned());
            }
        }
        Ok(options.myfeed_repo.clone())
    }

    pub fn retry(&self, id: &str) -> Result<Value> {
        let _lock = self.lock("jobs", false)?;
        self.require_idle()?;
        let mut state = self.load(id)?;
        let status = state["status"].as_str().unwrap_or("");
        if ![
            "failed_prepare",
            "failed_packages",
            "complete_with_warnings",
        ]
        .contains(&status)
        {
            bail!("Only package and service steps can be retried");
        }
        state["status"] = json!(if status == "failed_prepare" {
            "queued"
        } else {
            "installing"
        });
        state["error"] = json!("");
        state["boot_attempts"] = json!(0);
        state["warnings"] = json!([]);
        self.save(&mut state)?;
        Ok(state)
    }

    pub fn worker(&self, once: bool) -> Result<()> {
        let _lock = self.lock("worker", false)?;
        loop {
            for state in self.states()?.into_iter().rev() {
                let id = state["id"].as_str().context("Task has no ID")?;
                let status = state["status"].as_str().unwrap_or("");
                match status {
                    "validating" => self.validate(id)?,
                    "queued" | "preparing_packages" | "applying" => self.migrate(id)?,
                    "installing" => crate::packages::install_packages(self, id)?,
                    "awaiting_reboot"
                        if state["apply_boot"].as_str() != Some(self.current_boot()?.as_str()) =>
                    {
                        crate::packages::install_packages(self, id)?
                    }
                    "failed_packages"
                        if state["boot_attempts"].as_u64().unwrap_or(0) < 3
                            && state["last_boot"].as_str()
                                != Some(self.current_boot()?.as_str()) =>
                    {
                        crate::packages::install_packages(self, id)?
                    }
                    _ => (),
                }
            }
            if once {
                return Ok(());
            }
            thread::sleep(Duration::from_secs(2));
        }
    }
}
