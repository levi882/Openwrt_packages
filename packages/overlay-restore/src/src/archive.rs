use crate::settings::Options;
use crate::util::{disk_free, hex, mkdir, open_regular, words};
use anyhow::{Context, Result, bail};
use flate2::read::MultiGzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub fn safe_name(name: &str) -> Result<String> {
    // OpenWrt uses Unix paths: backslashes are literal filename characters.
    if name.len() > 4096 || name.starts_with('/') || name.chars().any(char::is_control) {
        bail!("Unsafe archive path: {name:?}");
    }
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            ".." => bail!("Parent traversal in archive: {name:?}"),
            "" | "." => (),
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

struct LimitedReader<R> {
    stream: R,
    limit: u64,
    count: u64,
}
impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let amount = buffer
            .len()
            .min((self.limit.saturating_sub(self.count) + 1).min(usize::MAX as u64) as usize);
        let count = self.stream.read(&mut buffer[..amount])?;
        self.count += count as u64;
        if self.count > self.limit {
            return Err(std::io::Error::other(
                "Backup exceeds the expanded size limit",
            ));
        }
        Ok(count)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub archive_path: String,
    pub kind: String,
    pub size: u64,
    pub mode: u32,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Plan {
    pub layout: String,
    pub expanded_bytes: u64,
    pub selected_bytes: u64,
    pub files: Vec<Entry>,
    pub skipped: BTreeMap<String, usize>,
    pub metadata_available: bool,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub lan_ip: String,
    #[serde(default)]
    pub settings: Value,
    #[serde(default)]
    pub myfeed_repo: String,
}

fn pax_attributes(contents: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    let mut position = 0;
    while position < contents.len() {
        let space = contents[position..]
            .iter()
            .position(|byte| *byte == b' ')
            .context("Invalid PAX record")?
            + position;
        let length: usize = std::str::from_utf8(&contents[position..space])?
            .parse()
            .context("Invalid PAX length")?;
        let end = position.checked_add(length).context("Invalid PAX length")?;
        if end > contents.len() || end <= space + 1 || contents[end - 1] != b'\n' {
            bail!("Invalid PAX record length");
        }
        let record = std::str::from_utf8(&contents[space + 1..end - 1])?;
        let (key, value) = record.split_once('=').context("Invalid PAX attribute")?;
        result.insert(key.to_owned(), value.to_owned());
        position = end;
    }
    Ok(result)
}

fn check_link(name: &str, target: &str) -> Result<()> {
    if target.len() > 4096 || target.chars().any(char::is_control) {
        bail!("Invalid archive link: {name}");
    }
    if target.starts_with('/') {
        return Ok(());
    }
    let mut depth = name.split('/').count().saturating_sub(1);
    for part in target.split('/') {
        match part {
            ".." => {
                if depth == 0 {
                    bail!("Archive link escapes its root: {name}");
                }
                depth -= 1;
            }
            "" | "." => (),
            _ => depth += 1,
        }
    }
    Ok(())
}

fn visit(
    filename: &Path,
    limit: u64,
    mut visitor: impl FnMut(&Entry, &mut dyn Read) -> Result<()>,
) -> Result<u64> {
    let reader = LimitedReader {
        stream: MultiGzDecoder::new(open_regular(filename)?),
        limit,
        count: 0,
    };
    let mut archive = tar::Archive::new(reader);
    let mut seen = HashSet::new();
    let mut long_name = None;
    let mut long_link = None;
    let mut attributes = BTreeMap::new();
    let mut count = 0;
    let mut path_bytes = 0;
    // Raw iteration lets us bound extension headers before allocating them.
    // The normal tar iterator may buffer arbitrary GNU/PAX extension payloads.
    for item in archive.entries()?.raw(true) {
        let mut item = item.context("Unreadable tar entry")?;
        count += 1;
        if count > 100000 {
            bail!("Backup contains too many entries");
        }
        let size = item.size();
        if size > limit {
            bail!("Invalid archive member size");
        }
        let entry_type = item.header().entry_type().as_byte();
        if b"LKxg".contains(&entry_type) {
            if size > 65536 {
                bail!("Archive extension header exceeds its size limit");
            }
            let mut contents = Vec::new();
            item.read_to_end(&mut contents)?;
            match entry_type {
                b'L' | b'K' => {
                    if contents.last() == Some(&0) {
                        contents.pop();
                    }
                    let text = String::from_utf8(contents)
                        .context("Invalid archive extension encoding")?;
                    if entry_type == b'L' {
                        long_name = Some(text);
                    } else {
                        long_link = Some(text);
                    }
                }
                _ => {
                    let parsed = pax_attributes(&contents)?;
                    if entry_type == b'g' {
                        if parsed.keys().any(|key| {
                            ["path", "linkpath", "size"].contains(&key.as_str())
                                || key.starts_with("GNU.sparse")
                        }) {
                            bail!("Unsupported global archive path or size override");
                        }
                    } else {
                        attributes.extend(parsed);
                        let bytes: usize = attributes
                            .iter()
                            .map(|(key, value)| key.len() + value.len())
                            .sum();
                        if bytes > 65536 {
                            bail!("Combined archive extension metadata exceeds its size limit");
                        }
                    }
                }
            }
            continue;
        }
        if let Some(pax_size) = attributes.get("size")
            && pax_size.parse::<u64>()? != size
        {
            bail!("Conflicting archive member sizes");
        }
        let name = attributes
            .get("path")
            .cloned()
            .or(long_name.take())
            .unwrap_or(
                String::from_utf8(item.path_bytes().into_owned())
                    .context("Invalid archive path encoding")?,
            );
        let name = safe_name(&name)?;
        if !name.is_empty() && !seen.insert(name.clone()) {
            bail!("Duplicate archive path: {name}");
        }
        path_bytes += name.len() + 128;
        if path_bytes > 32 * 1024 * 1024 {
            bail!("Archive entry metadata exceeds its size limit");
        }
        let kind = match entry_type {
            b'0' | 0 => "file",
            b'5' => "directory",
            b'1' | b'2' => "link",
            _ => "special",
        };
        let sparse = attributes.keys().any(|key| key.starts_with("GNU.sparse"));
        if kind == "link" {
            let target = attributes
                .get("linkpath")
                .cloned()
                .or(long_link.take())
                .unwrap_or(String::from_utf8(
                    item.link_name_bytes().unwrap_or_default().into_owned(),
                )?);
            check_link(&name, &target)?;
        }
        attributes.clear();
        long_name = None;
        long_link = None;
        if name.is_empty() {
            continue;
        }
        let entry = Entry {
            archive_path: name,
            kind: if sparse { "special" } else { kind }.to_owned(),
            size,
            mode: item.header().mode()? & 0o777,
            path: String::new(),
            sha256: String::new(),
        };
        visitor(&entry, &mut item)?;
        // Consume the complete member even when the visitor skips it.
        std::io::copy(&mut item, &mut std::io::sink())?;
    }
    if long_name.is_some() || long_link.is_some() || !attributes.is_empty() {
        bail!("Archive ends with an incomplete extension header");
    }
    // Consume gzip trailers and concatenated streams as well as the tar headers.
    let mut reader = archive.into_inner();
    std::io::copy(&mut reader, &mut std::io::sink())
        .context("Unreadable backup or invalid gzip checksum")?;
    Ok(reader.count)
}

pub fn relative_name<'a>(name: &'a str, layout: &str) -> &'a str {
    match layout {
        "overlay" => name.strip_prefix("overlay/upper/").unwrap_or(""),
        "upper" => name.strip_prefix("upper/").unwrap_or(""),
        _ => name,
    }
}

