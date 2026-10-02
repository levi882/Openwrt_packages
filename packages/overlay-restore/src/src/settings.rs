use crate::util::words;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;

pub const PACKAGE_KEYS: &[&str] = &[
    "install_packages",
    "myfeed_packages",
    "optional_packages",
    "remove_packages",
];
pub const LIST_KEYS: &[&str] = &[
    "install_packages",
    "myfeed_packages",
    "optional_packages",
    "remove_packages",
    "iptv_refresh_allow_ips",
    "iptv_nginx_allow_ips",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Options {
    #[serde(default)]
    pub clean_overlay: bool,
    #[serde(default)]
    pub overlay_device: String,
    pub keep_current_extroot: bool,
    pub keep_network: bool,
    pub restore_credentials: bool,
    pub reboot: bool,
    pub iptv_enable: bool,
    pub max_upload_mb: u64,
    pub max_expanded_mb: u64,
    pub myfeed_repo: String,
    pub myfeed_key_url: String,
    pub iptv_repo_root: String,
    pub iptv_refresh_token: String,
    pub iptv_refresh_iface: String,
    pub iptv_refresh_host: String,
    pub iptv_refresh_port: u64,
    pub iptv_refresh_allow_ips: Vec<String>,
    pub iptv_nginx_allow_ips: Vec<String>,
    pub iptv_public_url: String,
    pub ha_config_root: String,
    pub install_packages: Vec<String>,
    pub myfeed_packages: Vec<String>,
    pub optional_packages: Vec<String>,
    pub remove_packages: Vec<String>,
}

pub fn defaults() -> Value {
    serde_json::from_str(include_str!("defaults.json"))
        .expect("Embedded defaults must be valid JSON")
}

impl Options {
    pub fn defaults() -> Self {
        serde_json::from_value(defaults()).expect("Embedded defaults must match Options")
    }

    pub fn validate(&self) -> Result<()> {
        validate(&serde_json::to_value(self)?)?;
        Ok(())
    }
}

fn valid_package(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+_.-".contains(&byte))
}

pub fn valid_url(text: &str, allow_http: bool) -> Result<()> {
    let url = url::Url::parse(text).context("Invalid URL")?;
    if !(url.scheme() == "https" || allow_http && url.scheme() == "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || text
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        bail!("Invalid URL");
    }
    Ok(())
}

pub fn validate(input: &Value) -> Result<Options> {
    let mut output = defaults();
    let input = input.as_object().context("Expected recovery settings")?;
    for (key, default) in defaults().as_object().expect("Defaults are an object") {
        let value = input.get(key).unwrap_or(default);
        output[key] = match default {
            Value::Bool(_) => {
                let text = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
                    .to_lowercase();
                match text.as_str() {
                    "true" | "1" => json!(true),
                    "false" | "0" => json!(false),
                    _ => bail!("Expected a boolean for {key}"),
                }
            }
            Value::Number(_) => {
                let number = value
                    .as_u64()
                    .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                    .with_context(|| format!("Expected an integer for {key}"))?;
                let bounds = if key.ends_with("port") {
                    (1, 65535)
                } else if key == "max_upload_mb" {
                    (1, 1024)
                } else {
                    (8, 4096)
                };
                if number < bounds.0 || number > bounds.1 {
                    bail!("Out of range: {key}");
                }
                json!(number)
            }
            Value::Array(_) => {
                let list = value
                    .as_array()
                    .with_context(|| format!("Expected a list for {key}"))?;
                if list.len() > 200 {
                    bail!("Too many values for {key}");
                }
                let mut seen = HashSet::new();
                let mut result = Vec::new();
                for item in list {
                    let item = item
                        .as_str()
                        .with_context(|| format!("Invalid list value for {key}"))?;
                    if PACKAGE_KEYS.contains(&key.as_str()) {
                        if !valid_package(item) {
                            bail!("Invalid package name: {item}");
                        }
                    } else if item.parse::<IpAddr>().is_err()
                        && item.parse::<ipnet::IpNet>().is_err()
                    {
                        bail!("Invalid allowed IP: {item}");
                    }
                    if seen.insert(item) {
                        result.push(item);
                    }
                }
                json!(result)
            }
            _ => {
                let text = value
                    .as_str()
                    .with_context(|| format!("Invalid value for {key}"))?;
                if text.len() > 4096 || text.chars().any(char::is_control) {
                    bail!("Invalid value for {key}");
                }
                json!(text)
            }
        };
    }
    let options: Options = serde_json::from_value(output)?;
    if options.clean_overlay && (!options.keep_current_extroot || !options.reboot) {
        bail!("Clean overlay recovery requires keeping the current extroot and automatic reboot");
    }
    if !options.overlay_device.is_empty() && !crate::extroot::valid_device(&options.overlay_device)
    {
        bail!("Select a block device from the overlay partition list");
    }
    valid_url(&options.myfeed_repo, false)?;
    valid_url(&options.myfeed_key_url, false)?;
    valid_url(&options.iptv_public_url, true)?;
    for path in [&options.iptv_repo_root, &options.ha_config_root] {
        if !path.starts_with("/mnt/") || path.split('/').any(|part| part == "..") {
            bail!("Storage paths must be under /mnt");
        }
    }
    let token = &options.iptv_refresh_token;
    if token.len() > 256
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._~-".contains(&byte))
    {
        bail!("IPTV token must use URL-safe characters");
    }
    let interface = &options.iptv_refresh_iface;
    if interface.is_empty()
        || interface.len() > 64
        || !interface
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
    {
        bail!("Invalid IPTV interface");
    }
    options
        .iptv_refresh_host
        .parse::<IpAddr>()
        .context("Invalid IPTV listen address")?;
    let installed: HashSet<_> = options
        .install_packages
        .iter()
        .chain(&options.myfeed_packages)
        .chain(&options.optional_packages)
        .collect();
    let protected = [
        "base-files",
        "busybox",
        "libc",
        "kernel",
        "procd",
        "rpcd",
        "uci",
        "uhttpd",
        "luci-base",
        "luci-theme-bootstrap",
        "overlay-restore",
        "luci-app-overlay-restore",
        "apk-mbedtls",
        "apk-openssl",
    ];
    for package in &options.remove_packages {
        if installed.contains(package) {
            bail!("Package selected for both removal and installation: {package}");
        }
        if protected.contains(&package.as_str())
            || !["luci-app-", "luci-i18n-", "luci-proto-", "luci-theme-"]
                .iter()
                .any(|prefix| package.starts_with(prefix) && package.len() > prefix.len())
        {
            bail!("Only nonessential LuCI packages can be removed");
        }
    }
    Ok(options)
}

pub fn from_uci(text: &str, environment: Option<&BTreeMap<String, String>>) -> Result<Options> {
    let mut settings = defaults();
    if !text.trim().is_empty() {
        for key in LIST_KEYS {
            settings[*key] = json!([]);
        }
    }
    for line in text.lines() {
        let Some((key, value)) = line
            .strip_prefix("overlay_restore.main.")
            .and_then(|line| line.split_once('='))
        else {
            continue;
        };
        if settings.get(key).is_none() {
            continue;
        }
        let parts = words(value)?;
        settings[key] = if LIST_KEYS.contains(&key) {
            json!(parts)
        } else {
            json!(if parts.len() == 1 {
                parts[0].as_str()
            } else {
                ""
            })
        };
    }
    if let Some(environment) = environment {
        for (variable, key) in [
            ("RESTORE_INSTALL_PACKAGES", "install_packages"),
            ("RESTORE_MYFEED_INSTALL_PACKAGES", "myfeed_packages"),
            (
                "RESTORE_MYFEED_OPTIONAL_INSTALL_PACKAGES",
                "optional_packages",
            ),
            (
                "RESTORE_REMOVE_PREINSTALLED_LUCI_PACKAGES",
                "remove_packages",
            ),
            ("RESTORE_MYFEED_REPO", "myfeed_repo"),
            ("RESTORE_MYFEED_KEY_URL", "myfeed_key_url"),
            ("RESTORE_IPTV_ENABLE", "iptv_enable"),
            ("RESTORE_IPTV_REPO_ROOT", "iptv_repo_root"),
            ("RESTORE_IPTV_REFRESH_TOKEN", "iptv_refresh_token"),
            ("RESTORE_IPTV_REFRESH_IFACE", "iptv_refresh_iface"),
            ("RESTORE_IPTV_REFRESH_HOST", "iptv_refresh_host"),
            ("RESTORE_IPTV_REFRESH_PORT", "iptv_refresh_port"),
            ("RESTORE_IPTV_REFRESH_ALLOW_IPS", "iptv_refresh_allow_ips"),
            ("RESTORE_IPTV_NGINX_ALLOW_IPS", "iptv_nginx_allow_ips"),
            ("RESTORE_HA_CONFIG_ROOT", "ha_config_root"),
        ] {
            if let Some(value) = environment.get(variable) {
                settings[key] = if LIST_KEYS.contains(&key) {
                    json!(words(value)?)
                } else {
                    json!(value)
                };
            }
        }
        if let Some(value) = environment.get("RESTORE_KEEP_EXTROOT") {
            settings["keep_current_extroot"] = json!(value != "1");
        }
    }
    validate(&settings)
}
