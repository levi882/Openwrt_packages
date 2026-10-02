//! Select an inactive data partition without formatting it. Activation changes
//! only extroot configuration after the prepared external upper is committed.
use crate::clean::{Overlay, block_identity, discover, mounts, regular_dir};
use crate::engine::Jobs;
use crate::util::{mkdir, words};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Activation {
    pub origin: Overlay,
    pub uuid: String,
    pub original_fstab: String,
    #[serde(default)]
    pub original_extroot_uuid: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Partition {
    pub device: String,
    pub uuid: String,
    pub filesystem: String,
    pub label: String,
    pub mountpoint: String,
}

pub fn valid_device(device: &str) -> bool {
    device.strip_prefix("/dev/").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 64
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn block_records(text: &str) -> Result<Vec<Partition>> {
    let mut partitions = Vec::new();
    for line in text.lines() {
        let fields = words(line)?;
        let Some(device) = fields.first().and_then(|field| field.strip_suffix(':')) else {
            continue;
        };
        if !valid_device(device) || device.starts_with("/dev/loop") {
            continue;
        }
        let value = |name: &str| {
            fields
                .iter()
                .skip(1)
                .find_map(|field| field.strip_prefix(name))
                .unwrap_or("")
                .to_owned()
        };
        let filesystem = value("TYPE=");
        let uuid = value("UUID=");
        if !matches!(filesystem.as_str(), "ext4" | "f2fs")
            || uuid.is_empty()
            || !uuid
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
        {
            continue;
        }
        partitions.push(Partition {
            device: device.into(),
            uuid,
            filesystem,
            label: value("LABEL="),
            mountpoint: String::new(),
        });
    }
    Ok(partitions)
}

pub fn partitions(jobs: &Jobs) -> Result<Vec<Partition>> {
    let (code, output) = jobs.run(&["/sbin/block", "info"], None, 20)?;
    if code != 0 {
        bail!("Unable to list block filesystems; install block-mount");
    }
    let entries = mounts(&fs::read_to_string(jobs.root.join("proc/self/mountinfo"))?)?;
    let mut result = Vec::new();
    for mut partition in block_records(&output)? {
        let identity = block_identity(&jobs.root.join(partition.device.trim_start_matches('/')))?;
        let attached: Vec<_> = entries
            .iter()
            .filter(|entry| entry.identity == identity)
            .collect();
        if attached.iter().any(|entry| {
            ["/", "/rom", "/overlay", "/boot"].contains(&entry.point.as_str())
                || entry.point.starts_with("/overlay/")
                || entry.point.starts_with("/rom/")
                || entry.point.starts_with("/boot/")
                || entry.root != "/"
        }) {
            continue;
        }
        if let Some(entry) = attached.first() {
            partition.mountpoint.clone_from(&entry.point);
        }
        result.push(partition);
    }
    result.sort_by(|first, second| first.device.cmp(&second.device));
    Ok(result)
}

pub fn devices(jobs: &Jobs) -> Result<Value> {
    Ok(json!({"current": discover(&jobs.root).ok(), "devices": partitions(jobs)?}))
}

pub fn select(jobs: &Jobs, device: &str) -> Result<(Overlay, Option<Activation>)> {
    let origin = discover(&jobs.root)?;
    if device.is_empty() || device == origin.device {
        return Ok((origin, None));
    }
    if !valid_device(device) {
        bail!("Invalid extroot device");
    }
    let partition = partitions(jobs)?
        .into_iter()
        .find(|partition| partition.device == device)
        .context("Selected extroot partition is absent, unsupported or used by the system")?;
    let overlay = Overlay {
        identity: block_identity(&jobs.root.join(device.trim_start_matches('/')))?,
        device: device.into(),
        filesystem: partition.filesystem,
        mountpoint: "/overlay".into(),
        firmware: origin.firmware.clone(),
        loop_device: None,
    };
    let original_fstab = match fs::read_to_string(jobs.root.join("etc/config/fstab")) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    Ok((
        overlay,
        Some(Activation {
            origin,
            uuid: partition.uuid,
            original_fstab,
            original_extroot_uuid: None,
        }),
    ))
}

pub fn configured_fstab(text: &str, uuid: &str) -> Result<String> {
    if uuid.is_empty()
        || !uuid
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        bail!("Invalid extroot UUID");
    }
    let preserved = crate::archive::merge_fstab(text, "")?;
    Ok(format!(
        "{preserved}\nconfig mount 'overlay_restore_extroot'\n\toption uuid '{uuid}'\n\toption target '/overlay'\n\toption enabled '1'\n\toption options 'rw,noatime'\n"
    ))
}

pub(crate) fn matches_uuid(text: &str, device: &str, uuid: &str) -> Result<bool> {
    Ok(block_records(text)?
        .iter()
        .any(|entry| entry.device == device && entry.uuid == uuid))
}

pub(crate) fn verify_uuid(jobs: &Jobs, device: &str, uuid: &str) -> Result<()> {
    let (code, output) = jobs.run(&["/sbin/block", "info", device], None, 20)?;
    if code != 0 || !matches_uuid(&output, device, uuid)? {
        bail!("Selected external filesystem UUID changed");
    }
    Ok(())
}

pub(crate) fn boot_marker(root: &Path) -> Result<Option<Vec<u8>>> {
    let parent = root.join("etc");
    match fs::symlink_metadata(&parent) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => (),
    }
    regular_dir(&parent)?;
    let path = parent.join(".extroot-uuid");
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= 4096 => {
            let mut file = crate::util::open_regular(&path)?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(&mut file, 4097), &mut bytes)?;
            if bytes.len() > 4096 {
                bail!("Extroot boot marker grew during inspection");
            }
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        _ => bail!("Extroot boot marker must be a small regular file"),
    }
}