fn metadata_owners(
    metadata: &BTreeMap<String, Vec<u8>>,
    layout: &str,
) -> Result<(BTreeMap<String, String>, bool)> {
    let mut owners = BTreeMap::new();
    let mut available = false;
    for (name, data) in metadata {
        let name = relative_name(name, layout);
        let text = String::from_utf8_lossy(data);
        if name == "lib/apk/db/installed" {
            let (mut package, mut directory) = ("", "");
            for line in text.lines() {
                if let Some(value) = line.strip_prefix("P:") {
                    package = value;
                    directory = "";
                } else if let Some(value) = line.strip_prefix("F:") {
                    directory = value;
                } else if let Some(value) = line.strip_prefix("R:")
                    && !package.is_empty()
                    && !directory.is_empty()
                {
                    let path =
                        safe_name(&format!("{}/{value}", directory.trim_start_matches('/')))?;
                    if !path.is_empty() {
                        owners.insert(path, package.to_owned());
                        available = true;
                    }
                }
            }
        } else if name.ends_with(".list")
            && (name.starts_with("lib/apk/packages/") || name.starts_with("usr/lib/opkg/info/"))
        {
            available = true;
            let package = Path::new(name)
                .file_name()
                .context("Invalid package list")?
                .to_string_lossy()
                .trim_end_matches(".list")
                .to_owned();
            for line in text.lines() {
                let path = safe_name(line.trim().trim_start_matches('/'))?;
                if !path.is_empty() {
                    owners.insert(path, package.clone());
                }
            }
        }
    }
    Ok((owners, available))
}

