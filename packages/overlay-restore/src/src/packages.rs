use crate::archive::Plan;
use crate::engine::Jobs;
use crate::settings::Options;
use crate::util::{atomic_write, read_json, save_json, sync_parent, tail};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize)]
struct Repository {
    existed: bool,
    original: String,
    url: String,
    packages: Vec<String>,
    tagged_before: Vec<String>,
}

fn tagged_name(line: &str) -> Option<&str> {
    let (name, suffix) = line.split_once("@myfeed")?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+_.-".contains(&byte))
        || !suffix.is_empty() && !suffix.starts_with(['<', '=', '>', '~'])
    {
        return None;
    }
    Some(name)
}

fn retain_clean_feed_tag(jobs: &Jobs, id: &str) -> Result<()> {
    let state = jobs.load(id)?;
    if state["clean_overlay"]["phase"] != "switched"
        || !fs::read_to_string(jobs.root.join("etc/overlay-restore/clean-id"))
            .is_ok_and(|marker| marker == id)
    {
        return Ok(());
    }
    let world = jobs.root.join("etc/apk/world");
    if !world.is_file()
        || !fs::read_to_string(world)?
            .lines()
            .any(|line| tagged_name(line).is_some())
    {
        return Ok(());
    }
    let repo = jobs.root.join("etc/apk/repositories.d/00-myfeed.list");
    let mut contents = if repo.is_file() {
        fs::read_to_string(&repo)?
    } else {
        String::new()
    };
    if contents
        .lines()
        .any(|line| line.split_whitespace().next() == Some("@myfeed"))
    {
        return Ok(());
    }
    let plan: Plan = read_json(&jobs.path(id)?.join("plan.json"))?;
    if contents.is_empty() {
        contents = plan.myfeed_repo.clone();
    }
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(&format!("@myfeed {}\n", plan.myfeed_repo));
    atomic_write(&repo, contents, 0o644)?;
    jobs.log(
        id,
        "Retained the myfeed repository tag required by clean bootstrap packages.",
    )
}

pub fn restore_repository(jobs: &Jobs, id: &str) -> Result<()> {
    let record_file = jobs.path(id)?.join("repository.json");
    if !record_file.exists() {
        return retain_clean_feed_tag(jobs, id);
    }
    let record: Repository = read_json(&record_file)?;
    let contents = if record.existed {
        record.original.clone()
    } else {
        record.url.clone() + "\n"
    };
    let repo = jobs.root.join("etc/apk/repositories.d/00-myfeed.list");
    atomic_write(&repo, contents, 0o644)?;
    let world = jobs.root.join("etc/apk/world");
    if world.is_file() {
        let lines: Vec<_> = fs::read_to_string(&world)?
            .lines()
            .map(|line| {
                if tagged_name(line).is_some_and(|name| {
                    record.packages.iter().any(|package| package == name)
                        && !record.tagged_before.iter().any(|package| package == name)
                }) {
                    line.replacen("@myfeed", "", 1)
                } else {
                    line.to_owned()
                }
            })
            .collect();
        atomic_write(&world, lines.join("\n") + "\n", 0o644)?;
    }
    fs::remove_file(&record_file)?;
    sync_parent(&record_file)?;
    retain_clean_feed_tag(jobs, id)?;
    jobs.log(
        id,
        "Restored repository configuration and removed temporary package tags.",
    )
}

