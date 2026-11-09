//! Thin async wrapper around the system `git` binary.
//!
//! Credentials never appear on the command line: when a token is available it
//! is injected as an `http.<host>.extraHeader` through `GIT_CONFIG_*`
//! environment variables, which only the current user can read.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use base64::Engine;
use tokio::{io::AsyncWriteExt, process::Command};

/// Trailer/marker that identifies this bot's commits in any history.
pub const BOT_MARKER: &str = "Git-Opt: lossless-optimization";

const NETWORK_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const LOCAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// HTTP credentials for one host, e.g. `https://github.com/`.
#[derive(Clone)]
pub struct GitAuth {
    pub url_prefix: String,
    pub token: String,
}

impl GitAuth {
    fn env(&self) -> Vec<(String, String)> {
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{}", self.token));
        vec![
            ("GIT_CONFIG_COUNT".into(), "1".into()),
            ("GIT_CONFIG_KEY_0".into(), format!("http.{}.extraHeader", self.url_prefix)),
            ("GIT_CONFIG_VALUE_0".into(), format!("Authorization: Basic {basic}")),
        ]
    }
}

/// Author identity for bot commits.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

/// One blob from `git ls-tree -r -l`.
#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub sha: String,
    pub size: u64,
    pub path: String,
}

#[derive(Clone)]
pub struct Git {
    dir: PathBuf,
    auth: Option<GitAuth>,
    /// Extra `-c key=value` pairs prepended to every command.
    config: Vec<String>,
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>, auth: Option<GitAuth>) -> Self {
        Self { dir: dir.into(), auth, config: Vec::new() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Adds a `-c key=value` applied to every git invocation of this handle.
    pub fn with_config(mut self, kv: impl Into<String>) -> Self {
        self.config.push(kv.into());
        self
    }

    fn command(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "true")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(auth) = &self.auth {
            cmd.envs(auth.env());
        }
        for kv in &self.config {
            cmd.arg("-c").arg(kv);
        }
        cmd
    }

    async fn exec(&self, cwd: &Path, args: &[&str], stdin: Option<&[u8]>, timeout: Duration) -> Result<Vec<u8>> {
        let mut cmd = self.command(cwd);
        cmd.args(args);
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd.spawn().context("failed to spawn git (is it installed?)")?;
        if let Some(input) = stdin {
            let mut pipe = child.stdin.take().expect("stdin piped");
            let input = input.to_vec();
            tokio::spawn(async move {
                let _ = pipe.write_all(&input).await;
            });
        }
        let out = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .with_context(|| format!("git {} timed out", args.first().unwrap_or(&"")))??;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!("git {} failed: {}", args.join(" "), stderr.trim());
        }
        Ok(out.stdout)
    }

    async fn run(&self, args: &[&str]) -> Result<String> {
        let out = self.exec(&self.dir, args, None, LOCAL_TIMEOUT).await?;
        Ok(String::from_utf8_lossy(&out).trim_end().to_string())
    }

    async fn run_network(&self, args: &[&str]) -> Result<String> {
        let out = self.exec(&self.dir, args, None, NETWORK_TIMEOUT).await?;
        Ok(String::from_utf8_lossy(&out).trim_end().to_string())
    }

    pub fn is_repo(&self) -> bool {
        self.dir.join(".git").is_dir()
    }

    /// Clones `url` into this handle's directory (single branch, no tags).
    pub async fn clone_from(&self, url: &str, branch: &str) -> Result<()> {
        let parent = self.dir.parent().context("clone dir has no parent")?;
        tokio::fs::create_dir_all(parent).await?;
        if self.dir.exists() {
            tokio::fs::remove_dir_all(&self.dir).await?;
        }
        let target = self.dir.to_string_lossy().to_string();
        self.exec(
            parent,
            &["clone", "--quiet", "--no-tags", "--single-branch", "--branch", branch, url, &target],
            None,
            NETWORK_TIMEOUT,
        )
        .await?;
        // The clone belongs to the bot: keep bytes untouched and hooks off.
        self.run(&["config", "core.autocrlf", "false"]).await?;
        self.run(&["config", "advice.detachedHead", "false"]).await?;
        Ok(())
    }

    pub async fn set_origin(&self, url: &str) -> Result<()> {
        self.run(&["remote", "set-url", "origin", url]).await.map(drop)
    }

    /// Fetches `branch` from origin into `refs/remotes/origin/<branch>`.
    pub async fn fetch_branch(&self, branch: &str) -> Result<()> {
        let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
        self.run_network(&["fetch", "--quiet", "--no-tags", "--prune", "origin", &refspec])
            .await
            .map(drop)
    }

    pub async fn rev_parse(&self, rev: &str) -> Result<String> {
        self.run(&["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")])
            .await
            .with_context(|| format!("unknown revision {rev}"))
    }

    /// All regular-file blobs reachable from `rev`, with their sizes.
    pub async fn ls_tree(&self, rev: &str) -> Result<Vec<TreeEntry>> {
        let out = self.exec(&self.dir, &["ls-tree", "-r", "-l", "-z", "--full-tree", rev], None, LOCAL_TIMEOUT).await?;
        let mut entries = Vec::new();
        for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record = String::from_utf8_lossy(record);
            let Some((meta, path)) = record.split_once('\t') else { continue };
            let mut parts = meta.split_whitespace();
            let (Some(mode), Some(kind), Some(sha), Some(size)) = (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            // Only regular files: skip symlinks (120000) and submodules (commit).
            if kind != "blob" || !(mode == "100644" || mode == "100755") {
                continue;
            }
            entries.push(TreeEntry {
                mode: mode.into(),
                sha: sha.into(),
                size: size.parse().unwrap_or(0),
                path: path.into(),
            });
        }
        Ok(entries)
    }

    /// Blob ids written by any commit in `rev`'s history that carries the bot
    /// marker. Works for merge, rebase and squash merges because it relies on
    /// content hashes rather than on commit ids.
    pub async fn blobs_from_bot_commits(&self, rev: &str) -> Result<HashSet<String>> {
        let out = self
            .run(&[
                "log",
                "--format=%H",
                "--raw",
                "--no-abbrev",
                "--no-renames",
                "--fixed-strings",
                &format!("--grep={BOT_MARKER}"),
                rev,
            ])
            .await?;
        Ok(out
            .lines()
            .filter(|l| l.starts_with(':'))
            .filter_map(|l| l.split_whitespace().nth(3))
            .filter(|sha| sha.bytes().any(|b| b != b'0'))
            .map(str::to_string)
            .collect())
    }

    /// Resets the working tree to `start` on a fresh local `branch`.
    pub async fn checkout_fresh(&self, branch: &str, start: &str) -> Result<()> {
        self.run(&["checkout", "--quiet", "--force", "-B", branch, start]).await?;
        self.run(&["clean", "-ffdxq"]).await?;
        Ok(())
    }

    /// Paths handled by Git LFS (we never touch those).
    pub async fn lfs_paths(&self, paths: &[&str]) -> Result<HashSet<String>> {
        if paths.is_empty() {
            return Ok(HashSet::new());
        }
        let mut input = Vec::new();
        for p in paths {
            input.extend_from_slice(p.as_bytes());
            input.push(0);
        }
        let out = self
            .exec(&self.dir, &["check-attr", "-z", "--stdin", "filter"], Some(&input), LOCAL_TIMEOUT)
            .await?;
        let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
        Ok(fields
            .chunks(3)
            .filter(|c| c.len() == 3 && c[2] == b"lfs")
            .map(|c| String::from_utf8_lossy(c[0]).into_owned())
            .collect())
    }

    pub async fn has_object(&self, sha: &str) -> bool {
        self.run(&["cat-file", "-e", sha]).await.is_ok()
    }

    pub async fn cat_blob(&self, sha: &str) -> Result<Vec<u8>> {
        self.exec(&self.dir, &["cat-file", "blob", sha], None, LOCAL_TIMEOUT).await
    }

    /// Stages `paths` and commits them. Returns the new commit id and the
    /// blob id each path now has.
    pub async fn commit(
        &self,
        paths: &[String],
        message: &str,
        identity: &Identity,
        sign: bool,
    ) -> Result<(String, Vec<(String, String)>)> {
        let mut pathspec = Vec::new();
        for p in paths {
            pathspec.extend_from_slice(p.as_bytes());
            pathspec.push(0);
        }
        self.exec(
            &self.dir,
            &["add", "--pathspec-from-file=-", "--pathspec-file-nul"],
            Some(&pathspec),
            LOCAL_TIMEOUT,
        )
        .await?;

        let staged = self
            .exec(&self.dir, &["diff", "--cached", "--name-only", "-z"], None, LOCAL_TIMEOUT)
            .await?;
        if staged.is_empty() {
            bail!("nothing staged to commit");
        }

        let name = format!("user.name={}", identity.name);
        let email = format!("user.email={}", identity.email);
        let mut args = vec!["-c", &name, "-c", &email, "-c", "core.hooksPath=/dev/null"];
        if !sign {
            args.extend(["-c", "commit.gpgsign=false"]);
        }
        args.extend(["commit", "--quiet", "--no-verify", "--file=-"]);
        self.exec(&self.dir, &args, Some(message.as_bytes()), LOCAL_TIMEOUT).await?;

        let sha = self.rev_parse("HEAD").await?;
        // `-z` raw output alternates "<:meta>\0<path>\0".
        let raw = self
            .exec(&self.dir, &["diff-tree", "-r", "-z", "--no-commit-id", "--no-renames", "HEAD"], None, LOCAL_TIMEOUT)
            .await?;
        let fields: Vec<&[u8]> = raw.split(|b| *b == 0).filter(|r| !r.is_empty()).collect();
        let blobs = fields
            .chunks(2)
            .filter(|c| c.len() == 2)
            .filter_map(|c| {
                let meta = String::from_utf8_lossy(c[0]);
                let new_sha = meta.split_whitespace().nth(3)?.to_string();
                Some((String::from_utf8_lossy(c[1]).into_owned(), new_sha))
            })
            .collect();
        Ok((sha, blobs))
    }

    /// Force-pushes the current HEAD to `branch` on `url`.
    pub async fn push(&self, url: &str, branch: &str) -> Result<()> {
        let refspec = format!("+HEAD:refs/heads/{branch}");
        self.run_network(&["push", "--quiet", "--no-verify", url, &refspec]).await.map(drop)
    }

    /// Reads a config value as seen from this directory (repo, global, system).
    pub async fn config_get(&self, key: &str) -> Option<String> {
        let dir = if self.dir.is_dir() { self.dir.clone() } else { std::env::temp_dir() };
        let out = self.exec(&dir, &["config", "--get", key], None, LOCAL_TIMEOUT).await.ok()?;
        let v = String::from_utf8_lossy(&out).trim().to_string();
        (!v.is_empty()).then_some(v)
    }
}

/// `git --version`, or an error if git is not installed.
pub async fn version() -> Result<String> {
    let out = Git::new(std::env::temp_dir(), None)
        .exec(&std::env::temp_dir(), &["--version"], None, LOCAL_TIMEOUT)
        .await?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// `https://github.com/owner/repo.git` -> `https://github.com/`
pub fn url_prefix(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    Some(format!("https://{host}/"))
}
