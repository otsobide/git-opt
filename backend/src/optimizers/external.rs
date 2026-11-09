//! Helpers to drive external command-line optimizers through temp files.

use std::{path::Path, process::Stdio, time::Duration};

use anyhow::{Context, Result, bail};
use tokio::process::Command;

const TIMEOUT: Duration = Duration::from_secs(180);

/// Runs `program` with `args`, where `{in}` and `{out}` are replaced by temp
/// file paths. Returns the bytes written to `{out}`.
pub async fn run_on_bytes(program: &str, args: &[&str], input: &[u8], ext: &str) -> Result<Vec<u8>> {
    let dir = tempfile::Builder::new().prefix("git-opt-").tempdir()?;
    let in_path = dir.path().join(format!("in.{ext}"));
    let out_path = dir.path().join(format!("out.{ext}"));
    tokio::fs::write(&in_path, input).await?;

    let args: Vec<String> = args.iter().map(|a| substitute(a, &in_path, &out_path)).collect();
    let mut cmd = Command::new(program);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let out = tokio::time::timeout(TIMEOUT, cmd.output())
        .await
        .with_context(|| format!("{program} timed out"))?
        .with_context(|| format!("cannot run {program}"))?;
    if !out.status.success() {
        bail!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    tokio::fs::read(&out_path).await.with_context(|| format!("{program} produced no output"))
}

fn substitute(arg: &str, input: &Path, output: &Path) -> String {
    arg.replace("{in}", &input.to_string_lossy()).replace("{out}", &output.to_string_lossy())
}

/// Returns the first line printed by `program args…` (stdout or stderr), if
/// the program can be started at all.
pub async fn probe_version(program: &str, args: &[&str]) -> std::result::Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(|e| format!("`{program}` not found in PATH ({e})"))?;
    let text = if out.stdout.is_empty() { out.stderr } else { out.stdout };
    Ok(String::from_utf8_lossy(&text).lines().next().unwrap_or(program).trim().to_string())
}
