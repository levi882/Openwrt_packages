use crate::engine::Jobs;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fs;

pub fn call(jobs: &Jobs, method: &str, arguments: &Value) -> Result<Value> {
    let allowed: &[&str] = match method {
        "prepare" | "list" => &[],
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
        let state = jobs.prepare(&upload, &jobs.read_settings(None)?)?;
        fs::remove_file(upload)?;
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
