use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn timestamp() -> String {
    let seconds = now() as i64;
    let mut result = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: seconds and result point to correctly sized time_t/tm storage.
    if unsafe { libc::localtime_r(&seconds, result.as_mut_ptr()) }.is_null() {
        return seconds.to_string();
    }
    // SAFETY: a nonnull localtime_r result confirms that result was initialized.
    let time = unsafe { result.assume_init() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        time.tm_year + 1900,
        time.tm_mon + 1,
        time.tm_mday,
        time.tm_hour,
        time.tm_min,
        time.tm_sec
    )
}

pub fn random_hex(bytes: usize) -> Result<String> {
    let mut data = vec![0; bytes];
    File::open("/dev/urandom")?.read_exact(&mut data)?;
    Ok(hex(&data))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn mkdir(path: &Path, mode: u32) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(path)?;
    Ok(())
}

pub fn sync_parent(path: &Path) -> Result<()> {
    File::open(path.parent().context("Path has no parent")?)?.sync_all()?;
    Ok(())
}

fn replace(path: &Path, mode: u32, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    mkdir(path.parent().context("Path has no parent")?, 0o700)?;
    let temporary = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name()
            .context("Path has no name")?
            .to_string_lossy(),
        random_hex(6)?
    ));
    let result = (|| {
        let previous = match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    bail!(
                        "Replacement target is not a regular file: {}",
                        path.display()
                    );
                }
                Some(metadata)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        if let Some(metadata) = previous {
            // SAFETY: output owns a live file descriptor; the numeric IDs came from stat.
            if unsafe { libc::fchown(output.as_raw_fd(), metadata.uid(), metadata.gid()) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        output.set_permissions(fs::Permissions::from_mode(mode))?;
        write(&mut output)?;
        output.sync_all()?;
        fs::rename(&temporary, path)?;
        sync_parent(path)
    })();
    let _ = fs::remove_file(temporary);
    result
}

pub fn atomic_write(path: &Path, contents: impl AsRef<[u8]>, mode: u32) -> Result<()> {
    replace(path, mode, |output| {
        output.write_all(contents.as_ref())?;
        Ok(())
    })
}

pub fn open_regular(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        bail!("Expected a regular file: {}", path.display());
    }
    Ok(file)
}

pub fn atomic_copy(path: &Path, source: &Path, mode: u32) -> Result<()> {
    replace(path, mode, |output| {
        std::io::copy(&mut open_regular(source)?, output)?;
        Ok(())
    })
}

pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut contents = serde_json::to_vec_pretty(value)?;
    contents.push(b'\n');
    atomic_write(path, contents, 0o600)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut file = open_regular(path)?;
    if file.metadata()?.len() > 128 * 1024 * 1024 {
        bail!("Recovery record exceeds its size limit");
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("Invalid recovery record: {}", path.display()))
}

pub fn digest_file(path: &Path) -> Result<String> {
    let mut stream = open_regular(path)?;
    let mut digest = Sha256::new();
    let mut block = [0; 65536];
    loop {
        let count = stream.read(&mut block)?;
        if count == 0 {
            break;
        }
        digest.update(&block[..count]);
    }
    Ok(hex(&digest.finalize()))
}

pub fn disk_free(path: &Path) -> Result<u64> {
    let name = CString::new(path.as_os_str().as_bytes())?;
    let mut information = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: name is NUL terminated and information points to writable statvfs storage.
    if unsafe { libc::statvfs(name.as_ptr(), information.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: statvfs initialized information after returning success.
    let information = unsafe { information.assume_init() };
    Ok(information.f_bavail.saturating_mul(information.f_frsize))
}

pub fn require_space(path: &Path, required: u64, purpose: &str) -> Result<()> {
    let available = disk_free(path)?;
    if available < required {
        bail!(
            "Insufficient storage for {purpose}: {} has {} MiB available; {} MiB required",
            path.display(),
            available / (1024 * 1024),
            required.div_ceil(1024 * 1024),
        );
    }
    Ok(())
}

pub struct Lock(File);
impl Lock {
    pub fn acquire(path: &Path, blocking: bool) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .append(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        // SAFETY: file owns a live descriptor. flock releases its lock when file is dropped.
        if unsafe {
            libc::flock(
                file.as_raw_fd(),
                libc::LOCK_EX | if blocking { 0 } else { libc::LOCK_NB },
            )
        } != 0
        {
            bail!(
                "A recovery operation is already running: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        // SAFETY: self.0 is still open during this drop implementation.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub fn tail(text: &str, limit: usize) -> String {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

fn drain_tail(mut input: impl Read) -> String {
    let mut output = Vec::new();
    let mut block = [0; 8192];
    while let Ok(count) = input.read(&mut block) {
        if count == 0 {
            break;
        }
        output.extend_from_slice(&block[..count]);
        if output.len() > 65536 {
            output.drain(..output.len() - 65536);
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

pub trait Runner: Send + Sync {
    fn execute(&self, arguments: &[String], timeout: Duration) -> (i32, String);
}

pub struct SystemRunner;
impl Runner for SystemRunner {
    fn execute(&self, arguments: &[String], timeout: Duration) -> (i32, String) {
        let result = (|| -> Result<(i32, String)> {
            let mut child = Command::new(arguments.first().context("Empty command")?)
                .args(&arguments[1..])
                .env_clear()
                .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0)
                .spawn()?;
            let output = child.stdout.take().context("Missing stdout")?;
            let errors = child.stderr.take().context("Missing stderr")?;
            let (output_sender, output_receiver) = mpsc::channel();
            let (error_sender, error_receiver) = mpsc::channel();
            thread::spawn(move || {
                let _ = output_sender.send(drain_tail(output));
            });
            thread::spawn(move || {
                let _ = error_sender.send(drain_tail(errors));
            });
            let deadline = Instant::now() + timeout;
            let mut expired = false;
            let status = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if Instant::now() >= deadline {
                    expired = true;
                    // SAFETY: this child was started as leader of its own process group.
                    unsafe {
                        libc::kill(-(child.id() as i32), libc::SIGKILL);
                    }
                    break child.wait()?;
                }
                thread::sleep(Duration::from_millis(50));
            };
            let mut text = String::new();
            for receiver in [output_receiver, error_receiver] {
                let remaining = deadline
                    .saturating_duration_since(Instant::now())
                    .max(Duration::from_millis(50));
                match receiver.recv_timeout(remaining) {
                    Ok(output) => text.push_str(&output),
                    Err(_) => {
                        expired = true;
                        // SAFETY: descendants inherit the dedicated child process group.
                        unsafe {
                            libc::kill(-(child.id() as i32), libc::SIGKILL);
                        }
                        if let Ok(output) = receiver.recv_timeout(Duration::from_secs(1)) {
                            text.push_str(&output);
                        }
                    }
                }
            }
            if expired {
                text.push_str("\nCommand timed out");
            }
            Ok((
                if expired {
                    1
                } else {
                    status.code().unwrap_or(1)
                },
                text,
            ))
        })();
        result.unwrap_or_else(|error| (1, error.to_string()))
    }
}

pub fn words(text: &str) -> Result<Vec<String>> {
    let mut parser = shlex::Shlex::new(text);
    let result = parser.by_ref().collect();
    if parser.had_error {
        bail!("Invalid shell quoting in configuration");
    }
    Ok(result)
}

pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
