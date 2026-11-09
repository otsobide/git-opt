//! The text of the bot's commits and pull requests.
//!
//! Both end with [`BOT_MARKER`]: it is how the bot recognises its own work in
//! any history (merge, rebase or squash). Changing it would make every past
//! optimization look new.

use std::collections::BTreeSet;

use crate::{
    model::FileChange,
    values::{OptimizerId, Savings},
};

pub const BOT_MARKER: &str = "Git-Opt: lossless-optimization";

const MAX_COMMIT_LINES: usize = 200;
const MAX_PR_ROWS: usize = 300;

fn percent(s: &Savings) -> String {
    format!("{:.2}%", s.percent())
}

/// Biggest saving first.
fn sorted(changes: &[FileChange]) -> Vec<&FileChange> {
    let mut v: Vec<&FileChange> = changes.iter().collect();
    v.sort_by_key(|c| std::cmp::Reverse(c.savings.saved()));
    v
}

pub fn commit_message(title: &str, changes: &[FileChange]) -> String {
    let total: Savings = changes.iter().map(|c| c.savings).sum();
    let mut msg = format!("{title}\n\n*Total -- {} -> {} ({})\n\n", total.before, total.after, percent(&total));
    for c in sorted(changes).iter().take(MAX_COMMIT_LINES) {
        msg.push_str(&format!("{} -- {} -> {} ({})\n", c.path, c.savings.before, c.savings.after, percent(&c.savings)));
    }
    if changes.len() > MAX_COMMIT_LINES {
        msg.push_str(&format!("…and {} more\n", changes.len() - MAX_COMMIT_LINES));
    }
    msg.push_str(&format!("\n{BOT_MARKER}\n"));
    msg
}

/// `explain` gives the one-line description of an optimizer (each format
/// strategy describes itself), shown once per optimizer used.
pub fn pull_request_body(changes: &[FileChange], explain: impl Fn(&OptimizerId) -> Option<String>) -> String {
    let total: Savings = changes.iter().map(|c| c.savings).sum();
    let mut body = String::from("## Lossless image optimization\n\n");
    body.push_str(&format!(
        "{} file{} optimized: **{} → {}** (saves {}, **{}**).\n\n",
        changes.len(),
        if changes.len() == 1 { "" } else { "s" },
        total.before,
        total.after,
        total.saved(),
        percent(&total),
    ));
    body.push_str(
        "Every image renders to **exactly the same pixels** as before: only the encoding changed. \
         Each result was decoded and compared pixel by pixel against the original before committing.\n\n",
    );
    body.push_str("| File | Before | After | Saved |\n|:--|--:|--:|--:|\n");
    for c in sorted(changes).iter().take(MAX_PR_ROWS) {
        let path = c.path.as_str().replace('|', "\\|").replace('`', "'");
        body.push_str(&format!("| `{path}` | {} | {} | {} |\n", c.savings.before, c.savings.after, percent(&c.savings)));
    }
    if changes.len() > MAX_PR_ROWS {
        body.push_str(&format!("| …and {} more | | | |\n", changes.len() - MAX_PR_ROWS));
    }
    body.push_str("\n<details><summary>How</summary>\n\n");
    let used: BTreeSet<&OptimizerId> = changes.iter().map(|c| &c.optimizer).collect();
    for line in used.into_iter().filter_map(explain) {
        body.push_str(&format!("- {line}\n"));
    }
    body.push_str("\n</details>\n\n");
    body.push_str(&format!("---\n<sub>Opened by git-opt, a self-hosted bot. `{BOT_MARKER}`</sub>\n"));
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::{BlobId, ByteSize, RepoPath};

    fn change(path: &str, optimizer: &str, before: u64, after: u64) -> FileChange {
        FileChange {
            path: RepoPath::new(path).unwrap(),
            optimizer: OptimizerId::new(optimizer).unwrap(),
            original_blob: BlobId::new("a".repeat(40)).unwrap(),
            savings: Savings::new(ByteSize::new(before), ByteSize::new(after)),
        }
    }

    fn changes() -> Vec<FileChange> {
        vec![change("docs/b.jpg", "mozjpeg", 4096, 4000), change("a.png", "oxipng", 2048, 1024)]
    }

    #[test]
    fn commit_message_carries_marker_and_totals() {
        let msg = commit_message("[git-opt] Optimize images", &changes());
        assert!(msg.starts_with("[git-opt] Optimize images\n\n"));
        assert!(msg.contains("*Total -- 6.00 KiB -> 4.91 KiB (18.23%)"), "{msg}");
        assert!(msg.trim_end().ends_with(BOT_MARKER));
        assert!(msg.find("a.png").unwrap() < msg.find("docs/b.jpg").unwrap(), "biggest saving first");
    }

    #[test]
    fn pr_body_explains_each_optimizer_once() {
        let body = pull_request_body(&changes(), |id| Some(format!("explained {id}")));
        assert!(body.contains("- explained mozjpeg\n") && body.contains("- explained oxipng\n"));
        assert_eq!(body.matches("explained").count(), 2);
        assert!(body.contains(BOT_MARKER));
    }
}
