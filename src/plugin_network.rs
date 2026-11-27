//! Nonblocking, bounded HTTP jobs for plugin background callbacks.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
};
type Job = Arc<Mutex<Option<Value>>>;
static JOBS: LazyLock<Mutex<HashMap<(String, String), Job>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
pub fn get(plugin: &str, url: &str) -> Result<Value> {
    let key = (plugin.to_owned(), url.to_owned());
    let mut jobs = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(job) = jobs.get(&key) {
        let ready = job.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(response) = ready {
            jobs.remove(&key);
            return Ok(response);
        }
        return Ok(json!({"pending":true}));
    }
    if jobs.keys().filter(|(id, _)| id == plugin).count() >= 8 {
        bail!("too many pending plugin HTTP requests");
    }
    let job = Arc::new(Mutex::new(None));
    jobs.insert(key.clone(), job.clone());
    let url = url.to_owned();
    if let Err(error) = std::thread::Builder::new()
        .name("workshop-plugin-http".into())
        .spawn(move || {
            let response = fetch(&url).unwrap_or_else(|_| json!({"status":0,"body":""}));
            *job.lock().unwrap_or_else(|e| e.into_inner()) = Some(response);
        })
    {
        jobs.remove(&key);
        return Err(error.into());
    }
    Ok(json!({"pending":true}))
}
fn fetch(url: &str) -> Result<Value> {
    let output = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--max-time",
            "20",
            "--max-filesize",
            "8388608",
            "--write-out",
            "\n%{http_code}",
            "--",
            url,
        ])
        .output()?;
    if !output.status.success() || output.stdout.len() > 8 * 1024 * 1024 {
        bail!("plugin HTTP request failed");
    }
    let body = String::from_utf8(output.stdout)?;
    let (body, status) = body.rsplit_once('\n').context("HTTP status missing")?;
    Ok(json!({"status":status.parse::<u16>()?,"body":body}))
}
