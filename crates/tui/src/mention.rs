//! `@`-file mentions for the TUI prompt.
//!
//! Two concerns live here, kept as pure functions so they can be unit-tested without a terminal:
//!
//! - The **file picker** backing data: listing the project's files ([`list_project_files`]) and
//!   fuzzy-ranking them against the fragment typed after `@` ([`fuzzy_filter`]). The picker's open
//!   state, key handling, and token insertion are owned by [`crate::app::App`]; this module only
//!   provides the candidate set and ranking.
//! - **Expansion for a live turn**: parsing `@path` (and `@path:start-end` / `@path#Lstart-end`
//!   line-range) tokens out of a submitted prompt ([`parse`]) and inlining the referenced files'
//!   contents into the text sent to the model ([`expand`]), so a mentioned file is actually provided
//!   to the model on the turn it is referenced.

use std::path::Path;
use std::process::Command;

/// The cap on candidate files held for the picker, so a very large tree cannot flood memory or the
/// popup. The list is sorted before truncation, so the cap is deterministic.
const MAX_FILES: usize = 5000;

/// The cap on bytes of a single mentioned file inlined into a turn, so a huge file cannot blow the
/// prompt. A larger file is truncated with an explicit marker.
const MAX_INLINE_BYTES: usize = 64 * 1024;

/// A parsed `@`-mention: a file path and an optional inclusive 1-based line range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    /// The referenced path, relative to the working directory (as the user typed it).
    pub path: String,
    /// An optional `(start, end)` 1-based inclusive line range.
    pub range: Option<(usize, usize)>,
}

/// A human-readable language/type label for a file path, derived from its extension.
///
/// A file-type helper of the mention module (e.g. `rust`, `toml`): common extensions map to a
/// friendly language name; anything else falls back to the lowercased extension, or `file` when the
/// path has no extension. Retained as part of the mention API and value-locked by its unit test; the
/// `@`-mention popup itself now surfaces a `dir · size` meta column rather than a type label.
#[must_use]
#[allow(dead_code)]
pub fn type_label(path: &str) -> String {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let Some(ext) = ext else {
        return "file".to_owned();
    };
    let friendly = match ext.as_str() {
        "rs" => "rust",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "go" => "go",
        "c" => "c",
        "h" | "hpp" => "header",
        "cc" | "cpp" | "cxx" => "c++",
        "sh" | "bash" | "zsh" => "shell",
        "html" | "htm" => "html",
        "css" => "css",
        "lock" => "lock",
        "txt" | "text" => "text",
        "sql" => "sql",
        "rb" => "ruby",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        _ => return ext,
    };
    friendly.to_owned()
}

/// Format a byte count in the repo-standard human-readable form (e.g. `0.7 KB`, `6.2 KB`, `1.2 MB`).
///
/// This is the value-locked `X.Y KB`/`MB`/`GB` size form; it is retained and pinned by its unit
/// test. The `@`-mention popup renders sizes with the tighter [`format_size_compact`] form
/// (`4.3k`/`1.2M`) to match `perm_mention.png`, so this function is no longer wired into the popup
/// meta column but is kept as the shared repo-standard formatter.
#[must_use]
#[allow(dead_code)]
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b < MB {
        format!("{:.1} KB", b / KB)
    } else if b < GB {
        format!("{:.1} MB", b / MB)
    } else {
        format!("{:.1} GB", b / GB)
    }
}

/// Format a byte count in the compact `@`-mention size form used by `perm_mention.png`: a raw byte
/// count under 1 KiB (`912`), then one decimal with a single-letter unit (`4.3k`, `1.2M`, `2.0G`).
///
/// This is the mention-local size renderer for the popup's `dir · size` meta column; it is distinct
/// from the shared [`format_size`] so tightening the popup's units does not disturb the value-locked
/// `X.Y KB` form.
#[must_use]
pub fn format_size_compact(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b < KB {
        bytes.to_string()
    } else if b < MB {
        format!("{:.1}k", b / KB)
    } else if b < GB {
        format!("{:.1}M", b / MB)
    } else {
        format!("{:.1}G", b / GB)
    }
}

/// List the project's files relative to `cwd`, for the file picker.
///
/// Backed by the real `rg --files` (ripgrep is a build precondition), so the list honors
/// `.gitignore` and skips build artifacts exactly like the agent's search tools. If ripgrep is not
/// available, falls back to a bounded manual walk so the picker still functions. Paths are returned
/// sorted and de-duplicated, capped at [`MAX_FILES`].
#[must_use]
pub fn list_project_files(cwd: &Path) -> Vec<String> {
    let mut files = ripgrep_files(cwd).unwrap_or_else(|| walk_files(cwd));
    files.sort_unstable();
    files.dedup();
    files.truncate(MAX_FILES);
    files
}