fn below(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn exclusion(
    path: &str,
    kind: &str,
    owners: &BTreeMap<String, String>,
    metadata: bool,
    options: &Options,
) -> &'static str {
    if path.is_empty() {
        return "archive layout";
    }
    if path.split('/').any(|part| part.starts_with(".wh.")) || kind == "special" {
        return "whiteouts and special files";
    }
    let prefixes = [
        "etc/apk",
        "lib/apk",
        "etc/opkg",
        "usr/lib/opkg",
        "etc/overlay-restore",
        "usr/libexec/overlay-restore",
        "usr/sbin/overlay-restore",
        "usr/libexec/rpcd",
        "usr/share/rpcd",
        "usr/share/luci",
        "usr/share/ucode/luci",
        "usr/lib/lua/luci",
        "www/luci-static",
        "www/cgi-bin",
        "etc/init.d",
        "etc/rc.d",
        "etc/uci-defaults",
        "lib/modules",
        "etc/modules.d",
        "etc/modules-boot.d",
        "dev",
        "proc",
        "sys",
        "tmp",
        "run",
        "mnt",
        "media",
        "boot",
        "rom",
        "overlay",
    ];
    let files = [
        "etc/config/overlay_restore",
        "etc/board.json",
        "etc/openwrt_release",
        "etc/openwrt_version",
        "etc/os-release",
        "etc/urandom.seed",
        "etc/resolv.conf",
        "etc/mtab",
    ];
    if files.contains(&path) || prefixes.iter().any(|prefix| below(path, prefix)) {
        return "current firmware and recovery tools";
    }
    if kind != "file" {
        return "links and directories";
    }
    if !options.restore_credentials
        && ([
            "etc/passwd",
            "etc/group",
            "etc/shadow",
            "etc/gshadow",
            "etc/config/dropbear",
            "etc/config/rpcd",
        ]
        .contains(&path)
            || ["etc/dropbear", "etc/ssh", "root/.ssh"]
                .iter()
                .any(|prefix| below(path, prefix)))
    {
        return "current credentials";
    }
    if options.keep_network
        && [
            "etc/config/network",
            "etc/config/dhcp",
            "etc/config/firewall",
        ]
        .contains(&path)
    {
        return "current network";
    }
    if [".apk-new", ".apk-old", "-opkg"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
        || [
            "etc/smartdns/smartdns.cache",
            "etc/smartdns/data/smartdns.cache",
        ]
        .contains(&path)
    {
        return "runtime cache";
    }
    if below(path, "etc") || below(path, "root") {
        return "";
    }
    if owners.contains_key(path) {
        return "packaged program files";
    }
    if ["www", "opt", "srv"]
        .iter()
        .any(|prefix| below(path, prefix))
        || metadata
            && ["usr/bin", "usr/sbin", "usr/share"]
                .iter()
                .any(|prefix| below(path, prefix))
    {
        return "";
    }
    "unclassified program files"
}

pub fn inspect_backup(filename: &Path, options: &Options, staging: &Path) -> Result<Plan> {
    let mut entries = Vec::new();
    let mut metadata = BTreeMap::new();
    let mut metadata_size = 0;
    let limit = options.max_expanded_mb * 1024 * 1024;
    let expanded = visit(filename, limit, |entry, input| {
        if entry.kind == "file"
            && (entry.archive_path.ends_with("lib/apk/db/installed")
                || entry.archive_path.ends_with(".list")
                    && (entry.archive_path.contains("lib/apk/packages/")
                        || entry.archive_path.contains("usr/lib/opkg/info/")))
        {
            metadata_size += entry.size;
            if metadata_size > 32 * 1024 * 1024 {
                bail!("Package metadata exceeds its size limit");
            }
            let mut contents = Vec::new();
            input.read_to_end(&mut contents)?;
            metadata.insert(entry.archive_path.clone(), contents);
        }
        entries.push(entry.clone());
        Ok(())
    })?;
    let layout = if entries
        .iter()
        .any(|entry| entry.archive_path.starts_with("overlay/upper/"))
    {
        "overlay"
    } else if entries
        .iter()
        .any(|entry| entry.archive_path.starts_with("upper/"))
    {
        "upper"
    } else if entries
        .iter()
        .any(|entry| entry.archive_path.starts_with("etc/config/"))
    {
        "sysupgrade"
    } else {
        bail!("Expected an overlay/upper, upper, or sysupgrade configuration backup");
    };
    let (owners, available) = metadata_owners(&metadata, layout)?;
    let mut selected = BTreeMap::new();
    let mut skipped = BTreeMap::new();
    for mut entry in entries {
        let path = relative_name(&entry.archive_path, layout).to_owned();
        let reason = exclusion(&path, &entry.kind, &owners, available, options);
        if !reason.is_empty() {
            *skipped.entry(reason.to_owned()).or_insert(0) += 1;
        } else {
            entry.path = path.clone();
            if selected.insert(path.clone(), entry).is_some() {
                bail!("Duplicate restored path: {path}");
            }
        }
    }
    if selected.is_empty() {
        bail!("Backup contains no restorable configuration or user files");
    }
    for path in selected.keys() {
        for parent in Path::new(path).ancestors().skip(1) {
            if selected.contains_key(parent.to_str().context("Invalid parent path")?) {
                bail!("File used as an archive directory: {path}");
            }
        }
    }
    let selected_bytes: u64 = selected.values().map(|entry| entry.size).sum();
    if selected_bytes + 8 * 1024 * 1024
        > disk_free(staging.parent().context("Staging path has no parent")?)?
    {
        bail!("Insufficient staging space for the selected files");
    }
    if staging.exists() {
        bail!("Staging directory already exists");
    }
    mkdir(staging, 0o700)?;
    visit(filename, limit, |entry, input| {
        let relative = relative_name(&entry.archive_path, layout);
        let Some(selected) = selected.get_mut(relative) else {
            return Ok(());
        };
        let path = staging.join(relative);
        mkdir(path.parent().context("Invalid staging path")?, 0o700)?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 65536];
        let mut written = 0;
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            written += count as u64;
        }
        if written != selected.size {
            bail!("Incomplete staged member: {relative}");
        }
        output.set_permissions(fs::Permissions::from_mode(selected.mode))?;
        output.sync_all()?;
        selected.sha256 = hex(&digest.finalize());
        Ok(())
    })?;
    Ok(Plan {
        layout: layout.to_owned(),
        expanded_bytes: expanded,
        selected_bytes,
        files: selected.into_values().collect(),
        skipped,
        metadata_available: available,
        warnings: if available || layout == "sysupgrade" {
            vec![]
        } else {
            vec![
                "Package file lists are absent; unclassified program files will be skipped."
                    .to_owned(),
            ]
        },
        sha256: String::new(),
        lan_ip: String::new(),
        settings: Value::Null,
        myfeed_repo: String::new(),
    })
}

