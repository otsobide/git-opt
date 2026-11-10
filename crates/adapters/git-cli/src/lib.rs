//! The [`VersionControl`] port implemented with the system `git` binary.
//!
//! Every invocation runs with prompts disabled, a C locale and a timeout.
//! Credentials never appear in arguments or URLs: when a token is available
//! it is sent as an `http.<host>.extraHeader`, scoped to the host of the URL
//! being used, through `GIT_CONFIG_*` environment variables.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use base64::Engine;
use git_opt_domain::{
    ports::{TreeEntry, VersionControl, Workspace},
    values::{AccessToken, BlobId, BranchName, ByteSize, CloneUrl, CommitSha, Identity, RepoPath, RepoSlug},
};
use tokio::{io::AsyncWriteExt, process::Command, sync::Mutex};

const NETWORK_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const LOCAL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Working copies live in `<root>/<owner>/<name>`.
pub struct GitCli {
    root: PathBuf,
}

impl GitCli {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dir(&self, slug: &RepoSlug) -> PathBuf {
        self.root.join(slug.owner().as_str()).join(slug.name().as_str())
    }
}

#[async_trait]
impl VersionControl for GitCli {
    fn workspace(&self, slug: &RepoSlug, token: Option<AccessToken>) -> Box<dyn Workspace> {
        Box::new(GitWorkspace { dir: self.dir(slug), token, origin: Mutex::new(None) })
    }

    async fn remove_workspace(&self, slug: &RepoSlug) -> Result<()> {
        match tokio::fs::remove_dir_all(self.dir(slug)).await {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    async fn version(&self) -> Result<String> {
        let out = run_git(&std::env::temp_dir(), &["--version"], None, None, LOCAL_TIMEOUT).await?;
        Ok(String::from_utf8_lossy(&out).trim().to_string())
    }

    async fn configured_identity(&self) -> Option<Identity> {
        let get = |key: &'static str| async move {
            let out =
                run_git(&std::env::temp_dir(), &["config", "--get", key], None, None, LOCAL_TIMEOUT).await.ok()?;
            Some(String::from_utf8_lossy(&out).trim().to_string())
        };
        Identity::new(get("user.name").await?, get("user.email").await?).ok()
    }
}

/// HTTP credentials scoped to one URL prefix, as `GIT_CONFIG_*` variables.
fn credential_env(token: &AccessToken, url: &CloneUrl) -> Vec<(String, String)> {
    let Some(prefix) = url.https_prefix() else { return Vec::new() };
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{}", token.expose()));
    vec![
        ("GIT_CONFIG_COUNT".into(), "1".into()),
        ("GIT_CONFIG_KEY_0".into(), format!("http.{prefix}.extraHeader")),
        ("GIT_CONFIG_VALUE_0".into(), format!("Authorization: Basic {basic}")),
    ]
}

/// The git subcommand in `args`, skipping global `-c key=value` options.
fn subcommand<'a>(args: &[&'a str]) -> &'a str {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "-c" => {
                it.next();
            }
            a if a.starts_with('-') => {}
            a => return a,
        }
    }
    "command"
}

async fn run_git(
    cwd: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    env: Option<Vec<(String, String)>>,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "true")
        .env("LC_ALL", "C")
        .envs(env.unwrap_or_default())
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().context("failed to spawn git (is it installed?)")?;
    if let Some(input) = stdin {
        let mut pipe = child.stdin.take().expect("stdin is piped");
        let input = input.to_vec();
        tokio::spawn(async move {
            let _ = pipe.write_all(&input).await;
        });
    }
    let out = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .with_context(|| format!("git {} timed out", subcommand(args)))??;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        tracing::debug!(args = %args.join(" "), stderr = %stderr.trim(), "git failed");
        bail!("git {} failed: {}", subcommand(args), stderr.trim());
    }
    Ok(out.stdout)
}

fn nul_separated(paths: &[RepoPath]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in paths {
        out.extend_from_slice(p.as_str().as_bytes());
        out.push(0);
    }
    out
}

