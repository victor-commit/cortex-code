//! Apply parsed update chunks to a file's content, in memory.
//!
//! This module is the pure transformation half of `apply_patch`: given a file's current text and an
//! ordered list of [`Chunk`]s, it produces the patched text or a clear error when a chunk's context
//! cannot be located. It never touches the filesystem, so it is exhaustively unit-testable.
//!
//! Hunk context is located with decreasing strictness so a hunk still applies when its context
//! lines drift in whitespace/indentation (validation contract VAL-TOOLS-025): an exact line match
//! is tried first, then a match ignoring trailing whitespace, then one ignoring leading and
//! trailing whitespace. The replacement text is inserted verbatim, so only the targeted region
//! changes and untargeted lines are preserved.

use super::parser::Chunk;

/// Apply `chunks` to `original`, returning the patched content or a clear error.
///
/// # Errors
/// Returns a descriptive message when a chunk's `@@` context or its old lines cannot be located in
/// `original`, even after whitespace-tolerant matching.
pub fn apply_chunks(original: &str, chunks: &[Chunk]) -> Result<String, String> {
    let mut lines: Vec<String> = original.split('\n').map(str::to_owned).collect();
    // `split('\n')` yields a trailing empty element for a file ending in a newline; drop it so line
    // indices match a conventional line-oriented diff. It is restored when rejoining.
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    // Compute every replacement as (start_index, removed_len, inserted_lines) before mutating, so a
    // later chunk is located against the unmodified line indices.
    let mut replacements: Vec<(usize, usize, Vec<String>)> = Vec::new();
    let mut cursor = 0usize;

    for chunk in chunks {
        // Anchor on the `@@` context when present. If it cannot be located the change lines still
        // anchor the region, so a missing/mis-extracted context is a soft fallback rather than a
        // hard failure — but a pure insertion has no other anchor, so it must error.
        let mut anchored = false;
        if let Some(context) = &chunk.context {
            if let Some(idx) = seek(&lines, std::slice::from_ref(context), cursor, false) {
                cursor = idx + 1;
                anchored = true;
            } else if chunk.old_lines.is_empty() {
                return Err(format!("could not find context line '{context}'"));
            }
        }

        if chunk.old_lines.is_empty() {
            // A pure insertion: place the new lines just after the matched context, or at the end of
            // the file when the chunk carried no usable context.
            let at = if anchored { cursor } else { lines.len() };
            replacements.push((at, 0, chunk.new_lines.clone()));
            continue;
        }

        let mut pattern: &[String] = &chunk.old_lines;
        let mut insert: &[String] = &chunk.new_lines;
        let mut found = seek(&lines, pattern, cursor, chunk.is_eof);

        // The old-line block sometimes ends with an empty sentinel standing for the file's final
        // newline, which is not present in `lines`. Retry without it (and its paired new sentinel).
        if found.is_none() && pattern.last().is_some_and(String::is_empty) {
            pattern = &pattern[..pattern.len() - 1];
            if insert.last().is_some_and(String::is_empty) {
                insert = &insert[..insert.len() - 1];
            }
            found = seek(&lines, pattern, cursor, chunk.is_eof);
        }

        match found {
            Some(start) => {
                replacements.push((start, pattern.len(), insert.to_vec()));
                cursor = start + pattern.len();
            }
            None => {
                return Err(format!(
                    "could not find the lines to change:\n{}",
                    chunk.old_lines.join("\n")
                ));
            }
        }
    }

    replacements.sort_by_key(|(start, _, _)| *start);
    // Apply in descending order so an earlier replacement does not shift the indices of later ones.
    for (start, removed, inserted) in replacements.iter().rev() {
        for _ in 0..*removed {
            if *start < lines.len() {
                lines.remove(*start);
            }
        }
        for (offset, line) in inserted.iter().enumerate() {
            lines.insert(start + offset, line.clone());
        }
    }

    // Restore the trailing newline so the patched file ends in one, matching diff conventions.
    if !lines.last().is_some_and(String::is_empty) {
        lines.push(String::new());
    }
    Ok(lines.join("\n"))
}