pub(crate) fn update_boot_marker(
    root: &Path,
    original: &Option<Vec<u8>>,
    rollback: bool,
) -> Result<()> {
    boot_marker(root)?;
    let path = root.join("etc/.extroot-uuid");
    if rollback && let Some(bytes) = original {
        crate::util::atomic_write(&path, bytes, 0o644)?;
    } else {
        match fs::remove_file(&path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) struct Target {
    pub path: PathBuf,
    mounted: bool,
}

impl Target {
    pub fn open(jobs: &Jobs, overlay: &Overlay, id: &str) -> Result<Self> {
        let entries = mounts(&fs::read_to_string(jobs.root.join("proc/self/mountinfo"))?)?;
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.identity == overlay.identity && entry.root == "/")
        {
            if entry.filesystem != overlay.filesystem
                || !entry.options.split(',').any(|part| part == "rw")
            {
                bail!("Selected filesystem must be mounted read-write with the expected type");
            }
            let path = jobs.root.join(entry.point.trim_start_matches('/'));
            regular_dir(&path)?;
            for mount in &entries {
                if mount.point.starts_with(&format!("{}/upper/", entry.point))
                    || mount.point.starts_with(&format!("{}/work/", entry.point))
                    || (entry.point != "/overlay"
                        && mount.filesystem == "overlay"
                        && (mount
                            .super_options
                            .contains(&format!("upperdir={}/upper", entry.point))
                            || mount
                                .super_options
                                .contains(&format!("workdir={}/work", entry.point))))
                {
                    bail!("Selected external upper/work is in use by another mount");
                }
            }
            return Ok(Self {
                path,
                mounted: false,
            });
        }
        if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("Invalid target mount task");
        }
        let path = jobs.root.join(format!("tmp/overlay-restore-target-{id}"));
        mkdir(&path, 0o700)?;
        regular_dir(&path)?;
        if fs::read_dir(&path)?.next().is_some() {
            bail!("Temporary extroot mountpoint is not empty");
        }
        let source = jobs.root.join(overlay.device.trim_start_matches('/'));
        if block_identity(&source)? != overlay.identity {
            bail!("Selected external device changed");
        }
        mount_device(&source, &path, &overlay.filesystem)?;
        Ok(Self {
            path,
            mounted: true,
        })
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        if self.mounted {
            if let Ok(path) = CString::new(self.path.as_os_str().as_bytes()) {
                // SAFETY: path is a valid C string referring to this owned mount.
                unsafe {
                    libc::umount(path.as_ptr());
                }
            }
            let _ = fs::remove_dir(&self.path);
        }
    }
}

pub(crate) fn mount_device(source: &Path, target: &Path, filesystem: &str) -> Result<()> {
    if !matches!(filesystem, "ext4" | "f2fs") {
        bail!("Unsupported extroot filesystem");
    }
    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    let filesystem = CString::new(filesystem)?;
    // SAFETY: all arguments are valid C strings; no options pointer is supplied.
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_labels_are_parsed_without_shell_execution() {
        let text = "/dev/sda1: UUID=\"1234-abcd\" LABEL=\"Backup Disk\" TYPE=\"ext4\"\n/dev/nvme0n1p3: UUID=\"abcd-1234\" TYPE=\"f2fs\"\n/dev/loop0: UUID=\"abcd\" TYPE=\"ext4\"\n/dev/sdb1: UUID=\"abcd\" TYPE=\"swap\"";
        let partitions = block_records(text).unwrap();
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0].label, "Backup Disk");
        assert_eq!(partitions[1].device, "/dev/nvme0n1p3");
        for device in [
            "/dev/../sda",
            "/dev/sda1;reboot",
            "/dev/sda1\n",
            "/dev/disk/by-uuid/abcd",
            "/tmp/sda1",
        ] {
            assert!(!valid_device(device));
        }
    }

    #[test]
    fn enabling_extroot_preserves_other_mounts_and_replaces_old_root_entries() {
        let old = "config mount 'old'\n option target '/overlay'\n option uuid 'old'\nconfig mount 'data'\n option target '/mnt/data'\n option uuid 'data'\n";
        let configured = configured_fstab(old, "abcd-1234").unwrap();
        assert!(configured.contains("/mnt/data"));
        assert!(!configured.contains("uuid 'old'"));
        assert!(configured.contains("target '/overlay'"));
        assert!(configured.contains("uuid 'abcd-1234'"));
        assert!(configured_fstab(old, "abcd'\nreboot").is_err());
    }

    #[test]
    fn stale_boot_uuid_is_retained_and_replaced_without_removing_other_data() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        mkdir(&root.join("etc"), 0o755).unwrap();
        fs::write(root.join("etc/.extroot-uuid"), b"old-firmware-uuid").unwrap();
        fs::write(root.join("etc/.extroot-default"), b"").unwrap();
        fs::write(root.join("etc/data"), b"keep").unwrap();
        let original = boot_marker(root).unwrap();
        update_boot_marker(root, &original, false).unwrap();
        assert!(!root.join("etc/.extroot-uuid").exists());
        assert!(root.join("etc/.extroot-default").exists());
        fs::write(root.join("etc/.extroot-uuid"), b"new-firmware-uuid").unwrap();
        update_boot_marker(root, &original, true).unwrap();
        assert_eq!(
            fs::read(root.join("etc/.extroot-uuid")).unwrap(),
            b"old-firmware-uuid"
        );
        assert_eq!(fs::read(root.join("etc/data")).unwrap(), b"keep");
    }
}