/// Try to list files via `rg --files`, returning `None` when ripgrep is unavailable or errors so the
/// caller can fall back to a manual walk.
fn ripgrep_files(cwd: &Path) -> Option<Vec<String>> {
    let output = Command::new("rg")
        .arg("--files")
        .arg("--color")
        .arg("never")
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let list: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    Some(list)
}

/// A bounded manual directory walk used only when ripgrep is unavailable. Skips common noise
/// directories and hidden entries, and stops once [`MAX_FILES`] paths are collected.
fn walk_files(cwd: &Path) -> Vec<String> {
    fn is_skipped(name: &str) -> bool {
        matches!(name, ".git" | "target" | "node_modules") || name.starts_with('.')
    }

    let mut out = Vec::new();
    let mut stack = vec![cwd.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_skipped(&name) {
                continue;
            }
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(ft) if ft.is_file() => {
                    if let Ok(rel) = path.strip_prefix(cwd) {
                        out.push(rel.to_string_lossy().into_owned());
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Fuzzy-rank `files` against `query` (the fragment typed after `@`), best match first.
///
/// An empty query keeps every file in its given (sorted) order. A non-empty query keeps only files
/// whose path contains the query as a case-insensitive subsequence, scored so that contiguous
/// matches, matches inside the file name, and segment-boundary matches rank higher; ties break on
/// the shorter path then lexicographically. The result is capped at `limit`.
#[must_use]
pub fn fuzzy_filter<'a>(files: &'a [String], query: &str, limit: usize) -> Vec<&'a str> {
    if query.trim().is_empty() {
        return files.iter().take(limit).map(String::as_str).collect();
    }
    let mut scored: Vec<(i32, &'a str)> = files
        .iter()
        .filter_map(|path| fuzzy_score(path, query).map(|score| (score, path.as_str())))
        .collect();
    // Higher score first; then shorter path; then lexicographic for a stable, deterministic order.
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.len().cmp(&b.1.len()))
            .then_with(|| a.1.cmp(b.1))
    });
    scored.into_iter().take(limit).map(|(_, p)| p).collect()
}

/// Score `path` against `query`, or `None` when `query` is not a case-insensitive subsequence of
/// `path`. A higher score is a better match.
fn fuzzy_score(path: &str, query: &str) -> Option<i32> {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let p = path.to_ascii_lowercase();
    let chars: Vec<char> = p.chars().collect();
    let needles: Vec<char> = q.chars().collect();

    let mut score = 0i32;
    let mut qi = 0;
    let mut prev: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        if qi < needles.len() && c == needles[qi] {
            score += 1;
            if let Some(p) = prev {
                if p + 1 == i {
                    score += 5;
                }
            }
            if i == 0 || matches!(chars[i - 1], '/' | '_' | '-' | '.') {
                score += 3;
            }
            prev = Some(i);
            qi += 1;
        }
    }
    if qi != needles.len() {
        return None;
    }

    let base = p.rsplit('/').next().unwrap_or(&p);
    if base.contains(&q) {
        score += 30;
        if base.starts_with(&q) {
            score += 15;
        }
    } else if p.contains(&q) {
        score += 10;
    }
    score -= (path.len() as i32) / 24;
    Some(score)
}

