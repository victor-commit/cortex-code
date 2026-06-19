//! Anti-rebrand workspace guard (VAL-SKEL-052..056, VAL-SKEL-069).
//!
//! The mission forbids the upstream brand identity from ever appearing in shipped code,
//! prompts, docs, or commit messages. This test enforces that invariant as part of
//! `cargo test --workspace`: it scans every file under `crates/**` (and the shipped `docs/`
//! tree, plus the commit-message history when this is a git checkout) for a case-insensitive
//! match of the forbidden brand string, mirroring `rg -i <brand> crates/` semantics.
//!
//! The needle is assembled from fragments at runtime so this guard file itself contains no
//! contiguous occurrence of the brand and therefore never flags itself (and `rg -i` finds
//! nothing here either). The forbidden URLs/org names (`<brand>.ai`, `anomalyco/<brand>`) both
//! contain the bare brand token, so a single case-insensitive scan for it covers them too.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The forbidden upstream brand token, assembled so it never appears verbatim in this source.
fn brand_needle() -> String {
    ["open", "code"].concat()
}

/// The `crates/` root: the parent of this crate's manifest directory.
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate manifest dir has a parent (the crates/ root)")
        .to_path_buf()
}

/// The workspace root: the parent of `crates/`.
fn workspace_root() -> PathBuf {
    crates_dir()
        .parent()
        .expect("crates/ has a parent (the workspace root)")
        .to_path_buf()
}

/// Recursively collect every regular file under `dir`, skipping build artifacts (`target`) and
/// VCS metadata (`.git`) to match ripgrep's default gitignore-aware traversal.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            let name = entry.file_name();
            if name == "target" || name == ".git" {
                continue;
            }
            collect_files(&entry.path(), out);
        } else if file_type.is_file() {
            out.push(entry.path());
        }
    }
}

/// Case-insensitive (ASCII) byte search; returns the offset of the first match.
fn find_brand(needle: &[u8], haystack: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let lowered: Vec<u8> = haystack.iter().map(u8::to_ascii_lowercase).collect();
    lowered
        .windows(needle.len())
        .position(|window| window == needle)
}

/// 1-based line number containing the byte at `offset`.
fn line_of(haystack: &[u8], offset: usize) -> usize {
    haystack[..offset].iter().filter(|&&b| b == b'\n').count() + 1
}

/// Scan a file tree for the forbidden brand, returning `path:line` for every hit.
fn scan_tree(root: &Path, needle: &[u8]) -> Vec<String> {
    let mut files = Vec::new();
    collect_files(root, &mut files);
    let mut hits = Vec::new();
    for path in &files {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        if let Some(offset) = find_brand(needle, &bytes) {
            hits.push(format!("{}:{}", path.display(), line_of(&bytes, offset)));
        }
    }
    hits
}

// VAL-SKEL-052/053/054/055/056: zero case-insensitive brand hits anywhere under crates/**,
// including comments, embedded prompts, and non-.rs assets. Inserting the brand anywhere here
// makes this fail and names the offending location.
#[test]
fn no_brand_string_under_crates() {
    let needle = brand_needle();
    let crates = crates_dir();
    let mut files = Vec::new();
    collect_files(&crates, &mut files);
    assert!(
        !files.is_empty(),
        "brand-guard scanned zero files under {} (resolution bug)",
        crates.display()
    );

    let hits = scan_tree(&crates, needle.as_bytes());
    assert!(
        hits.is_empty(),
        "brand-guard: forbidden brand string {needle:?} found under crates/ at:\n{}",
        hits.join("\n")
    );
}

// VAL-SKEL-069 (docs portion): the shipped docs tree carries zero brand hits. When no docs tree
// exists yet, there is nothing to guard and the check is trivially satisfied.
#[test]
fn no_brand_string_in_docs() {
    let needle = brand_needle();
    let docs = workspace_root().join("docs");
    if !docs.exists() {
        return;
    }
    let hits = scan_tree(&docs, needle.as_bytes());
    assert!(
        hits.is_empty(),
        "brand-guard: forbidden brand string {needle:?} found under docs/ at:\n{}",
        hits.join("\n")
    );
}

// VAL-SKEL-069 (history portion): commit messages carry zero brand hits. Skips cleanly when this
// is not a git checkout or git is unavailable; the shell validator covers history independently.
#[test]
fn no_brand_string_in_commit_messages() {
    let needle = brand_needle();
    let root = workspace_root();
    if !root.join(".git").exists() {
        return;
    }
    let output = match Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["log", "--format=%B"])
        .output()
    {
        Ok(output) => output,
        Err(_) => return,
    };
    if !output.status.success() {
        return;
    }
    assert!(
        find_brand(needle.as_bytes(), &output.stdout).is_none(),
        "brand-guard: forbidden brand string {needle:?} found in commit-message history"
    );
}
