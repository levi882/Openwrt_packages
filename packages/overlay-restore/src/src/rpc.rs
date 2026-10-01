use crate::engine::Jobs;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fs;
use std::path::{Component, Path, PathBuf};

fn backup_path(jobs: &Jobs, value: &Value) -> Result<PathBuf> {
    let path = value.as_str().context("Invalid backup path")?;
    let source = Path::new(path);
    if !source.is_absolute()
        || path.contains('\0')
        || source
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        || !(path.ends_with(".tar.gz") || path.ends_with(".tgz"))
    {
        bail!("Select an absolute .tar.gz or .tgz backup path");
    }
    let rooted = jobs.root.join(source.strip_prefix("/")?);
    let parent = rooted.parent().context("Backup has no parent directory")?;
    let parent = parent
        .canonicalize()
        .context("Backup directory not found")?;
    if !parent.starts_with(&jobs.root) {
        bail!("Backup directory is outside the router filesystem");
    }
    // Resolve mount/directory aliases but leave the file itself to open_regular(),
    // which rejects symbolic links and special files before reading any contents.
    Ok(parent.join(rooted.file_name().context("Backup has no filename")?))
}

pub fn call(jobs: &Jobs, method: &str, arguments: &Value) -> Result<Value> {
    let allowed: &[&str] = match method {
        "prepare" => &["path"],
        "list" => &[],
        "status" | "retry" => &["id"],
        "apply" => &["id", "confirmation"],
        _ => bail!("Unknown recovery method"),
    };
    let arguments = arguments
        .as_object()
        .context("Expected recovery arguments")?;
    if arguments
        .keys()
        .any(|key| key != "ubus_rpc_session" && !allowed.contains(&key.as_str()))
    {
        bail!("Unexpected recovery arguments");
    }
    if method == "prepare" {
        let upload = jobs.root.join("tmp/overlay-restore-upload.tar.gz");
        let source = match arguments.get("path") {
            Some(path) => backup_path(jobs, path)?,
            None => upload.clone(),
        };
        let state = jobs.prepare(&source, &jobs.read_settings(None)?)?;
        if arguments.get("path").is_none() {
            fs::remove_file(upload)?;
        }
        jobs.start_worker()?;
        return Ok(json!({"id": state["id"]}));
    }
    if method == "list" {
        return list(jobs);
    }
    let id = arguments
        .get("id")
        .and_then(Value::as_str)
        .context("Invalid task ID")?;
    if method == "status" {
        return jobs.public(id, true);
    }
    let state = if method == "apply" {
        jobs.apply(
            id,
            arguments
                .get("confirmation")
                .and_then(Value::as_str)
                .context("Explicit confirmation is required")?,
            None,
        )?
    } else {
        jobs.retry(id)?
    };
    jobs.start_worker()?;
    Ok(json!({"id": state["id"], "status": state["status"]}))
}

pub fn list(jobs: &Jobs) -> Result<Value> {
    let mut tasks = Vec::new();
    for state in jobs.states()?.iter().take(10) {
        tasks.push(jobs.public(state["id"].as_str().context("Task has no ID")?, false)?);
    }
    Ok(json!({"tasks": tasks}))
}
