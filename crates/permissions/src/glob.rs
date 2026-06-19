//! The anchored glob matcher used to match a rule pattern against an action's subject.
//!
//! Blueprint: the durable runner's permission layer (architecture §4) classifies a gated action
//! by matching the rule pattern against the action's subject (a file path for file tools, the
//! parsed command for `bash`). The matcher is deliberately small and subject-agnostic: deciding
//! *what* the subject is for each tool is the scoping layer's job; this module only answers
//! "does this pattern match this string".
//!
//! Semantics (anchored at both ends, case-sensitive):
//! - `*` matches any run of characters, including the empty run and path separators, so a single
//!   `*` matches everything and `src/**` matches both `src/a.txt` and `src/a/b.txt`.
//! - `?` matches exactly one character.
//! - every other character matches itself literally.

/// Return whether `pattern` matches the whole of `text` under the anchored glob semantics above.
#[must_use]
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();

    let mut pattern_index = 0usize;
    let mut text_index = 0usize;
    // The most recent `*` position and the text position it started consuming from, so a failed
    // match can backtrack to let the star swallow one more character.
    let mut star_pattern: Option<usize> = None;
    let mut star_text = 0usize;

    while text_index < text.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == text[text_index])
        {
            pattern_index += 1;
            text_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star_pattern = Some(pattern_index);
            star_text = text_index;
            pattern_index += 1;
        } else if let Some(star) = star_pattern {
            pattern_index = star + 1;
            star_text += 1;
            text_index = star_text;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_match_is_anchored_both_ends() {
        assert!(glob_match("rm", "rm"));
        assert!(!glob_match("rm", "rm x"));
        assert!(!glob_match("rm", "trm"));
    }

    #[test]
    fn bare_star_matches_anything_including_empty() {
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "rm x"));
        assert!(glob_match("*", "echo hi"));
    }

    #[test]
    fn star_matches_a_trailing_run() {
        assert!(glob_match("rm *", "rm target.txt"));
        assert!(glob_match("git status*", "git status --short"));
        assert!(!glob_match("git push*", "git status"));
    }

    #[test]
    fn star_spans_path_separators() {
        assert!(glob_match("src/**", "src/a.txt"));
        assert!(glob_match("src/**", "src/nested/dir/a.txt"));
        assert!(glob_match("src/*", "src/nested/dir/a.txt"));
        assert!(!glob_match("src/**", "secrets/a.txt"));
    }

    #[test]
    fn question_mark_matches_exactly_one_char() {
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(!glob_match("a?c", "abbc"));
    }

    #[test]
    fn multiple_stars_backtrack_correctly() {
        assert!(glob_match("*push*", "git push origin main"));
        assert!(!glob_match("*push*", "git status"));
    }
}
