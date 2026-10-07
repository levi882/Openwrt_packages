//! Keep the firmware runtime while upgrading selected applications.
use crate::util::{Lock, atomic_write, mkdir, read_json, save_json, sync_parent};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const JOURNAL: &str = "etc/overlay-restore/firmware-world.json";

pub fn system_package(name: &str) -> bool {
    matches!(
        name,
        "base-files"
            | "busybox"
            | "libc"
            | "kernel"
            | "procd"
            | "rpcd"
            | "uci"
            | "ubus"
            | "ubusd"
            | "uhttpd"
            | "ucode"
            | "netifd"
            | "firewall4"
            | "fstools"
            | "logd"
            | "dropbear"
            | "dnsmasq"
            | "dnsmasq-full"
            | "nftables"
            | "nftables-json"
            | "ip-full"
            | "ip-tiny"
            | "block-mount"
            | "ca-bundle"
            | "urngd"
            | "uclient-fetch"
            | "jsonfilter"
            | "jshn"
            | "mtd"
            | "fwtool"
            | "odhcp6c"
            | "odhcpd"
            | "odhcpd-ipv6only"
            | "luci"
            | "luci-base"
            | "luci-light"
            | "luci-ssl"
            | "luci-ssl-openssl"
            | "luci-theme-bootstrap"
    ) || [
        "kmod-",
        "luci-mod-",
        "luci-lib-",
        "rpcd-",
        "ucode-",
        "procd-",
        "uhttpd-",
        "apk-",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn database(root: &Path) -> Result<String> {
    fs::read_to_string(root.join("lib/apk/db/installed"))
        .context("Unable to read the current firmware package database")
}

pub fn versions(contents: &str) -> Result<BTreeMap<String, String>> {
    let mut packages = BTreeMap::new();
    for record in contents.split("\n\n").filter(|r| !r.trim().is_empty()) {
        let name = record
            .lines()
            .find_map(|l| l.strip_prefix("P:"))
            .context("Package database entry is missing its name")?;
        let version = record
            .lines()
            .find_map(|l| l.strip_prefix("V:"))
            .context("Package database entry is missing its version")?;
        if name.is_empty()
            || version.is_empty()
            || name
                .bytes()
                .any(|b| !b.is_ascii_alphanumeric() && !b"+_.-".contains(&b))
            || version.chars().any(|c| c.is_whitespace() || c.is_control())
            || packages.insert(name.into(), version.into()).is_some()
        {
            bail!("Invalid or duplicate firmware package: {name}");
        }
    }
    Ok(packages)
}

pub fn protected(root: &Path) -> Result<BTreeMap<String, String>> {
    let current = versions(&database(root)?)?;
    let rom = root.join("rom/lib/apk/db/installed");
    let firmware = if rom.is_file() {
        versions(&fs::read_to_string(rom)?)?
    } else {
        current.clone()
    };
    Ok(current
        .into_iter()
        .filter(|(name, _)| {
            system_package(name) || name.starts_with("lib") && firmware.contains_key(name)
        })
        .collect())
}

pub fn constraint_name(line: &str) -> &str {
    line.trim_start_matches('!')
        .split(['@', '=', '<', '>', '~'])
        .next()
        .unwrap_or("")
}

/// Model the firmware, rather than an empty system or the old overlay, when
/// caching application dependencies for a future clean recovery.
pub fn seed_solver(source: &Path, target: &Path) -> Result<()> {
    let rom = source.join("rom/lib/apk/db/installed");
    let contents = if rom.is_file() {
        fs::read_to_string(rom)?
    } else {
        database(source)?
    };
    let all = versions(&contents)?;
    let pins = all
        .into_iter()
        .filter(|(name, _)| system_package(name) || name.starts_with("lib"))
        .collect::<BTreeMap<_, _>>();
    let records = contents
        .split("\n\n")
        .filter(|record| {
            record
                .lines()
                .find_map(|l| l.strip_prefix("P:"))
                .is_some_and(|name| pins.contains_key(name))
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    atomic_write(
        &target.join("lib/apk/db/installed"),
        records + "\n\n",
        0o600,
    )?;
    let world = pins
        .iter()
        .map(|(name, version)| format!("{name}={version}\n"))
        .collect::<String>();
    atomic_write(&target.join("etc/apk/world"), world, 0o600)
}

#[derive(Serialize, Deserialize)]
struct World {
    pins: BTreeMap<String, String>,
    original: Vec<String>,
}

/// Undo only our temporary constraints, retaining applications added by APK.
/// A durable journal also repairs a transaction interrupted by a reboot.
pub fn restore_world(root: &Path) -> Result<()> {
    let journal = root.join(JOURNAL);
    if !journal.exists() {
        return Ok(());
    }
    let saved: World = read_json(&journal)?;
    let path = root.join("etc/apk/world");
    let current = fs::read_to_string(&path)?;
    let mut lines = current
        .split_whitespace()
        .filter(|line| !saved.pins.contains_key(constraint_name(line)))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    lines.extend(saved.original);
    atomic_write(&path, lines.join("\n") + "\n", 0o644)?;
    fs::remove_file(&journal)?;
    sync_parent(&journal)
}

pub fn verify(root: &Path, expected: &BTreeMap<String, String>) -> Result<()> {
    let current = versions(&database(root)?)?;
    for (name, version) in expected {
        if current.get(name) != Some(version) {
            bail!("Firmware package changed or is missing: {name} (expected {version})");
        }
    }
    Ok(())
}

fn safe_plan(output: &str, pins: &BTreeMap<String, String>) -> Result<()> {
    for line in output.lines() {
        let Some((counter, action)) = line.trim().split_once(") ") else {
            continue;
        };
        if !counter.starts_with('(') || !counter.contains('/') {
            continue;
        }
        let mut words = action.split_whitespace();
        let operation = words.next().unwrap_or("");
        let name = words.next().unwrap_or("");
        if pins.contains_key(name) {
            bail!("Refusing to replace a firmware package: {operation} {name}");
        }
    }
    Ok(())
}

pub fn transaction(
    root: &Path,
    arguments: &[&str],
    mut run: impl FnMut(&[&str]) -> Result<(i32, String)>,
) -> Result<(i32, String)> {
    mkdir(&root.join("tmp"), 0o1777)?;
    let _lock = Lock::acquire(&root.join("tmp/overlay-restore-firmware.lock"), true)?;
    restore_world(root)?;
    let pins = protected(root)?;
    let path = root.join("etc/apk/world");
    let original = fs::read_to_string(&path).context("Unable to read APK world constraints")?;
    let saved = World {
        original: original
            .split_whitespace()
            .filter(|line| pins.contains_key(constraint_name(line)))
            .map(str::to_owned)
            .collect(),
        pins,
    };
    save_json(&root.join(JOURNAL), &saved)?;
    let mut constraints = original
        .split_whitespace()
        .filter(|line| !saved.pins.contains_key(constraint_name(line)))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    constraints.extend(
        saved
            .pins
            .iter()
            .map(|(name, version)| format!("{name}={version}")),
    );
    let result = (|| {
        atomic_write(&path, constraints.join("\n") + "\n", 0o644)?;
        let mut simulation = arguments.to_vec();
        simulation.push("--simulate");
        simulation.push("--no-logfile");
        let (code, output) = run(&simulation)?;
        if code != 0 {
            return Ok((
                code,
                format!(
                    "Application dependencies cannot be resolved while preserving firmware packages: {output}"
                ),
            ));
        }
        safe_plan(&output, &saved.pins)?;
        let result = run(arguments)?;
        verify(root, &saved.pins)?;
        Ok(result)
    })();
    let restored = restore_world(root);
    match (result, restored) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (_, Err(error)) => Err(error.context("Unable to restore temporary firmware constraints")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{TempDir, tempdir};

    fn fixture() -> TempDir {
        let root = tempdir().unwrap();
        atomic_write(&root.path().join("lib/apk/db/installed"),
            "P:kernel\nV:6.18.55-r1\n\nP:kmod-test\nV:6.18.55-r1\n\nP:luci-base\nV:26.100\n\nP:luci-mod-status\nV:26.099\n\nP:libc\nV:1.2.5\n\n", 0o644).unwrap();
        atomic_write(
            &root.path().join("etc/apk/world"),
            "kernel\nluci-base@custom\ncurl\n",
            0o644,
        )
        .unwrap();
        root
    }

    #[test]
    fn pins_preserve_individual_versions_and_restore_user_constraints() {
        let root = fixture();
        let mut calls = 0;
        transaction(root.path(), &["apk", "add", "--upgrade", "curl"], |args| {
            calls += 1;
            let world = fs::read_to_string(root.path().join("etc/apk/world"))?;
            assert!(world.contains("luci-base=26.100\n"));
            assert!(world.contains("luci-mod-status=26.099\n"));
            assert!(world.contains("kmod-test=6.18.55-r1\n"));
            if !args.contains(&"--simulate") {
                atomic_write(
                    &root.path().join("etc/apk/world"),
                    world + "nikki@myfeed\n",
                    0o644,
                )?;
            }
            Ok((0, "(1/1) Installing curl (new)".into()))
        })
        .unwrap();
        assert_eq!(calls, 2);
        let world = fs::read_to_string(root.path().join("etc/apk/world")).unwrap();
        assert!(world.contains("luci-base@custom\n"));
        assert!(world.contains("nikki@myfeed\n"));
        assert!(!world.contains("=26."));
        assert!(!root.path().join(JOURNAL).exists());
    }

    #[test]
    fn unsafe_simulation_never_executes_or_changes_firmware() {
        for operation in ["Upgrading", "Downgrading", "Purging", "Reinstalling"] {
            let root = fixture();
            let before = database(root.path()).unwrap();
            let original = fs::read_to_string(root.path().join("etc/apk/world")).unwrap();
            let mut calls = 0;
            let error = transaction(root.path(), &["apk", "add", "app"], |args| {
                calls += 1;
                assert!(args.contains(&"--simulate"));
                Ok((0, format!("( 1/12) {operation} luci-base (old -> new)")))
            })
            .unwrap_err();
            assert!(error.to_string().contains("Refusing to replace"));
            assert_eq!(calls, 1);
            assert_eq!(database(root.path()).unwrap(), before);
            let restored = fs::read_to_string(root.path().join("etc/apk/world")).unwrap();
            assert_eq!(
                restored
                    .split_whitespace()
                    .collect::<std::collections::BTreeSet<_>>(),
                original
                    .split_whitespace()
                    .collect::<std::collections::BTreeSet<_>>()
            );
        }
    }

    #[test]
    fn dependency_conflict_restores_world_without_installing() {
        let root = fixture();
        let mut calls = 0;
        let (code, output) = transaction(root.path(), &["apk", "add", "app"], |_| {
            calls += 1;
            Ok((1, "app requires luci-base>=26.200".into()))
        })
        .unwrap();
        assert_eq!(code, 1);
        assert_eq!(calls, 1);
        assert!(output.contains("preserving firmware"));
        assert!(
            !fs::read_to_string(root.path().join("etc/apk/world"))
                .unwrap()
                .contains("=26.")
        );
    }

    #[test]
    fn interrupted_world_pins_can_be_recovered_without_losing_applications() {
        let root = fixture();
        save_json(
            &root.path().join(JOURNAL),
            &World {
                pins: BTreeMap::from([("luci-base".into(), "26.100".into())]),
                original: vec!["luci-base@custom".into()],
            },
        )
        .unwrap();
        atomic_write(
            &root.path().join("etc/apk/world"),
            "kernel\nluci-base=26.100\nnikki@myfeed\n",
            0o644,
        )
        .unwrap();
        restore_world(root.path()).unwrap();
        let world = fs::read_to_string(root.path().join("etc/apk/world")).unwrap();
        assert!(world.contains("nikki@myfeed\n"));
        assert!(world.contains("luci-base@custom\n"));
        assert!(!world.contains("=26."));
    }

    #[test]
    fn application_libraries_outside_the_firmware_can_upgrade() {
        let root = fixture();
        atomic_write(
            &root.path().join("rom/lib/apk/db/installed"),
            database(root.path()).unwrap(),
            0o644,
        )
        .unwrap();
        let current =
            database(root.path()).unwrap() + "P:libcurl4\nV:8.20\n\nP:mihomo-meta\nV:1.19.31\n\n";
        atomic_write(&root.path().join("lib/apk/db/installed"), current, 0o644).unwrap();
        let pins = protected(root.path()).unwrap();
        assert!(!pins.contains_key("libcurl4"));
        assert!(!pins.contains_key("mihomo-meta"));
        assert!(pins.contains_key("libc"));
    }
}
