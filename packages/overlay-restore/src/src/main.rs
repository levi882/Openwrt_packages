use anyhow::{Context, Result, bail};
use overlay_restore::{engine::Jobs, rpc};
use serde_json::Value;
use std::io::Read;
use std::path::Path;

fn help() {
    println!(
        "overlay-restore {}\nInspect and migrate OpenWrt backup configuration\n\nCommands:\n  inspect BACKUP\n  apply TASK_ID --confirm TASK_ID [--no-reboot]\n  status TASK_ID\n  retry TASK_ID\n  list\n  usage\n  cleanup\n  remove TASK_ID --confirm TASK_ID\n  worker [--once]\n  rpc METHOD\n  --version",
        env!("CARGO_PKG_VERSION")
    );
}

fn main_inner() -> Result<i32> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let command = arguments.first().map(String::as_str).unwrap_or("--help");
    if ["--help", "-h", "help"].contains(&command) {
        help();
        return Ok(0);
    }
    if command == "--version" {
        println!(
            "overlay-restore {} (Rust, x86_64)",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(0);
    }
    // SAFETY: geteuid has no arguments or memory preconditions.
    if unsafe { libc::geteuid() } != 0 {
        bail!("Run the recovery tool as root");
    }
    if !Path::new("/etc/openwrt_release").is_file() || !Path::new("/usr/bin/apk").is_file() {
        bail!("This command requires OpenWrt with APK; use cargo test on development hosts");
    }
    let jobs = Jobs::new()?;
    let mut exit_code = 0;
    let result = match command {
        "inspect" if arguments.len() == 2 => {
            let environment = std::env::vars().collect();
            let options = jobs.read_settings(Some(&environment))?;
            let state = jobs.prepare(Path::new(&arguments[1]), &options)?;
            let id = state["id"].as_str().context("Task has no ID")?;
            jobs.validate(id)?;
            let result = jobs.public(id, true)?;
            if result["status"] != "ready" {
                exit_code = 1;
            }
            result
        }
        "apply" => {
            let id = arguments.get(1).context("Task ID is required")?;
            let mut confirmation = None;
            let mut reboot = None;
            let mut index = 2;
            while index < arguments.len() {
                match arguments[index].as_str() {
                    "--confirm" if confirmation.is_none() => {
                        index += 1;
                        confirmation = Some(
                            arguments
                                .get(index)
                                .context("Confirmation ID is required")?,
                        );
                    }
                    "--no-reboot" if reboot.is_none() => reboot = Some(false),
                    _ => bail!("Unexpected apply argument"),
                }
                index += 1;
            }
            let state = jobs.apply(
                id,
                confirmation.context("--confirm TASK_ID is required")?,
                reboot,
            )?;
            jobs.start_worker()?;
            state
        }
        "status" if arguments.len() == 2 => jobs.public(&arguments[1], true)?,
        "retry" if arguments.len() == 2 => {
            let state = jobs.retry(&arguments[1])?;
            jobs.start_worker()?;
            state
        }
        "list" if arguments.len() == 1 => rpc::list(&jobs)?,
        "usage" if arguments.len() == 1 => jobs.usage()?,
        "cleanup" if arguments.len() == 1 => jobs.cleanup()?,
        "remove" if arguments.len() == 4 && arguments[2] == "--confirm" => {
            jobs.remove(&arguments[1], &arguments[3])?
        }
        "worker" if arguments.len() == 1 || arguments.len() == 2 && arguments[1] == "--once" => {
            jobs.worker(arguments.len() == 2)?;
            return Ok(0);
        }
        "rpc" if arguments.len() == 2 => {
            let mut input = Vec::new();
            std::io::stdin().take(65537).read_to_end(&mut input)?;
            if input.len() > 65536 {
                bail!("RPC arguments exceed their size limit");
            }
            let input: Value = serde_json::from_slice(&input)?;
            rpc::call(&jobs, &arguments[1], &input)?
        }
        _ => bail!("Unknown command or invalid arguments; use --help"),
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(exit_code)
}

fn main() {
    let result = main_inner();
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            println!("{}", serde_json::json!({"error": error.to_string()}));
            std::process::exit(1);
        }
    }
}