fn sections(text: &str) -> Result<Vec<Vec<String>>> {
    let mut result = Vec::new();
    let mut section = Vec::new();
    for line in text.split_inclusive('\n') {
        if words(line)?.first().is_some_and(|word| word == "config") && !section.is_empty() {
            result.push(std::mem::take(&mut section));
        }
        section.push(line.to_owned());
    }
    if !section.is_empty() {
        result.push(section);
    }
    Ok(result)
}

fn extroot_section(section: &[String]) -> Result<bool> {
    for line in section {
        let words = words(line)?;
        if words.len() >= 3
            && words[0] == "option"
            && words[1] == "target"
            && ["/", "/overlay"].contains(&words[2].as_str())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn merge_fstab(backup: &str, current: &str) -> Result<String> {
    let mut result = Vec::new();
    for section in sections(backup)? {
        if !extroot_section(&section)? {
            result.push(section.concat().trim_end_matches('\n').to_owned());
        }
    }
    for section in sections(current)? {
        if extroot_section(&section)? {
            result.push(section.concat().trim_end_matches('\n').to_owned());
        }
    }
    Ok(result.join("\n") + "\n")
}

pub fn lan_address(text: &str) -> Result<String> {
    for section in sections(text)? {
        let mut is_lan = false;
        let mut address = String::new();
        for line in section {
            let words = words(&line)?;
            if words.len() >= 3
                && words[0] == "config"
                && words[1] == "interface"
                && words[2] == "lan"
            {
                is_lan = true;
            }
            if words.len() >= 3 && words[0] == "option" && words[1] == "ipaddr" {
                address = words[2].split('/').next().unwrap_or("").to_owned();
            }
        }
        if is_lan && !address.is_empty() {
            return Ok(address);
        }
    }
    Ok(String::new())
}

pub fn payload_path(directory: &Path, relative: &str) -> PathBuf {
    directory.join(relative)
}
