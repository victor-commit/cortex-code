//! Parse the `apply_patch` envelope into structured file hunks.
//!
//! The patch language is a stripped-down, file-oriented diff wrapped in a `*** Begin Patch` /
//! `*** End Patch` envelope. Inside the envelope is a sequence of file sections, each introduced by
//! one of three headers:
//!
//! ```text
//! *** Begin Patch
//! *** Add File: path        # every following `+` line is the new file's content
//! +first line
//! +second line
//! *** Update File: path     # context-anchored @@ hunks patch the file in place
//! *** Move to: new/path     # optional rename
//! @@ optional context
//!  unchanged context line
//! -removed line
//! +added line
//! *** Delete File: path     # remove the file (nothing follows)
//! *** End Patch
//! ```
//!
//! Parsing is purely textual: it never touches the filesystem. A structurally invalid envelope
//! resolves to an [`Err`] carrying a clear, model-actionable message; applying the parsed hunks is
//! the [`super::apply`] module's job.

const BEGIN_MARKER: &str = "*** Begin Patch";
const END_MARKER: &str = "*** End Patch";
const ADD_MARKER: &str = "*** Add File: ";
const DELETE_MARKER: &str = "*** Delete File: ";
const UPDATE_MARKER: &str = "*** Update File: ";
const MOVE_MARKER: &str = "*** Move to: ";
const EOF_MARKER: &str = "*** End of File";

/// Whether a stripped hunk-header label is a positional line range (e.g. `-1,1 +1,1` or `-2 +2`)
/// rather than a real context line: such a label only carries digits, commas, spaces, and `+`/`-`,
/// so it cannot anchor against file content and is treated as no context.
fn is_line_range(label: &str) -> bool {
    !label.is_empty()
        && label
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, ',' | ' ' | '-' | '+'))
}

/// A single file operation parsed from the envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hunk {
    /// Create a new file whose entire content is the section's `+` lines.
    Add {
        /// The path of the file to create.
        path: String,
        /// The full content of the new file (each `+` line plus a trailing newline).
        contents: String,
    },
    /// Remove an existing file.
    Delete {
        /// The path of the file to remove.
        path: String,
    },
    /// Patch an existing file in place, optionally renaming it.
    Update {
        /// The path of the file to patch.
        path: String,
        /// The destination path when the section also renames the file.
        move_to: Option<String>,
        /// The ordered context-anchored change regions to apply.
        chunks: Vec<Chunk>,
    },
}

/// One contiguous change region within an [`Hunk::Update`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The `@@`-anchored context line used to narrow the region's location, if provided.
    pub context: Option<String>,
    /// The lines the region must currently contain (context lines plus removed lines), in order.
    pub old_lines: Vec<String>,
    /// The lines the region is replaced with (context lines plus added lines), in order.
    pub new_lines: Vec<String>,
    /// Whether the region is anchored to the end of the file (`*** End of File`).
    pub is_eof: bool,
}

