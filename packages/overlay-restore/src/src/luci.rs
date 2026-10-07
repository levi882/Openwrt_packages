//! Keep the LuCI runtime and its installed core views in one APK transaction.
use crate::settings::Options;
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const CORE: [&str; 5] = [
    "luci-base",
    "luci-mod-status",
    "luci-mod-network",
    "luci-mod-system",
    "luci-mod-admin-full",
];

pub fn packages(root: &Path, options: &Options) -> Result<Vec<String>> {
    let mut installed = Vec::new();
    for prefix in ["", "rom"] {
        let database = root.join(prefix).join("lib/apk/db/installed");
        if database.is_file() {
            installed.extend(
                fs::read_to_string(&database)?
                    .lines()
                    .filter_map(|line| line.strip_prefix("P:").map(str::to_owned)),
            );
        }
        for name in CORE {
            if root
                .join(prefix)
                .join(format!("lib/apk/packages/{name}.list"))
                .is_file()
            {
                installed.push(name.to_owned());
            }
        }
    }
    Ok(CORE
        .into_iter()
        .filter(|name| {
            *name == "luci-base"
                || installed.iter().any(|package| package == name)
                || options
                    .install_packages
                    .iter()
                    .chain(&options.myfeed_packages)
                    .chain(&options.optional_packages)
                    .any(|package| package == name)
        })
        .map(str::to_owned)
        .collect())
}

pub fn verify_versions(output: &str, packages: &[String]) -> Result<String> {
    let mut versions = BTreeMap::new();
    for line in output.lines().filter(|line| line.contains("[installed]")) {
        let identity = line.split_whitespace().next().unwrap_or("");
        for package in packages {
            if let Some(version) = identity.strip_prefix(&format!("{package}-")) {
                versions.insert(package.as_str(), version);
            }
        }
    }
    let version = *versions
        .get("luci-base")
        .context("Unable to verify the installed LuCI runtime version")?;
    for package in packages {
        match versions.get(package.as_str()) {
            Some(installed) if *installed == version => (),
            Some(installed) => {
                bail!("LuCI core versions do not match: luci-base={version}, {package}={installed}")
            }
            None => bail!("LuCI core package is missing: {package}"),
        }
    }
    Ok(version.to_owned())
}
