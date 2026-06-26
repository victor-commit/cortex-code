//! Clean-room unified-diff model and colored rendering for file changes in the transcript.
//!
//! A real turn that edits or creates a file produces a [`FileDiff`]: a file-path header, then one
//! or more hunks of added (`+`), removed (`-`), and unchanged context lines. The diff is computed
//! from the file's before/after content (an LCS line diff trimmed to a few lines of context), and a
//! newly created file (no prior content) renders as an all-added, new-file-indicated diff.
//!
//! Rendering maps each line kind to a distinct semantic color from the active theme palette so
//! added and removed lines remain visually separable (architecture §6 / VAL-TUI-021..023): added
//! success color, removed error color, context dimmed text, and headers in the info accent.

use frame::style::ThemeColors;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::ops::Range;

/// Unchanged context lines kept around each change run.
pub const CONTEXT_LINES: usize = 3;

/// The visual classification of one rendered diff line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffLineKind {
    /// A line present only after the change (`+`).
    Added,
    /// A line present only before the change (`-`).
    Removed,
    /// An unchanged context line.
    Context,
    /// A file-path or `@@` hunk header.
    Header,
}

/// One parsed diff line: its kind plus its text without any `+`/`-` marker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffLine {
    /// The line's visual classification.
    pub kind: DiffLineKind,
    /// The line text, without its `+`/`-`/space marker.
    pub content: String,
}

impl DiffLine {
    fn header(content: impl Into<String>) -> Self {
        Self {
            kind: DiffLineKind::Header,
            content: content.into(),
        }
    }

    fn added(content: impl Into<String>) -> Self {
        Self {
            kind: DiffLineKind::Added,
            content: content.into(),
        }
    }

    fn removed(content: impl Into<String>) -> Self {
        Self {
            kind: DiffLineKind::Removed,
            content: content.into(),
        }
    }

    fn context(content: impl Into<String>) -> Self {
        Self {
            kind: DiffLineKind::Context,
            content: content.into(),
        }
    }
}

/// A computed diff for a single changed file, ready for themed rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiff {
    /// The workspace-relative path of the changed file.
    pub path: String,
    /// Whether this change created the file (rendered as an all-added, new-file diff).
    pub is_new: bool,
    /// Number of added lines.
    pub additions: usize,
    /// Number of removed lines.
    pub removals: usize,
    /// File contents before this change (None when the file was created).
    pub before: Option<String>,
    /// File contents after this change.
    pub after: Option<String>,
    /// Structured hunk data (ranges + body) for navigation and targeted revert.
    pub hunks: Vec<DiffHunk>,
    /// The header + hunk lines, in order.
    pub lines: Vec<DiffLine>,
}

