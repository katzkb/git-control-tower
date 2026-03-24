use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, bail};
use tokio::process::Command;

static DEBUG_LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();

pub fn init_debug_log() {
    DEBUG_LOG.get_or_init(|| {
        if std::env::var("GCT_DEBUG").is_ok() {
            let path = debug_log_path();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)
                .ok()
                .map(Mutex::new)
        } else {
            None
        }
    });
}

fn debug_log_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/gct/debug.log")
}

fn debug_log(msg: &str) {
    if let Some(Some(file)) = DEBUG_LOG.get()
        && let Ok(mut f) = file.lock()
    {
        let _ = writeln!(f, "{msg}");
        let _ = f.flush();
    }
}

pub async fn run_git(args: &[&str]) -> Result<String> {
    debug_log(&format!("$ git {}", args.join(" ")));

    let output = Command::new("git")
        .args(args)
        .output()
        .await
        .context("failed to execute git")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug_log(&format!("  → FAIL: {}", stderr.trim()));
        bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    debug_log(&format!("  → OK ({} bytes)", stdout.len()));
    Ok(stdout)
}

pub async fn run_gh(args: &[&str]) -> Result<String> {
    debug_log(&format!("$ gh {}", args.join(" ")));

    let output = Command::new("gh")
        .args(args)
        .output()
        .await
        .context("failed to execute gh")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug_log(&format!("  → FAIL: {}", stderr.trim()));
        bail!("gh {} failed: {}", args.join(" "), stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    debug_log(&format!("  → OK ({} bytes)", stdout.len()));
    Ok(stdout)
}