struct GitWorkspace {
    dir: PathBuf,
    token: Option<AccessToken>,
    /// Origin URL, remembered to scope credentials for fetches.
    origin: Mutex<Option<CloneUrl>>,
}

impl GitWorkspace {
    async fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        run_git(&self.dir, args, None, None, LOCAL_TIMEOUT).await
    }

    async fn git_text(&self, args: &[&str]) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.git(args).await?).trim_end().to_string())
    }

    async fn git_input(&self, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
        run_git(&self.dir, args, Some(input), None, LOCAL_TIMEOUT).await
    }

    /// A network command authenticated for `url`'s host.
    async fn git_remote(&self, cwd: &Path, args: &[&str], url: &CloneUrl) -> Result<Vec<u8>> {
        let env = self.token.as_ref().map(|t| credential_env(t, url));
        run_git(cwd, args, None, env, NETWORK_TIMEOUT).await
    }

    async fn origin(&self) -> Result<CloneUrl> {
        if let Some(url) = self.origin.lock().await.clone() {
            return Ok(url);
        }
        let url = CloneUrl::new(self.git_text(&["remote", "get-url", "origin"]).await?)?;
        *self.origin.lock().await = Some(url.clone());
        Ok(url)
    }

    async fn rev_parse(&self, rev: &str) -> Result<CommitSha> {
        let out = self.git_text(&["rev-parse", "--verify", "--quiet", &format!("{rev}^{{commit}}")]).await?;
        Ok(CommitSha::new(out)?)
    }
}

#[async_trait]
impl Workspace for GitWorkspace {
    fn exists(&self) -> bool {
        self.dir.join(".git").is_dir()
    }

    async fn clone_from(&self, url: &CloneUrl, branch: &BranchName) -> Result<()> {
        let parent = self.dir.parent().context("workspace has no parent directory")?;
        tokio::fs::create_dir_all(parent).await?;
        if self.dir.exists() {
            tokio::fs::remove_dir_all(&self.dir).await?;
        }
        let target = self.dir.to_string_lossy().to_string();
        let args =
            ["clone", "--quiet", "--no-tags", "--single-branch", "--branch", branch.as_str(), url.as_str(), &target];
        self.git_remote(parent, &args, url).await?;
        // The clone belongs to the bot: keep bytes untouched, no detached-HEAD chatter.
        self.git(&["config", "core.autocrlf", "false"]).await?;
        self.git(&["config", "advice.detachedHead", "false"]).await?;
        *self.origin.lock().await = Some(url.clone());
        Ok(())
    }

    async fn set_origin(&self, url: &CloneUrl) -> Result<()> {
        self.git(&["remote", "set-url", "origin", url.as_str()]).await?;
        *self.origin.lock().await = Some(url.clone());
        Ok(())
    }

    async fn fetch(&self, branch: &BranchName) -> Result<()> {
        let origin = self.origin().await?;
        let refspec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
        self.git_remote(&self.dir, &["fetch", "--quiet", "--no-tags", "--prune", "origin", &refspec], &origin)
            .await
            .map(drop)
    }

    async fn is_healthy(&self) -> bool {
        self.rev_parse("HEAD").await.is_ok()
    }

    async fn remote_head(&self, branch: &BranchName) -> Result<CommitSha> {
        self.rev_parse(&format!("refs/remotes/origin/{branch}")).await
    }

    async fn local_head(&self, branch: &BranchName) -> Option<CommitSha> {
        self.rev_parse(&format!("refs/heads/{branch}")).await.ok()
    }

    async fn tree(&self, commit: &CommitSha) -> Result<Vec<TreeEntry>> {
        let out = self.git(&["ls-tree", "-r", "-l", "-z", "--full-tree", commit.as_str()]).await?;
        let mut entries = Vec::new();
        for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record = String::from_utf8_lossy(record);
            let Some((meta, path)) = record.split_once('\t') else { continue };
            let mut parts = meta.split_whitespace();
            let (Some(mode), Some(kind), Some(sha), Some(size)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            // Only regular files: no symlinks (120000) or submodules (commit).
            if kind != "blob" || !(mode == "100644" || mode == "100755") {
                continue;
            }
            let (Ok(path), Ok(blob)) = (RepoPath::new(path), BlobId::new(sha)) else { continue };
            entries.push(TreeEntry { path, blob, size: ByteSize::new(size.parse().unwrap_or(0)) });
        }
        Ok(entries)
    }