pub fn tag_repository(jobs: &Jobs, id: &str, options: &Options) -> Result<()> {
    let task = jobs.path(id)?;
    let plan: Plan = read_json(&task.join("plan.json"))?;
    let url = plan.myfeed_repo;
    if jobs.feed_url(options)? != url {
        bail!("The myfeed URL changed after inspection; inspect a new task");
    }
    let repo = jobs.root.join("etc/apk/repositories.d/00-myfeed.list");
    let original = if repo.is_file() {
        fs::read_to_string(&repo)?
    } else {
        String::new()
    };
    let world = jobs.root.join("etc/apk/world");
    let tagged_before = if world.is_file() {
        fs::read_to_string(world)?
            .lines()
            .filter_map(|line| tagged_name(line).map(str::to_owned))
            .collect()
    } else {
        vec![]
    };
    let record = Repository {
        existed: repo.exists(),
        original: original.clone(),
        url: url.clone(),
        packages: options
            .myfeed_packages
            .iter()
            .chain(&options.optional_packages)
            .cloned()
            .collect(),
        tagged_before,
    };
    save_json(&task.join("repository.json"), &record)?;
    let mut lines: Vec<String> = original.lines().map(str::to_owned).collect();
    if let Some(line) = lines
        .iter_mut()
        .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
    {
        *line = format!("@myfeed {url}");
    } else {
        lines.push(format!("@myfeed {url}"));
    }
    atomic_write(&repo, lines.join("\n") + "\n", 0o644)?;
    let key = jobs.root.join("etc/apk/keys/myfeed.pem");
    if !key.is_file() {
        if crate::local_feed::trusted_key(jobs, &key).is_ok() {
            return Ok(());
        }
        if options.uses_local_feed() {
            bail!(
                "The trusted myfeed key is missing; local recovery cannot download a replacement"
            );
        }
        let downloaded = task.join("myfeed.pem");
        let filename = downloaded.to_str().context("Invalid key download path")?;
        let (code, _) = jobs.run(
            &[
                "uclient-fetch",
                "-T",
                "30",
                "-O",
                filename,
                &options.myfeed_key_url,
            ],
            Some(id),
            45,
        )?;
        if code != 0
            || !downloaded.is_file()
            || !(32..=16384).contains(&downloaded.metadata()?.len())
        {
            bail!("Unable to download the myfeed verification key");
        }
        let contents = fs::read(&downloaded)?;
        if !contents.windows(10).any(|window| window == b"PUBLIC KEY") {
            bail!("The myfeed key is not a PEM public key");
        }
        atomic_write(&key, contents, 0o644)?;
    }
    Ok(())
}

pub fn prepare_network_packages(jobs: &Jobs, id: &str) -> Result<()> {
    let options: Options = read_json(&jobs.path(id)?.join("options.json"))?;
    options.validate()?;
    restore_repository(jobs, id)?;
    if crate::local_feed::task(jobs, id, true)?.is_some() {
        jobs.log(id, "Using the prepared local feed for DNS/proxy packages; repository network access is disabled.")?;
    }
    let mut required = Vec::new();
    for name in ["smartdns", "nikki"] {
        if options
            .myfeed_packages
            .iter()
            .any(|package| package == name)
            && crate::local_feed::run(jobs, id, &["apk", "--wait", "30", "info", "-e", name], 40)?.0
                != 0
        {
            required.push(name);
        }
    }
    if required.is_empty() {
        return Ok(());
    }
    let result = (|| -> Result<()> {
        tag_repository(jobs, id, &options)?;
        if crate::local_feed::run(jobs, id, &["apk", "--wait", "30", "update"], 120)?.0 != 0 {
            bail!("Repositories are unavailable for preparing DNS/proxy packages");
        }
        for package in required {
            if crate::local_feed::run(
                jobs,
                id,
                &["apk", "--wait", "30", "add", &format!("{package}@myfeed")],
                180,
            )?
            .0 != 0
                || crate::local_feed::run(
                    jobs,
                    id,
                    &["apk", "--wait", "30", "info", "-e", package],
                    40,
                )?
                .0 != 0
            {
                bail!("Unable to prepare network runtime: {package}");
            }
        }
        jobs.log(
            id,
            "Current DNS/proxy package versions are available before configuration migration.",
        )
    })();
    let restored = restore_repository(jobs, id);
    result.and(restored)
}

fn execute(
    jobs: &Jobs,
    id: &str,
    deadline: Instant,
    arguments: &[&str],
    timeout: u64,
) -> Result<(i32, String)> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok((1, "Package operation time budget exhausted".to_owned()));
    }
    crate::local_feed::run(jobs, id, arguments, timeout.min(remaining.as_secs().max(1)))
}

fn installed(jobs: &Jobs, id: &str, deadline: Instant, package: &str) -> Result<bool> {
    let (code, output) = execute(
        jobs,
        id,
        deadline,
        &["apk", "--wait", "30", "info", "-e", package],
        40,
    )?;
    if code == 0 {
        return Ok(true);
    }
    // `info -e` returns one without an error for an absent package. Do not
    // confuse a timed-out or failed probe with a successful removal.
    if code == 1
        && output
            .lines()
            .all(|line| line.trim().is_empty() || line.starts_with("WARNING: "))
    {
        return Ok(false);
    }
    bail!("Unable to check installed package {package} (exit {code}): {output}")
}

