use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, bail};
use tokio::process::Command;

static DEBUG_LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();

pub fn init_debug_log(verbose: bool) {
    DEBUG_LOG.get_or_init(|| {
        let env_enabled = std::env::var("GCT_DEBUG")
            .map(|v| matches!(v.as_str(), "1" | "true"))
            .unwrap_or(false);
        if !env_enabled && !verbose {
            return None;
        }
        let path = debug_log_path();
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!(
                "Warning: cannot create debug log directory {}: {e}",
                parent.display()
            );
            return None;
        }
        match OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
        {
            Ok(f) => {
                eprintln!("Debug log: {}", path.display());
                Some(Mutex::new(f))
            }
            Err(e) => {
                eprintln!("Warning: cannot open debug log {}: {e}", path.display());
                None
            }
        }
    });
}

fn debug_log_path() -> PathBuf {
    crate::config::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/gct/debug.log")
}

pub fn debug_log(msg: &str) {
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