/// Parse every `@`-mention token out of `prompt`.
///
/// A mention is an `@` at the start of the text or after whitespace, followed by a run of
/// non-whitespace characters (the path), with an optional trailing line range introduced by `:` or
/// `#L` (e.g. `@src/main.rs:5-20` or `@src/main.rs#L5-20`). Tokens that are only `@`, or whose path
/// is empty, are ignored. Duplicates (same path and range) are returned once, in first-seen order.
#[must_use]
pub fn parse(prompt: &str) -> Vec<Mention> {
    let chars: Vec<char> = prompt.chars().collect();
    let mut mentions: Vec<Mention> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let at_boundary = i == 0 || chars[i - 1].is_whitespace();
        if chars[i] == '@' && at_boundary {
            let mut j = i + 1;
            while j < chars.len() && !chars[j].is_whitespace() {
                j += 1;
            }
            let token: String = chars[i + 1..j].iter().collect();
            if !token.is_empty() {
                let (path, range) = split_range(&token);
                if !path.is_empty() {
                    let mention = Mention {
                        path: path.to_owned(),
                        range,
                    };
                    if !mentions.contains(&mention) {
                        mentions.push(mention);
                    }
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    mentions
}

/// Split a mention token into its path and an optional line range. Recognizes a trailing `#L<range>`
/// or `:<range>` suffix (where `<range>` is `start-end` or a single `start`); anything else is taken
/// as the whole path with no range.
fn split_range(token: &str) -> (&str, Option<(usize, usize)>) {
    if let Some(idx) = token.rfind("#L") {
        if let Some(range) = parse_range(&token[idx + 2..]) {
            return (&token[..idx], Some(range));
        }
    }
    if let Some(idx) = token.rfind(':') {
        if let Some(range) = parse_range(&token[idx + 1..]) {
            return (&token[..idx], Some(range));
        }
    }
    (token, None)
}

/// Parse a `start-end` (or single `start`) 1-based inclusive line range, or `None` if it is not a
/// well-formed range with `1 <= start <= end`.
fn parse_range(spec: &str) -> Option<(usize, usize)> {
    if let Some((a, b)) = spec.split_once('-') {
        let start: usize = a.parse().ok()?;
        let end: usize = b.parse().ok()?;
        (start >= 1 && end >= start).then_some((start, end))
    } else {
        let n: usize = spec.parse().ok()?;
        (n >= 1).then_some((n, n))
    }
}

/// Expand the `@`-mentions in `prompt` by inlining the referenced files' contents, returning the
/// text to send to the model.
///
/// The original prompt is preserved verbatim; resolvable mentions are appended as a labeled
/// `<attached-files>` block (each with its path and, for a ranged mention, the selected lines). A
/// mention whose file cannot be read is skipped. When no mention resolves to readable content the
/// prompt is returned unchanged, so a plain prompt is never altered.
#[must_use]
pub fn expand(prompt: &str, cwd: &Path) -> String {
    let mentions = parse(prompt);
    if mentions.is_empty() {
        return prompt.to_owned();
    }

    let mut blocks = String::new();
    for mention in &mentions {
        let Some(content) = read_mention(cwd, mention) else {
            continue;
        };
        let label = match mention.range {
            Some((start, end)) => format!("{} (lines {start}-{end})", mention.path),
            None => mention.path.clone(),
        };
        blocks.push_str(&format!("File: {label}\n```\n{content}\n```\n\n"));
    }

    if blocks.is_empty() {
        return prompt.to_owned();
    }
    format!(
        "{prompt}\n\nThe following file(s) were referenced with @-mentions and are provided for \
         context:\n\n<attached-files>\n{blocks}</attached-files>"
    )
}

/// Read a mention's file content relative to `cwd`, applying a line range when present and capping
/// the inlined size. Returns `None` when the file cannot be read.
fn read_mention(cwd: &Path, mention: &Mention) -> Option<String> {
    let content = std::fs::read_to_string(cwd.join(&mention.path)).ok()?;
    let sliced = match mention.range {
        Some((start, end)) => content
            .lines()
            .skip(start - 1)
            .take(end + 1 - start)
            .collect::<Vec<_>>()
            .join("\n"),
        None => content,
    };
    Some(truncate_bytes(sliced))
}

/// Truncate `content` to [`MAX_INLINE_BYTES`] on a char boundary, appending an explicit marker when
/// truncation occurs.
fn truncate_bytes(content: String) -> String {
    if content.len() <= MAX_INLINE_BYTES {
        return content;
    }
    let mut end = MAX_INLINE_BYTES;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &content[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cortex-mention-{tag}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");
        dir
    }

    fn files() -> Vec<String> {
        vec![
            "crates/cli/src/main.rs".to_owned(),
            "crates/tui/src/app.rs".to_owned(),
            "crates/tui/src/mention.rs".to_owned(),
            "README.md".to_owned(),
            "Cargo.toml".to_owned(),
        ]
    }

    #[test]
    fn empty_query_keeps_all_files() {
        // VAL-TUI-049: a bare `@` lists project files (no filtering yet).
        let all = files();
        let got = fuzzy_filter(&all, "", 100);
        assert_eq!(got.len(), all.len());
    }

    #[test]
    fn query_fuzzy_ranks_matching_file_first() {
        // VAL-TUI-050: typing `main` surfaces `main.rs` ranked at the top.
        let all = files();
        let got = fuzzy_filter(&all, "main", 100);
        assert_eq!(got.first().copied(), Some("crates/cli/src/main.rs"));
    }

    #[test]
    fn query_excludes_non_subsequence_matches() {
        let all = files();
        let got = fuzzy_filter(&all, "main", 100);
        assert!(
            !got.contains(&"Cargo.toml"),
            "non-matching file excluded: {got:?}"
        );
    }

    #[test]
    fn filter_respects_the_limit() {
        let all = files();
        let got = fuzzy_filter(&all, "", 2);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn parse_plain_mention() {
        let mentions = parse("look at @src/main.rs please");
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].path, "src/main.rs");
        assert_eq!(mentions[0].range, None);
    }

    #[test]
    fn parse_colon_line_range() {
        // VAL-TUI-052: `@path:start-end` is a single mention token with a line range.
        let mentions = parse("see @src/main.rs:5-20 here");
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].path, "src/main.rs");
        assert_eq!(mentions[0].range, Some((5, 20)));
    }

    #[test]
    fn parse_hash_line_range() {
        let mentions = parse("see @src/main.rs#L5-20 here");
        assert_eq!(mentions[0].path, "src/main.rs");
        assert_eq!(mentions[0].range, Some((5, 20)));
    }

    #[test]
    fn parse_ignores_non_boundary_at() {
        // An `@` that is not at a word boundary (e.g. an email) is not a mention.
        let mentions = parse("mail me at name@example.com");
        assert!(mentions.is_empty(), "got: {mentions:?}");
    }

    #[test]
    fn parse_ignores_bare_at() {
        assert!(parse("just an @ symbol").is_empty());
    }

    #[test]
    fn parse_dedupes_repeats() {
        let mentions = parse("@a.txt and again @a.txt");
        assert_eq!(mentions.len(), 1);
    }

    #[test]
    fn expand_inlines_file_content() {
        // VAL-TUI-054: a mentioned file's contents are provided to the model.
        let dir = temp_dir("expand");
        fs::write(dir.join("note.txt"), "magic token BANANA-42\n").unwrap();
        let expanded = expand("what is in @note.txt ?", &dir);
        assert!(
            expanded.contains("BANANA-42"),
            "content inlined: {expanded}"
        );
        assert!(expanded.contains("what is in @note.txt"), "original kept");
        assert!(expanded.contains("<attached-files>"));
    }

    #[test]
    fn expand_applies_line_range() {
        let dir = temp_dir("range");
        fs::write(dir.join("lines.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        let expanded = expand("@lines.txt:2-3", &dir);
        assert!(expanded.contains("two"));
        assert!(expanded.contains("three"));
        assert!(
            !expanded.contains("five"),
            "out-of-range line excluded: {expanded}"
        );
    }

    #[test]
    fn expand_unchanged_without_mentions() {
        let dir = temp_dir("plain");
        let prompt = "just a normal prompt";
        assert_eq!(expand(prompt, &dir), prompt);
    }

    #[test]
    fn expand_skips_missing_file() {
        let dir = temp_dir("missing");
        let prompt = "look at @does-not-exist.rs";
        assert_eq!(expand(prompt, &dir), prompt);
    }

    #[test]
    fn type_label_maps_common_extensions() {
        // VAL-CONV-039: the `@`-picker meta shows a real type derived from the path.
        assert_eq!(type_label("src/stream_processor.rs"), "rust");
        assert_eq!(type_label("Cargo.toml"), "toml");
        assert_eq!(type_label("README.md"), "markdown");
        assert_eq!(type_label("script.py"), "python");
    }

    #[test]
    fn type_label_falls_back_to_extension_then_file() {
        // An unknown extension is shown verbatim (lowercased); a path with no extension reads `file`.
        assert_eq!(type_label("data.xyz"), "xyz");
        assert_eq!(type_label("Makefile"), "file");
    }

    #[test]
    fn format_size_reads_in_kilobytes_and_up() {
        // VAL-CONV-039: sizes match the mockup column (`6.2 KB`, `0.7 KB`), never raw bytes.
        assert_eq!(format_size(6_320), "6.2 KB");
        assert_eq!(format_size(717), "0.7 KB");
        assert_eq!(format_size(2_100), "2.1 KB");
        assert_eq!(format_size(2 * 1024 * 1024), "2.0 MB");
    }

    #[test]
    fn format_size_compact_uses_perm_mention_units() {
        // VAL-PERM-034: the `@`-mention size column uses the compact `perm_mention.png` units —
        // raw bytes under 1 KiB, then `X.Yk` / `X.YM` / `X.YG` (never the verbose `24.7 KB` form).
        assert_eq!(format_size_compact(912), "912");
        assert_eq!(format_size_compact(4_403), "4.3k");
        assert_eq!(format_size_compact(2_150), "2.1k");
        assert_eq!(format_size_compact(1_258_291), "1.2M");
        assert_eq!(format_size_compact(3 * 1024 * 1024 * 1024), "3.0G");
        // The tight form never falls back to the verbose repo-standard unit.
        assert!(!format_size_compact(4_403).contains("KB"));
    }

    #[test]
    fn list_project_files_finds_created_files() {
        let dir = temp_dir("list");
        fs::write(dir.join("alpha.rs"), "fn a() {}\n").unwrap();
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/beta.rs"), "fn b() {}\n").unwrap();
        let listed = list_project_files(&dir);
        assert!(listed.iter().any(|p| p == "alpha.rs"), "got: {listed:?}");
        assert!(listed.iter().any(|p| p == "sub/beta.rs"), "got: {listed:?}");
    }
}