impl FileDiff {
    /// Render the diff as colored ratatui lines using semantic colors from the active theme: a
    /// `+`/`-`/space marker per change line, the file path and `@@` hunks as accent headers, with
    /// added success and removed error colors so the two are visually distinct.
    #[must_use]
    pub fn render(&self) -> Vec<Line<'static>> {
        self.render_with_theme(&ThemeColors::dark())
    }

    /// Render the diff with an explicit theme palette.
    #[must_use]
    pub fn render_with_theme(&self, theme: &ThemeColors) -> Vec<Line<'static>> {
        self.lines
            .iter()
            .map(|line| render_line(line, theme))
            .collect()
    }

    /// The line ranges (in `self.lines`) occupied by each hunk, including that hunk's header.
    #[must_use]
    pub fn hunk_line_ranges(&self) -> Vec<Range<usize>> {
        let mut cursor = 1usize; // 0 = file path header.
        self.hunks
            .iter()
            .map(|hunk| {
                let start = cursor;
                cursor += 1 + hunk.lines.len();
                start..cursor
            })
            .collect()
    }

    /// Split-view rows (before | after) for side-by-side rendering.
    ///
    /// The first row is the file header. Each hunk contributes one header row and then body rows:
    /// context appears in both columns, removals in the left column, additions in the right column.
    #[must_use]
    pub fn split_rows(&self) -> Vec<SplitDiffRow> {
        let mut rows = vec![SplitDiffRow {
            kind: DiffLineKind::Header,
            left: self
                .lines
                .first()
                .map_or(String::new(), |line| line.content.clone()),
            right: String::new(),
        }];
        for hunk in &self.hunks {
            rows.push(SplitDiffRow {
                kind: DiffLineKind::Header,
                left: hunk.header(),
                right: String::new(),
            });
            for line in &hunk.lines {
                let (left, right) = match line.kind {
                    DiffLineKind::Context => (line.content.clone(), line.content.clone()),
                    DiffLineKind::Removed => (line.content.clone(), String::new()),
                    DiffLineKind::Added => (String::new(), line.content.clone()),
                    DiffLineKind::Header => (line.content.clone(), String::new()),
                };
                rows.push(SplitDiffRow {
                    kind: line.kind,
                    left,
                    right,
                });
            }
        }
        rows
    }

    /// The row ranges (in `split_rows`) occupied by each hunk, including that hunk's header.
    #[must_use]
    pub fn split_hunk_row_ranges(&self) -> Vec<Range<usize>> {
        let mut cursor = 1usize; // 0 = file path header.
        self.hunks
            .iter()
            .map(|hunk| {
                let start = cursor;
                cursor += 1 + hunk.lines.len();
                start..cursor
            })
            .collect()
    }
}

/// One unified-diff hunk with structured line ranges and body rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffHunk {
    /// 1-based old-file start line (0 only when old_len is 0).
    pub old_start: usize,
    /// Number of old-file lines in this hunk.
    pub old_len: usize,
    /// 1-based new-file start line (0 only when new_len is 0).
    pub new_start: usize,
    /// Number of new-file lines in this hunk.
    pub new_len: usize,
    /// Body lines (no hunk-header line), in unified order.
    pub lines: Vec<DiffLine>,
    /// The old-side text lines covered by this hunk (context + removed).
    pub old_lines: Vec<String>,
    /// The new-side text lines covered by this hunk (context + added).
    pub new_lines: Vec<String>,
}

impl DiffHunk {
    /// The canonical `@@ -old,len +new,len @@` header for this hunk.
    #[must_use]
    pub fn header(&self) -> String {
        format!(
            "@@ -{},{} +{},{} @@",
            self.old_start, self.old_len, self.new_start, self.new_len
        )
    }
}

/// One split-view row (`left` = old, `right` = new).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitDiffRow {
    /// Semantic row kind used for coloring.
    pub kind: DiffLineKind,
    /// Old-side text for the row.
    pub left: String,
    /// New-side text for the row.
    pub right: String,
}

fn render_line(line: &DiffLine, theme: &ThemeColors) -> Line<'static> {
    match line.kind {
        DiffLineKind::Header => Line::from(Span::styled(
            line.content.clone(),
            Style::default().fg(theme.info).add_modifier(Modifier::BOLD),
        )),
        DiffLineKind::Added => Line::from(Span::styled(
            format!("+ {}", line.content),
            Style::default().fg(theme.success),
        )),
        DiffLineKind::Removed => Line::from(Span::styled(
            format!("- {}", line.content),
            Style::default().fg(theme.error),
        )),
        DiffLineKind::Context => Line::from(Span::styled(
            format!("  {}", line.content),
            Style::default().fg(theme.text_dim),
        )),
    }
}