/// Parse the full envelope into its ordered hunks, or a clear parse error.
///
/// # Errors
/// Returns a descriptive message when the envelope is missing its `*** Begin Patch`/`*** End Patch`
/// markers, contains an unrecognized hunk header, or contains a malformed update hunk.
pub fn parse_envelope(patch: &str) -> Result<Vec<Hunk>, String> {
    let lines: Vec<&str> = patch.trim().lines().collect();
    if lines.first().map(|line| line.trim()) != Some(BEGIN_MARKER) {
        return Err(format!("the patch must begin with '{BEGIN_MARKER}'"));
    }
    if lines.len() < 2 || lines.last().map(|line| line.trim()) != Some(END_MARKER) {
        return Err(format!("the patch must end with '{END_MARKER}'"));
    }

    let body = &lines[1..lines.len() - 1];
    let mut hunks = Vec::new();
    let mut index = 0;
    while index < body.len() {
        // Skip blank separator lines between file sections.
        if body[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let (hunk, consumed) = parse_hunk(&body[index..])?;
        hunks.push(hunk);
        index += consumed;
    }
    Ok(hunks)
}

/// Parse one file section from the front of `lines`, returning the hunk and the number of lines it
/// consumed.
fn parse_hunk(lines: &[&str]) -> Result<(Hunk, usize), String> {
    let header = lines[0].trim();

    if let Some(path) = header.strip_prefix(ADD_MARKER) {
        let path = path.trim().to_owned();
        if path.is_empty() {
            return Err(format!("'{}' requires a path", ADD_MARKER.trim()));
        }
        let mut contents = String::new();
        let mut consumed = 1;
        for line in &lines[1..] {
            let Some(rest) = line.strip_prefix('+') else {
                break;
            };
            contents.push_str(rest);
            contents.push('\n');
            consumed += 1;
        }
        return Ok((Hunk::Add { path, contents }, consumed));
    }

    if let Some(path) = header.strip_prefix(DELETE_MARKER) {
        let path = path.trim().to_owned();
        if path.is_empty() {
            return Err(format!("'{}' requires a path", DELETE_MARKER.trim()));
        }
        return Ok((Hunk::Delete { path }, 1));
    }

    if let Some(path) = header.strip_prefix(UPDATE_MARKER) {
        let path = path.trim().to_owned();
        if path.is_empty() {
            return Err(format!("'{}' requires a path", UPDATE_MARKER.trim()));
        }
        let mut consumed = 1;
        let move_to = lines
            .get(consumed)
            .and_then(|line| line.trim().strip_prefix(MOVE_MARKER))
            .map(|dest| dest.trim().to_owned());
        if move_to.is_some() {
            consumed += 1;
        }

        let mut chunks = Vec::new();
        while consumed < lines.len() {
            let line = lines[consumed];
            if line.trim().is_empty() {
                consumed += 1;
                continue;
            }
            // A `***` marker starts the next file section, ending this update hunk.
            if line.starts_with('*') {
                break;
            }
            let (chunk, used) = parse_chunk(&lines[consumed..], chunks.is_empty())?;
            chunks.push(chunk);
            consumed += used;
        }

        if chunks.is_empty() {
            return Err(format!("update hunk for '{path}' contains no changes"));
        }
        return Ok((
            Hunk::Update {
                path,
                move_to,
                chunks,
            },
            consumed,
        ));
    }

    Err(format!(
        "'{header}' is not a valid hunk header; expected '{}', '{}', or '{}'",
        ADD_MARKER.trim(),
        DELETE_MARKER.trim(),
        UPDATE_MARKER.trim()
    ))
}

/// Parse one change region from the front of an update hunk's lines.
///
/// `allow_missing_context` permits the first chunk of an update hunk to omit the leading `@@`
/// marker (a common, lenient shape); a later chunk must start with `@@`.
fn parse_chunk(lines: &[&str], allow_missing_context: bool) -> Result<(Chunk, usize), String> {
    let first = lines[0].trim();
    let (context, start) = if first.starts_with('@') {
        // A hunk header: the canonical `@@`/`@@ context` form, or a unified-diff `@ -1,1 +1,1` /
        // `@@ -1,1 +1,1 @@` form some models emit. Strip the `@` runs and use the remaining label as
        // the context anchor, unless it is empty or a positional line range (which is no anchor).
        let label = first.trim_matches('@').trim();
        let context = if label.is_empty() || is_line_range(label) {
            None
        } else {
            Some(label.to_owned())
        };
        (context, 1)
    } else if allow_missing_context {
        (None, 0)
    } else {
        return Err(format!(
            "expected an '@@' context marker, found '{}'",
            lines[0]
        ));
    };

    if start >= lines.len() {
        return Err("update hunk contains a context marker with no following lines".to_owned());
    }

    let mut chunk = Chunk {
        context,
        old_lines: Vec::new(),
        new_lines: Vec::new(),
        is_eof: false,
    };
    let mut index = start;
    let mut parsed = 0;
    while index < lines.len() {
        let line = lines[index];
        if line.trim() == EOF_MARKER {
            if parsed == 0 {
                return Err("update hunk contains no change lines".to_owned());
            }
            chunk.is_eof = true;
            index += 1;
            break;
        }
        match line.chars().next() {
            // An empty line is a blank context line present in both sides.
            None => {
                chunk.old_lines.push(String::new());
                chunk.new_lines.push(String::new());
            }
            Some(' ') => {
                let content = line[1..].to_owned();
                chunk.old_lines.push(content.clone());
                chunk.new_lines.push(content);
            }
            Some('+') => chunk.new_lines.push(line[1..].to_owned()),
            Some('-') => chunk.old_lines.push(line[1..].to_owned()),
            // Any other leading character ends this chunk: it is either the next `@@` region or the
            // next `***` file section. A chunk with no change lines yet is malformed.
            Some(_) => {
                if parsed == 0 {
                    return Err(format!(
                        "unexpected line in update hunk: '{line}'; change lines must start with \
                         ' ' (context), '+' (added), or '-' (removed)"
                    ));
                }
                break;
            }
        }
        parsed += 1;
        index += 1;
    }

    Ok((chunk, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_a_patch_without_the_begin_marker() {
        let err = parse_envelope("*** Add File: a\n+x\n*** End Patch").unwrap_err();
        assert!(err.contains("Begin Patch"), "got: {err}");
    }

    #[test]
    fn rejects_a_patch_without_the_end_marker() {
        let err = parse_envelope("*** Begin Patch\n*** Add File: a\n+x").unwrap_err();
        assert!(err.contains("End Patch"), "got: {err}");
    }

    #[test]
    fn parses_an_add_file_section_with_exact_lines() {
        let hunks =
            parse_envelope("*** Begin Patch\n*** Add File: new.py\n+line1\n+line2\n*** End Patch")
                .unwrap();
        assert_eq!(
            hunks,
            vec![Hunk::Add {
                path: "new.py".to_owned(),
                contents: "line1\nline2\n".to_owned(),
            }]
        );
    }

    #[test]
    fn parses_a_delete_file_section() {
        let hunks =
            parse_envelope("*** Begin Patch\n*** Delete File: old.txt\n*** End Patch").unwrap();
        assert_eq!(
            hunks,
            vec![Hunk::Delete {
                path: "old.txt".to_owned(),
            }]
        );
    }

    #[test]
    fn parses_an_update_section_with_context_and_changes() {
        let patch = "*** Begin Patch\n*** Update File: app.py\n@@ def greet():\n-    print(\"Hi\")\n+    print(\"Hello\")\n*** End Patch";
        let hunks = parse_envelope(patch).unwrap();
        assert_eq!(
            hunks,
            vec![Hunk::Update {
                path: "app.py".to_owned(),
                move_to: None,
                chunks: vec![Chunk {
                    context: Some("def greet():".to_owned()),
                    old_lines: vec!["    print(\"Hi\")".to_owned()],
                    new_lines: vec!["    print(\"Hello\")".to_owned()],
                    is_eof: false,
                }],
            }]
        );
    }

    #[test]
    fn parses_an_update_section_with_a_rename() {
        let patch = "*** Begin Patch\n*** Update File: a.py\n*** Move to: b.py\n@@\n-old\n+new\n*** End Patch";
        let hunks = parse_envelope(patch).unwrap();
        assert_eq!(
            hunks,
            vec![Hunk::Update {
                path: "a.py".to_owned(),
                move_to: Some("b.py".to_owned()),
                chunks: vec![Chunk {
                    context: None,
                    old_lines: vec!["old".to_owned()],
                    new_lines: vec!["new".to_owned()],
                    is_eof: false,
                }],
            }]
        );
    }

    #[test]
    fn parses_a_multi_file_envelope_in_order() {
        let patch = "*** Begin Patch\n*** Add File: a.py\n+abc\n*** Delete File: b.py\n*** Update File: c.py\n@@\n-old\n+new\n*** End Patch";
        let hunks = parse_envelope(patch).unwrap();
        assert_eq!(hunks.len(), 3);
        assert!(matches!(hunks[0], Hunk::Add { .. }));
        assert!(matches!(hunks[1], Hunk::Delete { .. }));
        assert!(matches!(hunks[2], Hunk::Update { .. }));
    }

    #[test]
    fn parses_an_update_first_chunk_without_an_explicit_context_marker() {
        let patch = "*** Begin Patch\n*** Update File: f.py\n import foo\n+bar\n*** End Patch";
        let hunks = parse_envelope(patch).unwrap();
        assert_eq!(
            hunks,
            vec![Hunk::Update {
                path: "f.py".to_owned(),
                move_to: None,
                chunks: vec![Chunk {
                    context: None,
                    old_lines: vec!["import foo".to_owned()],
                    new_lines: vec!["import foo".to_owned(), "bar".to_owned()],
                    is_eof: false,
                }],
            }]
        );
    }

    #[test]
    fn rejects_an_unknown_hunk_header() {
        let err = parse_envelope("*** Begin Patch\nnonsense\n*** End Patch").unwrap_err();
        assert!(err.contains("not a valid hunk header"), "got: {err}");
    }

    #[test]
    fn rejects_an_empty_update_section() {
        let err =
            parse_envelope("*** Begin Patch\n*** Update File: t.py\n*** End Patch").unwrap_err();
        assert!(err.contains("no changes"), "got: {err}");
    }

    #[test]
    fn tolerates_a_unified_diff_style_hunk_header() {
        // Some models emit a `@ -1,1 +1,1` (or `@@ -1,1 +1,1 @@`) header instead of `@@ context`.
        // The positional range is not a usable anchor, so the chunk parses with no context and the
        // change lines are read normally.
        let patch = "*** Begin Patch\n*** Update File: f.py\n @ -1,1 +1,1\n-line two\n+CHANGED\n*** End Patch";
        let hunks = parse_envelope(patch).unwrap();
        assert_eq!(
            hunks,
            vec![Hunk::Update {
                path: "f.py".to_owned(),
                move_to: None,
                chunks: vec![Chunk {
                    context: None,
                    old_lines: vec!["line two".to_owned()],
                    new_lines: vec!["CHANGED".to_owned()],
                    is_eof: false,
                }],
            }]
        );
    }

    #[test]
    fn classifies_line_range_labels() {
        assert!(is_line_range("-1,1 +1,1"));
        assert!(is_line_range("-2 +2"));
        assert!(!is_line_range("def greet():"));
        assert!(!is_line_range(""));
    }

    #[test]
    fn parses_an_end_of_file_marker() {
        let patch =
            "*** Begin Patch\n*** Update File: f.py\n@@\n+line\n*** End of File\n*** End Patch";
        let hunks = parse_envelope(patch).unwrap();
        let Hunk::Update { chunks, .. } = &hunks[0] else {
            panic!("expected an update hunk");
        };
        assert!(chunks[0].is_eof);
        assert_eq!(chunks[0].new_lines, vec!["line".to_owned()]);
    }
}
