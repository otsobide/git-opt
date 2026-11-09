//! Commit message and pull request body.
//!
//! Both end with [`BOT_MARKER`], which is how the bot recognises its own work
//! in any history (merge, rebase or squash).

use crate::git::BOT_MARKER;

const MAX_COMMIT_LINES: usize = 200;
const MAX_PR_ROWS: usize = 300;

#[derive(Debug, Clone)]
pub struct FileReport {
    pub path: String,
    pub optimizer: String,
    pub before: u64,
    pub after: u64,
}

pub fn totals(files: &[FileReport]) -> (u64, u64) {
    files.iter().fold((0, 0), |(b, a), f| (b + f.before, a + f.after))
}

pub fn commit_message(title: &str, files: &[FileReport]) -> String {
    let (before, after) = totals(files);
    let mut msg = format!("{title}\n\n*Total -- {} -> {} ({})\n\n", size(before), size(after), percent(before, after));
    for f in sorted(files).iter().take(MAX_COMMIT_LINES) {
        msg.push_str(&format!("{} -- {} -> {} ({})\n", f.path, size(f.before), size(f.after), percent(f.before, f.after)));
    }
    if files.len() > MAX_COMMIT_LINES {
        msg.push_str(&format!("…and {} more\n", files.len() - MAX_COMMIT_LINES));
    }
    msg.push_str(&format!("\n{BOT_MARKER}\n"));
    msg
}

pub fn pr_body(files: &[FileReport]) -> String {
    let (before, after) = totals(files);
    let mut body = String::new();
    body.push_str("## Lossless image optimization\n\n");
    body.push_str(&format!(
        "{} file{} optimized: **{} → {}** (saves {}, **{}**).\n\n",
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        size(before),
        size(after),
        size(before - after),
        percent(before, after),
    ));
    body.push_str(
        "Every image decodes to **exactly the same pixels** as before: only the encoding changed. \
         Each result was decoded and compared pixel by pixel against the original before committing.\n\n",
    );
    body.push_str("| File | Before | After | Saved |\n|:--|--:|--:|--:|\n");
    for f in sorted(files).iter().take(MAX_PR_ROWS) {
        body.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            f.path.replace('|', "\\|").replace('`', "'"),
            size(f.before),
            size(f.after),
            percent(f.before, f.after)
        ));
    }
    if files.len() > MAX_PR_ROWS {
        body.push_str(&format!("| …and {} more | | | |\n", files.len() - MAX_PR_ROWS));
    }
    body.push_str("\n<details><summary>How</summary>\n\n");
    let used = |id: &str| files.iter().any(|f| f.optimizer == id);
    if used("oxipng") {
        body.push_str("- **PNG**: re-compressed with [oxipng](https://github.com/shssoichiro/oxipng) (filters, deflate, lossless color-type reduction).\n");
    }
    if used("mozjpeg") {
        body.push_str("- **JPEG**: lossless transcode with [mozjpeg](https://github.com/mozilla/mozjpeg) (like `jpegtran -copy all -optimize -progressive`): optimized Huffman tables and scans; DCT coefficients and all metadata (EXIF, ICC, XMP) are untouched.\n");
    }
    if used("webp") {
        body.push_str("- **WebP**: lossless files only, re-encoded with libwebp at maximum effort (`-lossless -z 9 -exact`); ICC, EXIF and XMP chunks are kept.\n");
    }
    if used("gifsicle") {
        body.push_str("- **GIF**: `gifsicle -O3`, without `--lossy`.\n");
    }
    body.push_str("\n</details>\n\n");
    body.push_str(&format!("---\n<sub>Opened by git-opt, a self-hosted bot. `{BOT_MARKER}`</sub>\n"));
    body
}

fn sorted(files: &[FileReport]) -> Vec<&FileReport> {
    let mut v: Vec<&FileReport> = files.iter().collect();
    v.sort_by_key(|f| std::cmp::Reverse(f.before.saturating_sub(f.after)));
    v
}

pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{v:.2} {}", UNITS[unit]) }
}

fn percent(before: u64, after: u64) -> String {
    if before == 0 {
        return "0%".into();
    }
    format!("{:.2}%", (before.saturating_sub(after)) as f64 * 100.0 / before as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<FileReport> {
        vec![
            FileReport { path: "a.png".into(), optimizer: "oxipng".into(), before: 2048, after: 1024 },
            FileReport { path: "docs/b.jpg".into(), optimizer: "mozjpeg".into(), before: 4096, after: 4000 },
        ]
    }

    #[test]
    fn commit_message_carries_marker_and_totals() {
        let msg = commit_message("[git-opt] Optimize images", &files());
        assert!(msg.starts_with("[git-opt] Optimize images\n\n"));
        assert!(msg.contains("*Total -- 6.00 KiB -> 4.91 KiB"));
        assert!(msg.trim_end().ends_with(BOT_MARKER));
        // Biggest saving first.
        assert!(msg.find("a.png").unwrap() < msg.find("docs/b.jpg").unwrap());
    }

    #[test]
    fn pr_body_carries_marker() {
        assert!(pr_body(&files()).contains(BOT_MARKER));
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1536), "1.50 KiB");
        assert_eq!(size(5 * 1024 * 1024), "5.00 MiB");
    }
}
