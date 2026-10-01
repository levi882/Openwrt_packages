use crate::engine::Jobs;
use crate::settings::Options;
use crate::util::{atomic_write, quote, random_hex, save_json, tail};
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;

fn run(jobs: &Jobs, id: &str, warnings: &mut Vec<String>, arguments: &[&str]) -> Result<i32> {
    let (code, output) = jobs.run(arguments, Some(id), 30)?;
    if code != 0 {
        warnings.push(format!(
            "Command failed: {}: {}",
            arguments
                .iter()
                .take(3)
                .copied()
                .collect::<Vec<_>>()
                .join(" "),
            tail(&output, 500)
        ));
    }
    Ok(code)
}

fn get(jobs: &Jobs, key: &str) -> Result<String> {
    Ok(jobs
        .run(&["uci", "-q", "get", key], None, 120)?
        .1
        .trim()
        .to_owned())
}

fn service(
    jobs: &Jobs,
    id: &str,
    warnings: &mut Vec<String>,
    name: &str,
    action: &str,
) -> Result<i32> {
    let script = format!("/etc/init.d/{name}");
    if !jobs.root.join(script.trim_start_matches('/')).is_file() {
        return Ok(1);
    }
    run(jobs, id, warnings, &[&script, action])
}

pub fn repair_services(jobs: &Jobs, id: &str, options: &mut Options) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let mut themes = BTreeMap::new();
    let metadata = jobs.root.join("lib/apk/packages");
    if metadata.is_dir() {
        for entry in fs::read_dir(metadata)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(theme) = name
                .strip_prefix("luci-theme-")
                .and_then(|name| name.strip_suffix(".list"))
            else {
                continue;
            };
            if !theme.is_empty()
                && theme
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
                && jobs.root.join("www/luci-static").join(theme).is_dir()
            {
                let name = theme
                    .split('-')
                    .map(|part| {
                        let mut bytes = part.to_lowercase().into_bytes();
                        if let Some(first) = bytes.first_mut() {
                            first.make_ascii_uppercase();
                        }
                        String::from_utf8(bytes).unwrap_or_default()
                    })
                    .collect::<String>();
                themes.insert(name, format!("/luci-static/{theme}"));
            }
        }
    }
    if jobs.root.join("www/luci-static/bootstrap").is_dir() {
        themes.insert("Bootstrap".to_owned(), "/luci-static/bootstrap".to_owned());
    }
    if !themes.is_empty() {
        run(
            jobs,
            id,
            &mut warnings,
            &["uci", "-q", "set", "luci.themes=internal"],
        )?;
        for (name, path) in &themes {
            run(
                jobs,
                id,
                &mut warnings,
                &["uci", "-q", "set", &format!("luci.themes.{name}={path}")],
            )?;
        }
        let current_theme = get(jobs, "luci.main.mediaurlbase")?;
        if !themes.values().any(|path| path == &current_theme)
            && let Some(path) = themes.get("Bootstrap").or_else(|| themes.values().next())
        {
            run(
                jobs,
                id,
                &mut warnings,
                &[
                    "uci",
                    "-q",
                    "set",
                    &format!("luci.main.mediaurlbase={path}"),
                ],
            )?;
        }
        run(jobs, id, &mut warnings, &["uci", "-q", "commit", "luci"])?;
    }
    for (name, enabled) in [
        ("smartdns", "smartdns.@smartdns[0].enabled"),
        ("homebox", "homebox.main.enabled"),
    ] {
        if get(jobs, enabled)? == "1" {
            if name == "homebox" {
                service(jobs, id, &mut warnings, name, "enable")?;
            }
            if service(jobs, id, &mut warnings, name, "restart")? != 0 {
                warnings.push(format!("{name} could not be restarted"));
            }
        }
    }
    if get(jobs, "smartdns.@smartdns[0].ui")? == "1"
        && !jobs.root.join("usr/lib/smartdns_ui.so").is_file()
    {
        warnings.push("SmartDNS WebUI is enabled but smartdns_ui.so is missing".to_owned());
    }
    if options.iptv_enable {
        if !jobs.root.join("etc/init.d/iptv-refresh").is_file() {
            warnings.push("IPTV Refresh is unavailable; IPTV was not enabled".to_owned());
        } else {
            let repo = jobs
                .root
                .join(options.iptv_repo_root.trim_start_matches('/'));
            if !repo.is_dir() {
                warnings.push(format!(
                    "IPTV storage is not ready: {}",
                    options.iptv_repo_root
                ));
            } else {
                if options.iptv_refresh_token.is_empty() {
                    options.iptv_refresh_token = random_hex(32)?;
                    save_json(&jobs.path(id)?.join("options.json"), options)?;
                }
                atomic_write(
                    &jobs.root.join("etc/iptv-refresh/token"),
                    options.iptv_refresh_token.clone() + "\n",
                    0o600,
                )?;
                let provider = jobs.root.join("etc/iptv-refresh/provider.env");
                if provider.is_file() {
                    atomic_write(
                        &provider,
                        fs::read_to_string(&provider)?
                            .replace("/mnt/iptv/iptv-refresh", &options.iptv_repo_root),
                        0o600,
                    )?;
                }
                if jobs.root.join("etc/init.d/iptv-refresh-httpd").is_file() {
                    service(jobs, id, &mut warnings, "iptv-refresh-httpd", "stop")?;
                    service(jobs, id, &mut warnings, "iptv-refresh-httpd", "disable")?;
                }
                for (key, value) in [
                    ("enabled", "1".to_owned()),
                    ("repo_root", options.iptv_repo_root.clone()),
                    ("nginx_proxy", "1".to_owned()),
                    ("listen_host", options.iptv_refresh_host.clone()),
                    ("listen_port", options.iptv_refresh_port.to_string()),
                    ("iface", options.iptv_refresh_iface.clone()),
                ] {
                    run(
                        jobs,
                        id,
                        &mut warnings,
                        &[
                            "uci",
                            "-q",
                            "set",
                            &format!("iptv-refresh.main.{key}={value}"),
                        ],
                    )?;
                }
                for (key, values) in [
                    ("allow_ip", &options.iptv_refresh_allow_ips),
                    ("nginx_allow_ip", &options.iptv_nginx_allow_ips),
                ] {
                    jobs.run(
                        &["uci", "-q", "delete", &format!("iptv-refresh.main.{key}")],
                        None,
                        120,
                    )?;
                    for value in values {
                        run(
                            jobs,
                            id,
                            &mut warnings,
                            &[
                                "uci",
                                "-q",
                                "add_list",
                                &format!("iptv-refresh.main.{key}={value}"),
                            ],
                        )?;
                    }
                }
                run(
                    jobs,
                    id,
                    &mut warnings,
                    &["uci", "-q", "commit", "iptv-refresh"],
                )?;
                let url = options.iptv_public_url.trim_end_matches('/');
                let refresh = format!("{url}/iptv/refresh?iface={}", options.iptv_refresh_iface);
                let environment = format!(
                    "IPTV_REFRESH_IFACE={}\nIPTV_REFRESH_URL={}\nIPTV_REFRESH_HEALTHZ_URL={}\n",
                    quote(&options.iptv_refresh_iface),
                    quote(&refresh),
                    quote(&format!("{url}/iptv/healthz"))
                );
                atomic_write(
                    &repo.join("config/local/iptv_refresh.env"),
                    environment,
                    0o600,
                )?;
                atomic_write(
                    &repo.join("config/local/home_assistant_rest_command.yaml"),
                    format!(
                        "rest_command:\n  iptv_refresh:\n    url: {}\n    method: GET\n",
                        serde_json::to_string(&refresh)?
                    ),
                    0o600,
                )?;
                let ha = jobs
                    .root
                    .join(options.ha_config_root.trim_start_matches('/'));
                if ha.is_dir() {
                    let secret = ha.join("secrets.yaml");
                    let contents = if secret.is_file() {
                        fs::read_to_string(&secret)?
                    } else {
                        String::new()
                    };
                    let mut lines: Vec<String> = contents
                        .lines()
                        .filter(|line| !line.starts_with("iptv_refresh_url:"))
                        .map(str::to_owned)
                        .collect();
                    lines.push(format!(
                        "iptv_refresh_url: {}",
                        serde_json::to_string(&refresh)?
                    ));
                    atomic_write(&secret, lines.join("\n") + "\n", 0o600)?;
                }
                service(jobs, id, &mut warnings, "iptv-refresh", "enable")?;
                service(jobs, id, &mut warnings, "iptv-refresh", "restart")?;
            }
        }
    }
    for name in ["rpcd", "uhttpd", "uwsgi", "nginx"] {
        if jobs.root.join("etc/init.d").join(name).is_file() {
            service(jobs, id, &mut warnings, name, "restart")?;
        }
    }
    Ok(warnings)
}