    async fn blobs_written_by_marked_commits(&self, commit: &CommitSha, marker: &str) -> Result<HashSet<BlobId>> {
        let grep = format!("--grep={marker}");
        let args =
            ["log", "--format=%H", "--raw", "--no-abbrev", "--no-renames", "--fixed-strings", &grep, commit.as_str()];
        let out = self.git_text(&args).await?;
        Ok(out
            .lines()
            .filter(|l| l.starts_with(':'))
            .filter_map(|l| l.split_whitespace().nth(3))
            .filter_map(|sha| BlobId::new(sha).ok())
            .filter(|blob| !blob.is_null())
            .collect())
    }

    async fn checkout_new_branch(&self, branch: &BranchName, start: &CommitSha) -> Result<()> {
        self.git(&["checkout", "--quiet", "--force", "-B", branch.as_str(), start.as_str()]).await?;
        self.git(&["clean", "-ffdxq"]).await?;
        Ok(())
    }

    async fn lfs_paths(&self, paths: &[RepoPath]) -> Result<HashSet<RepoPath>> {
        if paths.is_empty() {
            return Ok(HashSet::new());
        }
        let out = self.git_input(&["check-attr", "-z", "--stdin", "filter"], &nul_separated(paths)).await?;
        let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
        Ok(fields
            .chunks(3)
            .filter(|c| c.len() == 3 && c[2] == b"lfs")
            .filter_map(|c| RepoPath::new(String::from_utf8_lossy(c[0]).into_owned()).ok())
            .collect())
    }

    async fn has_blob(&self, blob: &BlobId) -> bool {
        self.git(&["cat-file", "-e", blob.as_str()]).await.is_ok()
    }

    async fn read_blob(&self, blob: &BlobId) -> Result<Vec<u8>> {
        self.git(&["cat-file", "blob", blob.as_str()]).await
    }

    async fn read_file(&self, path: &RepoPath) -> Result<Vec<u8>> {
        tokio::fs::read(self.dir.join(path.as_str())).await.with_context(|| format!("reading {path}"))
    }

    async fn write_file(&self, path: &RepoPath, bytes: &[u8]) -> Result<()> {
        tokio::fs::write(self.dir.join(path.as_str()), bytes).await.with_context(|| format!("writing {path}"))
    }

    async fn commit(
        &self,
        paths: &[RepoPath],
        message: &str,
        author: &Identity,
        sign: bool,
    ) -> Result<(CommitSha, HashMap<RepoPath, BlobId>)> {
        self.git_input(&["add", "--pathspec-from-file=-", "--pathspec-file-nul"], &nul_separated(paths)).await?;
        if self.git(&["diff", "--cached", "--name-only", "-z"]).await?.is_empty() {
            bail!("nothing staged to commit");
        }
        let name = format!("user.name={}", author.name());
        let email = format!("user.email={}", author.email());
        let mut args = vec!["-c", &name, "-c", &email, "-c", "core.hooksPath=/dev/null"];
        if !sign {
            args.extend(["-c", "commit.gpgsign=false"]);
        }
        args.extend(["commit", "--quiet", "--no-verify", "--file=-"]);
        self.git_input(&args, message.as_bytes()).await?;

        let sha = self.rev_parse("HEAD").await?;
        // `-z` raw output alternates "<:meta>\0<path>\0".
        let raw = self.git(&["diff-tree", "-r", "-z", "--no-commit-id", "--no-renames", "HEAD"]).await?;
        let fields: Vec<&[u8]> = raw.split(|b| *b == 0).filter(|r| !r.is_empty()).collect();
        let blobs = fields
            .chunks(2)
            .filter(|c| c.len() == 2)
            .filter_map(|c| {
                let meta = String::from_utf8_lossy(c[0]);
                let blob = BlobId::new(meta.split_whitespace().nth(3)?).ok()?;
                Some((RepoPath::new(String::from_utf8_lossy(c[1]).into_owned()).ok()?, blob))
            })
            .collect();
        Ok((sha, blobs))
    }