fn record(
    jobs: &Jobs,
    state: &mut Value,
    package: &str,
    status: &str,
    operation: &str,
    required: bool,
    message: &str,
) -> Result<()> {
    state["packages"][package] = json!({"status": status, "operation": operation, "required": required, "message": tail(message, 2000)});
    jobs.save(state)
}

pub fn install_packages(jobs: &Jobs, id: &str) -> Result<()> {
    crate::clean::finalize(jobs, id)?;
    let mut state = jobs.load(id)?;
    let mut options: Options = read_json(&jobs.path(id)?.join("options.json"))?;
    options.validate()?;
    state["status"] = json!("installing");
    state["error"] = json!("");
    state["last_boot"] = json!(jobs.current_boot()?);
    state["boot_attempts"] = json!(state["boot_attempts"].as_u64().unwrap_or(0) + 1);
    jobs.save(&mut state)?;
    let deadline = Instant::now() + Duration::from_secs(900);
    let (mut failures, mut warnings) = (Vec::new(), Vec::new());
    let result = (|| -> Result<()> {
        restore_repository(jobs, id)?;
        if crate::local_feed::task(jobs, id, true)?.is_some() {
            jobs.log(id, "Restoring software from the prepared local feed; repository network access is disabled. Normal installs and updates retain the online feeds.")?;
        }
        if !options.myfeed_packages.is_empty() || !options.optional_packages.is_empty() {
            tag_repository(jobs, id, &options)?;
        }
        let mut updated = false;
        for attempt in 0..3 {
            if execute(jobs, id, deadline, &["apk", "--wait", "30", "update"], 120)?.0 == 0 {
                updated = true;
                break;
            }
            if attempt < 2 && Instant::now() + Duration::from_secs(5) < deadline {
                thread::sleep(Duration::from_secs(5));
            }
        }
        if !updated {
            bail!("Package repositories are unavailable; the restored configuration is retained");
        }
        let mut removals = Vec::new();
        for package in &options.remove_packages {
            if installed(jobs, id, deadline, package)? {
                record(jobs, &mut state, package, "running", "remove", true, "")?;
                removals.push(package.as_str());
            } else {
                record(jobs, &mut state, package, "removed", "remove", true, "")?;
            }
        }
        if !removals.is_empty() {
            // Resolve the selected applications, translations and themes together.
            // Separate transactions can retain an application until its selected
            // translation is removed, leaving an obsolete failure in the task.
            let mut arguments = vec!["apk", "--wait", "30", "del"];
            arguments.extend(&removals);
            let (code, output) = execute(jobs, id, deadline, &arguments, 120)?;
            for package in removals {
                if Instant::now() >= deadline || installed(jobs, id, deadline, package)? {
                    let message = if output.trim().is_empty() {
                        format!("Package remains installed after removal (exit {code})")
                    } else {
                        output.clone()
                    };
                    record(
                        jobs, &mut state, package, "failed", "remove", true, &message,
                    )?;
                    failures.push(package.to_owned());
                } else {
                    record(jobs, &mut state, package, "removed", "remove", true, "")?;
                }
            }
        }
        let mut missing_installations = HashSet::new();
        for (packages, tagged, required) in [
            (&options.install_packages, false, true),
            (&options.myfeed_packages, true, true),
            (&options.optional_packages, true, false),
        ] {
            for package in packages {
                if crate::luci::CORE.contains(&package.as_str()) {
                    continue;
                }
                let present_before = installed(jobs, id, deadline, package)?;
                if present_before && state["packages"][package]["status"] == "installed" {
                    record(
                        jobs,
                        &mut state,
                        package,
                        "installed",
                        "install",
                        required,
                        "",
                    )?;
                    continue;
                }
                if !present_before {
                    missing_installations.insert(package.as_str());
                }
                record(
                    jobs, &mut state, package, "running", "install", required, "",
                )?;
                let argument = if tagged {
                    format!("{package}@myfeed")
                } else {
                    package.clone()
                };
                let (code, output) = execute(
                    jobs,
                    id,
                    deadline,
                    &["apk", "--wait", "30", "add", "--upgrade", &argument],
                    120,
                )?;
                if code != 0 || !installed(jobs, id, deadline, package)? {
                    record(
                        jobs, &mut state, package, "failed", "install", required, &output,
                    )?;
                    if required {
                        failures.push(package.clone());
                    } else {
                        warnings.push(format!("Optional package unavailable: {package}"));
                    }
                } else {
                    record(
                        jobs,
                        &mut state,
                        package,
                        "installed",
                        "install",
                        required,
                        "",
                    )?;
                }
            }
        }
        // Application dependencies may upgrade luci-base without upgrading the
        // already-installed status/network/system views. Resolve the whole core
        // last, also on retry, so later application installs cannot split it.
        let luci_packages = crate::luci::packages(&jobs.root, &options)?;
        for package in &luci_packages {
            record(jobs, &mut state, package, "running", "install", true, "")?;
        }
        let mut arguments = vec!["apk", "--wait", "30", "add", "--upgrade", "--latest"];
        arguments.extend(luci_packages.iter().map(String::as_str));
        let (code, output) = execute(jobs, id, deadline, &arguments, 180)?;
        let synchronized = (|| -> Result<String> {
            if code != 0 {
                bail!("Unable to upgrade the LuCI core together: {output}");
            }
            let mut verification = vec!["apk", "list", "--installed"];
            verification.extend(luci_packages.iter().map(String::as_str));
            let (code, output) = execute(jobs, id, deadline, &verification, 60)?;
            if code != 0 {
                bail!("Unable to verify the installed LuCI core: {output}");
            }
            crate::luci::verify_versions(&output, &luci_packages)
        })();
        match synchronized {
            Ok(version) => {
                for package in &luci_packages {
                    record(jobs, &mut state, package, "installed", "install", true, "")?;
                }
                jobs.log(
                    id,
                    &format!("LuCI core upgraded and verified together: {version}"),
                )?;
            }
            Err(error) => {
                for package in &luci_packages {
                    record(
                        jobs,
                        &mut state,
                        package,
                        "failed",
                        "install",
                        true,
                        &error.to_string(),
                    )?;
                }
                bail!("{error}");
            }
        }
        // Later dependency transactions can finish an earlier failed install,
        // or install a package that an earlier removal had just removed.
        // Report the final package state rather than an intermediate result.
        for (packages, operation, required) in [
            (&options.remove_packages, "remove", true),
            (&options.install_packages, "install", true),
            (&options.myfeed_packages, "install", true),
            (&options.optional_packages, "install", false),
        ] {
            for package in packages {
                if Instant::now() >= deadline {
                    break;
                }
                if operation == "install"
                    && (state["packages"][package]["status"] != "failed"
                        || !missing_installations.contains(package.as_str()))
                {
                    continue;
                }
                let present = installed(jobs, id, deadline, package)?;
                if present == (operation == "install") {
                    record(
                        jobs,
                        &mut state,
                        package,
                        if present { "installed" } else { "removed" },
                        operation,
                        required,
                        "",
                    )?;
                    failures.retain(|failed| failed != package);
                    warnings.retain(|warning| {
                        warning != &format!("Optional package unavailable: {package}")
                    });
                } else if operation == "remove" && state["packages"][package]["status"] != "failed"
                {
                    record(
                        jobs,
                        &mut state,
                        package,
                        "failed",
                        operation,
                        required,
                        "Package is still installed after dependency resolution",
                    )?;
                    failures.push(package.clone());
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        failures.push(error.to_string());
        jobs.log(id, &format!("Package operations failed: {error}"))?;
    }
    if let Err(error) = restore_repository(jobs, id) {
        failures.push(format!("Repository recovery failed: {error}"));
    }
    match crate::services::repair_services(jobs, id, &mut options) {
        Ok(service_warnings) => warnings.extend(service_warnings),
        Err(error) => warnings.push(format!("Service repair failed: {error}")),
    }
    state["warnings"] = json!(warnings);
    let status = if !failures.is_empty() {
        "failed_packages"
    } else if !warnings.is_empty() {
        "complete_with_warnings"
    } else {
        "complete"
    };
    state["status"] = json!(status);
    state["error"] = json!(failures.join("; "));
    jobs.save(&mut state)?;
    if failures.is_empty()
        && let Err(error) = crate::local_feed::arm_after_recovery(jobs, id, &options)
    {
        jobs.log(
            id,
            &format!("Unable to enable local feed synchronization: {error}"),
        )?;
    }
    jobs.log(id, &format!("Post-reboot recovery finished: {status}"))
}