/// Compute the diff of a single file change.
///
/// `before` is the file's prior content (`None` when the file did not exist, i.e. a creation);
/// `after` is its content after the change (`None` is treated as no renderable change). Returns
/// `None` when there is no textual change to show.
#[must_use]
pub fn compute_file_diff(
    path: &str,
    before: Option<&str>,
    after: Option<&str>,
) -> Option<FileDiff> {
    let after = after?;
    let is_new = before.is_none();
    let before_text = before.unwrap_or("");
    let old_lines = split_lines(before_text);
    let new_lines = split_lines(after);

    let ops = diff_ops(&old_lines, &new_lines);
    let additions = ops.iter().filter(|op| matches!(op, Op::Ins)).count();
    let removals = ops.iter().filter(|op| matches!(op, Op::Del)).count();
    if additions == 0 && removals == 0 {
        return None;
    }

    let hunks = hunk_lines(&old_lines, &new_lines, &ops);
    let mut lines = Vec::new();
    let header = if is_new {
        format!("{path} (new file)")
    } else {
        path.to_owned()
    };
    lines.push(DiffLine::header(header));
    for hunk in &hunks {
        lines.push(DiffLine::header(hunk.header()));
        lines.extend(hunk.lines.iter().cloned());
    }

    Some(FileDiff {
        path: path.to_owned(),
        is_new,
        additions,
        removals,
        before: before.map(str::to_owned),
        after: Some(after.to_owned()),
        hunks,
        lines,
    })
}

/// A single line-level diff operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Op {
    /// The line is unchanged (present in both).
    Keep,
    /// The line was removed (present only before).
    Del,
    /// The line was added (present only after).
    Ins,
}

/// Split content into lines, dropping the trailing empty element a final newline produces so a file
/// ending in `\n` is not reported as having an extra blank line.
fn split_lines(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = content.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// Build the hunk body (with `@@` headers) from the op sequence, keeping up to [`CONTEXT_LINES`]
/// unchanged lines around each change run and merging runs separated by a small gap.
fn hunk_lines(old: &[&str], new: &[&str], ops: &[Op]) -> Vec<DiffHunk> {
    let mut out: Vec<DiffHunk> = Vec::new();
    let mut old_index = 0usize;
    let mut new_index = 0usize;
    let mut cursor = 0usize;

    while cursor < ops.len() {
        if ops[cursor] == Op::Keep {
            old_index += 1;
            new_index += 1;
            cursor += 1;
            continue;
        }

        // Open a hunk with up to CONTEXT_LINES context lines before the first change.
        let context_before = CONTEXT_LINES.min(old_index.min(new_index));
        let hunk_old_start = old_index - context_before;
        let hunk_new_start = new_index - context_before;
        let mut body: Vec<DiffLine> = Vec::new();
        for line in &old[hunk_old_start..old_index] {
            body.push(DiffLine::context(*line));
        }
        let mut hunk_old_len = context_before;
        let mut hunk_new_len = context_before;

        while cursor < ops.len() {
            match ops[cursor] {
                Op::Del => {
                    body.push(DiffLine::removed(old[old_index]));
                    old_index += 1;
                    hunk_old_len += 1;
                    cursor += 1;
                }
                Op::Ins => {
                    body.push(DiffLine::added(new[new_index]));
                    new_index += 1;
                    hunk_new_len += 1;
                    cursor += 1;
                }
                Op::Keep => {
                    // Close the hunk if the next change is farther than 2*context away (otherwise
                    // merge through the gap so nearby changes stay in one readable hunk).
                    let gap = ops[cursor..]
                        .iter()
                        .take_while(|op| matches!(op, Op::Keep))
                        .count();
                    if gap > CONTEXT_LINES * 2 || cursor + gap == ops.len() {
                        let keep = gap.min(CONTEXT_LINES);
                        for line in &old[old_index..old_index + keep] {
                            body.push(DiffLine::context(*line));
                        }
                        old_index += gap;
                        new_index += gap;
                        hunk_old_len += keep;
                        hunk_new_len += keep;
                        cursor += gap;
                        break;
                    }
                    body.push(DiffLine::context(old[old_index]));
                    old_index += 1;
                    new_index += 1;
                    hunk_old_len += 1;
                    hunk_new_len += 1;
                    cursor += 1;
                }
            }
        }

        let old_display_start = if hunk_old_len == 0 {
            0
        } else {
            hunk_old_start + 1
        };
        let new_display_start = if hunk_new_len == 0 {
            0
        } else {
            hunk_new_start + 1
        };
        let old_lines = body
            .iter()
            .filter(|line| line.kind != DiffLineKind::Added)
            .map(|line| line.content.clone())
            .collect();
        let new_lines = body
            .iter()
            .filter(|line| line.kind != DiffLineKind::Removed)
            .map(|line| line.content.clone())
            .collect();
        out.push(DiffHunk {
            old_start: old_display_start,
            old_len: hunk_old_len,
            new_start: new_display_start,
            new_len: hunk_new_len,
            lines: body,
            old_lines,
            new_lines,
        });
    }

    out
}

/// Produce the op sequence diffing `old` -> `new`, trimming the common prefix/suffix first and
/// degrading an oversized middle to a whole-region replace so the result stays bounded.
fn diff_ops(old: &[&str], new: &[&str]) -> Vec<Op> {
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];

    let mut ops = vec![Op::Keep; prefix];
    if old_mid.len().saturating_mul(new_mid.len()) > 4_000_000 {
        ops.extend(std::iter::repeat_n(Op::Del, old_mid.len()));
        ops.extend(std::iter::repeat_n(Op::Ins, new_mid.len()));
    } else {
        ops.extend(lcs_ops(old_mid, new_mid));
    }
    ops.extend(std::iter::repeat_n(Op::Keep, suffix));
    ops
}