    async fn push(&self, url: &CloneUrl, branch: &BranchName) -> Result<()> {
        let refspec = format!("+HEAD:refs/heads/{branch}");
        self.git_remote(&self.dir, &["push", "--quiet", "--no-verify", url.as_str(), &refspec], url).await.map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_subcommand() {
        assert_eq!(subcommand(&["clone", "--quiet", "url"]), "clone");
        assert_eq!(subcommand(&["-c", "user.name=x", "-c", "core.hooksPath=/dev/null", "commit", "-q"]), "commit");
        assert_eq!(subcommand(&["--version"]), "command");
    }

    #[test]
    fn credentials_are_scoped_to_https_hosts() {
        let token = AccessToken::new("secret").unwrap();
        let env = credential_env(&token, &CloneUrl::new("https://github.com/o/r.git").unwrap());
        assert_eq!(env[1].1, "http.https://github.com/.extraHeader");
        assert!(!env[2].1.contains("secret"), "the token is base64-encoded in a header");
        assert!(credential_env(&token, &CloneUrl::new("file:///tmp/r.git").unwrap()).is_empty());
    }

    fn sh(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=T", "-c", "user.email=t@example.com", "-c", "init.defaultBranch=main"])
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    /// Clone, list, commit with the marker, find the marker's blobs, push.
    #[tokio::test]
    async fn round_trip_against_a_local_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("img")).unwrap();
        std::fs::write(src.join("img/a.png"), b"old bytes").unwrap();
        sh(&src, &["init", "-q"]);
        sh(&src, &["add", "-A"]);
        sh(&src, &["commit", "-qm", "init"]);
        sh(tmp.path(), &["clone", "-q", "--bare", "src", "remote.git"]);

        let git = GitCli::new(tmp.path().join("work"));
        let slug = RepoSlug::parse("octo/demo").unwrap();
        let ws = git.workspace(&slug, None);
        let url = CloneUrl::new(format!("file://{}", tmp.path().join("remote.git").display())).unwrap();
        let main = BranchName::new("main").unwrap();
        ws.clone_from(&url, &main).await.unwrap();
        assert!(ws.exists() && ws.is_healthy().await);
        ws.fetch(&main).await.unwrap();
        let head = ws.remote_head(&main).await.unwrap();

        let tree = ws.tree(&head).await.unwrap();
        assert_eq!(tree.len(), 1);
        let path = tree[0].path.clone();
        assert_eq!(ws.read_blob(&tree[0].blob).await.unwrap(), b"old bytes");

        let branch = BranchName::new("git-opt/test").unwrap();
        ws.checkout_new_branch(&branch, &head).await.unwrap();
        ws.write_file(&path, b"new bytes").await.unwrap();
        let author = Identity::new("Jane", "jane@example.com").unwrap();
        let (commit, blobs) =
            ws.commit(std::slice::from_ref(&path), "msg\n\nMarker: yes\n", &author, false).await.unwrap();
        assert_eq!(ws.local_head(&branch).await, Some(commit.clone()));
        let new_blob = blobs.get(&path).unwrap().clone();
        let marked = ws.blobs_written_by_marked_commits(&commit, "Marker: yes").await.unwrap();
        assert_eq!(marked, HashSet::from([new_blob]));

        ws.push(&url, &branch).await.unwrap();
        let other = git.workspace(&RepoSlug::parse("octo/check").unwrap(), None);
        other.clone_from(&url, &branch).await.unwrap();
        assert_eq!(other.read_file(&path).await.unwrap(), b"new bytes");

        git.remove_workspace(&slug).await.unwrap();
        assert!(!git.workspace(&slug, None).exists());
    }
}