/// Find the start index of `pattern` within `lines` at or after `start`, matching with decreasing
/// strictness so whitespace/indentation drift in the context does not defeat the search. When `eof`
/// is set the search prefers a match anchored at the end of the file.
fn seek(lines: &[String], pattern: &[String], start: usize, eof: bool) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if pattern.len() > lines.len() {
        return None;
    }
    let last = lines.len() - pattern.len();
    let search_start = if eof { last } else { start };

    for strictness in [Match::Exact, Match::TrimEnd, Match::Trim] {
        for i in search_start..=last {
            if pattern
                .iter()
                .enumerate()
                .all(|(offset, want)| strictness.eq(&lines[i + offset], want))
            {
                return Some(i);
            }
        }
    }
    None
}

/// How strictly two lines are compared when locating a hunk's context.
#[derive(Clone, Copy)]
enum Match {
    /// Byte-for-byte equality.
    Exact,
    /// Equality ignoring trailing whitespace.
    TrimEnd,
    /// Equality ignoring leading and trailing whitespace (tolerates indentation drift).
    Trim,
}

impl Match {
    fn eq(self, line: &str, want: &str) -> bool {
        match self {
            Match::Exact => line == want,
            Match::TrimEnd => line.trim_end() == want.trim_end(),
            Match::Trim => line.trim() == want.trim(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(context: Option<&str>, old: &[&str], new: &[&str]) -> Chunk {
        Chunk {
            context: context.map(str::to_owned),
            old_lines: old.iter().map(|s| (*s).to_owned()).collect(),
            new_lines: new.iter().map(|s| (*s).to_owned()).collect(),
            is_eof: false,
        }
    }

    #[test]
    fn replaces_a_single_line_and_leaves_the_rest() {
        let original = "one\ntwo\nthree\n";
        let patched = apply_chunks(original, &[chunk(None, &["two"], &["TWO"])]).unwrap();
        assert_eq!(patched, "one\nTWO\nthree\n");
    }

    #[test]
    fn applies_a_context_anchored_replacement_and_insertion() {
        let original = "def greet():\n    print(\"Hi\")\n    return\n";
        let patched = apply_chunks(
            original,
            &[chunk(
                Some("def greet():"),
                &["    print(\"Hi\")"],
                &["    print(\"Hello\")", "    print(\"world\")"],
            )],
        )
        .unwrap();
        assert_eq!(
            patched,
            "def greet():\n    print(\"Hello\")\n    print(\"world\")\n    return\n"
        );
    }

    #[test]
    fn fuzzy_matches_context_with_drifted_indentation() {
        // The on-disk line is deeply indented; the hunk's old line uses no indentation. The trim
        // pass still locates the region and applies the verbatim replacement.
        let original = "fn main() {\n        let x = 1;\n}\n";
        let patched = apply_chunks(
            original,
            &[chunk(None, &["let x = 1;"], &["    let y = 2;"])],
        )
        .unwrap();
        assert_eq!(patched, "fn main() {\n    let y = 2;\n}\n");
    }

    #[test]
    fn errors_when_a_pure_insertion_context_is_not_found() {
        // A pure insertion (no old lines) has only the context to anchor on, so a missing context is
        // a hard failure rather than a silent append.
        let original = "alpha\nbeta\n";
        let err = apply_chunks(original, &[chunk(Some("missing"), &[], &["NEW"])]).unwrap_err();
        assert!(err.contains("could not find context"), "got: {err}");
    }

    #[test]
    fn falls_back_to_old_lines_when_context_is_not_found() {
        // The context anchor does not exist in the file, but the removed line does: the change still
        // applies by anchoring on the old line rather than hard-failing on the context.
        let original = "alpha\nbeta\ngamma\n";
        let patched =
            apply_chunks(original, &[chunk(Some("missing context"), &["beta"], &["BETA"])]).unwrap();
        assert_eq!(patched, "alpha\nBETA\ngamma\n");
    }

    #[test]
    fn errors_when_old_lines_are_not_found() {
        let original = "alpha\nbeta\n";
        let err = apply_chunks(original, &[chunk(None, &["gamma"], &["GAMMA"])]).unwrap_err();
        assert!(err.contains("could not find the lines"), "got: {err}");
    }

    #[test]
    fn deletes_a_line_with_an_empty_replacement() {
        let original = "keep\ndrop\nkeep2\n";
        let patched = apply_chunks(original, &[chunk(None, &["drop"], &[])]).unwrap();
        assert_eq!(patched, "keep\nkeep2\n");
    }

    #[test]
    fn untargeted_lines_are_unchanged() {
        let original = "a\nb\nc\nd\ne\n";
        let patched = apply_chunks(original, &[chunk(None, &["c"], &["C"])]).unwrap();
        assert_eq!(patched, "a\nb\nC\nd\ne\n");
    }
}