/// The longest-common-subsequence op sequence over the changed middle.
fn lcs_ops(old: &[&str], new: &[&str]) -> Vec<Op> {
    let rows = old.len();
    let cols = new.len();
    let mut table = vec![vec![0usize; cols + 1]; rows + 1];
    for i in (0..rows).rev() {
        for j in (0..cols).rev() {
            table[i][j] = if old[i] == new[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut ops = Vec::with_capacity(rows + cols);
    let (mut i, mut j) = (0, 0);
    while i < rows && j < cols {
        if old[i] == new[j] {
            ops.push(Op::Keep);
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            ops.push(Op::Del);
            i += 1;
        } else {
            ops.push(Op::Ins);
            j += 1;
        }
    }
    ops.extend(std::iter::repeat_n(Op::Del, rows - i));
    ops.extend(std::iter::repeat_n(Op::Ins, cols - j));
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(diff: &FileDiff, kind: DiffLineKind) -> Vec<&str> {
        diff.lines
            .iter()
            .filter(|l| l.kind == kind)
            .map(|l| l.content.as_str())
            .collect()
    }

    #[test]
    fn identical_content_yields_no_diff() {
        assert!(compute_file_diff("f.txt", Some("a\nb\n"), Some("a\nb\n")).is_none());
    }

    #[test]
    fn edit_shows_path_header_context_and_markers() {
        let before = "fn main() {\n    println!(\"Hello\");\n    let x = 1;\n}\n";
        let after = "fn main() {\n    println!(\"Cortex\");\n    let x = 1;\n}\n";
        let diff = compute_file_diff("src/main.rs", Some(before), Some(after)).expect("a diff");

        assert!(!diff.is_new);
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.removals, 1);

        // The file-path header is present (VAL-TUI-022).
        assert_eq!(diff.lines[0].kind, DiffLineKind::Header);
        assert_eq!(diff.lines[0].content, "src/main.rs");

        // The removed/added pair is captured with the changed text.
        assert_eq!(
            kinds(&diff, DiffLineKind::Removed),
            vec!["    println!(\"Hello\");"]
        );
        assert_eq!(
            kinds(&diff, DiffLineKind::Added),
            vec!["    println!(\"Cortex\");"]
        );

        // At least one unchanged context line surrounds the edit (VAL-TUI-022).
        let context = kinds(&diff, DiffLineKind::Context);
        assert!(
            context.contains(&"fn main() {"),
            "expected a leading context line: {context:?}"
        );
        assert!(
            context.contains(&"    let x = 1;"),
            "expected a trailing context line: {context:?}"
        );
    }

    #[test]
    fn new_file_is_all_added_with_new_file_indicator() {
        let content = "line one\nline two\nline three\n";
        let diff = compute_file_diff("notes.txt", None, Some(content)).expect("a diff");

        assert!(diff.is_new);
        assert_eq!(diff.removals, 0);
        assert_eq!(diff.additions, 3);

        // The header marks the file as new (VAL-TUI-023).
        assert_eq!(diff.lines[0].kind, DiffLineKind::Header);
        assert!(
            diff.lines[0].content.contains("notes.txt") && diff.lines[0].content.contains("new"),
            "new-file header should name the file and indicate it is new: {}",
            diff.lines[0].content
        );

        // Every change line is an addition; none are removals.
        assert_eq!(
            kinds(&diff, DiffLineKind::Added),
            vec!["line one", "line two", "line three"]
        );
        assert!(kinds(&diff, DiffLineKind::Removed).is_empty());
    }

    #[test]
    fn render_colors_added_and_removed_lines_distinctly() {
        let before = "alpha\nbeta\ngamma\n";
        let after = "alpha\nBETA\ngamma\n";
        let diff = compute_file_diff("f.txt", Some(before), Some(after)).expect("a diff");
        let theme = ThemeColors::light();
        let rendered = diff.render_with_theme(&theme);

        let mut added_color = None;
        let mut removed_color = None;
        for line in &rendered {
            let span = &line.spans[0];
            let text = span.content.as_ref();
            if text.starts_with("+ ") {
                added_color = span.style.fg;
            } else if text.starts_with("- ") {
                removed_color = span.style.fg;
            }
        }

        assert_eq!(added_color, Some(theme.success), "added lines use success");
        assert_eq!(removed_color, Some(theme.error), "removed lines use error");
        assert_ne!(
            added_color, removed_color,
            "added and removed must be distinctly colored"
        );
    }

    #[test]
    fn handles_pure_insertion_into_an_existing_file() {
        let before = "a\nb\nc\n";
        let after = "a\nb\ninserted\nc\n";
        let diff = compute_file_diff("f.txt", Some(before), Some(after)).expect("a diff");
        assert_eq!(diff.additions, 1);
        assert_eq!(diff.removals, 0);
        assert_eq!(kinds(&diff, DiffLineKind::Added), vec!["inserted"]);
        // Context anchors the insertion.
        assert!(!kinds(&diff, DiffLineKind::Context).is_empty());
    }

    #[test]
    fn hunk_ranges_align_with_rendered_lines() {
        let before = "one\ntwo\nthree\nfour\nfive\n";
        let after = "ONE\ntwo\nthree\nFOUR\nfive\n";
        let diff = compute_file_diff("f.txt", Some(before), Some(after)).expect("a diff");
        let ranges = diff.hunk_line_ranges();
        assert!(!ranges.is_empty());
        for range in ranges {
            assert!(
                range.start < range.end && range.end <= diff.lines.len(),
                "hunk range should index rendered lines: {range:?} vs {}",
                diff.lines.len()
            );
            assert_eq!(diff.lines[range.start].kind, DiffLineKind::Header);
        }
    }

    #[test]
    fn split_rows_place_additions_and_removals_in_separate_columns() {
        let before = "alpha\nbeta\ngamma\n";
        let after = "alpha\nBETA\ngamma\n";
        let diff = compute_file_diff("f.txt", Some(before), Some(after)).expect("a diff");
        let rows = diff.split_rows();
        let removed = rows
            .iter()
            .find(|row| row.kind == DiffLineKind::Removed)
            .expect("a removed row");
        assert!(!removed.left.is_empty());
        assert!(removed.right.is_empty());
        let added = rows
            .iter()
            .find(|row| row.kind == DiffLineKind::Added)
            .expect("an added row");
        assert!(added.left.is_empty());
        assert!(!added.right.is_empty());
    }
}
