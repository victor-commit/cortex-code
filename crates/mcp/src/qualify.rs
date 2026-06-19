//! Deterministic, sanitized, server-qualified identifiers for MCP tools.
//!
//! Every MCP tool is exposed to the model under an id that combines its server name and tool name,
//! so two servers each exporting a tool named `search` yield two distinct, separately-callable ids
//! (`a_search`, `b_search`) with no clash. Each component is sanitized to the tool-name charset
//! (ASCII alphanumerics, `_`, `-`) so the id is always a legal function-call name regardless of the
//! raw server/tool names. The mapping is stable: the same (server, tool) pair always yields the
//! same id.

/// The characters allowed in a tool identifier; anything else is replaced with `_`.
fn sanitize(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "_".to_owned()
    } else {
        cleaned
    }
}

/// The base server-qualified id for `tool` exported by `server`: `<server>_<tool>`, each component
/// sanitized to the tool-name charset.
#[must_use]
pub fn qualified_id(server: &str, tool: &str) -> String {
    format!("{}_{}", sanitize(server), sanitize(tool))
}

/// Disambiguate `base` against the set of ids already in use, returning `base` when free or
/// `base_2`, `base_3`, … for the first free suffix. Sanitization can map two distinct (server, tool)
/// pairs to the same base id; this keeps the exposed catalogue clash-free and deterministic.
#[must_use]
pub fn disambiguate(base: &str, used: &mut std::collections::HashSet<String>) -> String {
    if used.insert(base.to_owned()) {
        return base.to_owned();
    }
    let mut counter = 2usize;
    loop {
        let candidate = format!("{base}_{counter}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        counter += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn two_servers_with_the_same_tool_yield_distinct_ids() {
        assert_eq!(qualified_id("a", "search"), "a_search");
        assert_eq!(qualified_id("b", "search"), "b_search");
        assert_ne!(qualified_id("a", "search"), qualified_id("b", "search"));
    }

    #[test]
    fn sanitizes_illegal_characters_to_underscore() {
        assert_eq!(qualified_id("my server", "do.thing"), "my_server_do_thing");
        assert_eq!(qualified_id("ns:1", "a/b"), "ns_1_a_b");
    }

    #[test]
    fn mapping_is_stable() {
        assert_eq!(qualified_id("srv", "tool"), qualified_id("srv", "tool"));
    }

    #[test]
    fn disambiguate_appends_a_counter_on_collision() {
        let mut used = HashSet::new();
        assert_eq!(disambiguate("a_b_c", &mut used), "a_b_c");
        assert_eq!(disambiguate("a_b_c", &mut used), "a_b_c_2");
        assert_eq!(disambiguate("a_b_c", &mut used), "a_b_c_3");
    }
}
