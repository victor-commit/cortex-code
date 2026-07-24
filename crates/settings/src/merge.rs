//! Per-key deep-merge of TOML tables.
//!
//! Layering is a recursive, per-key merge rather than a whole-file replacement: a higher
//! layer only overrides the keys it actually sets, and nested tables are merged key-by-key.
//! A key present only in a lower layer therefore survives when a higher layer sets a
//! *different* key (adding a higher layer never erases unrelated lower-layer keys).

use toml::Value;
use toml::value::Table;

/// Merge `overlay` into `base` in place. For every key:
/// - if both sides hold a sub-table, the sub-tables are merged recursively (per-key);
/// - otherwise the overlay value wins and replaces whatever `base` held.
pub fn merge_into(base: &mut Table, overlay: Table) {
    for (key, overlay_value) in overlay {
        match (base.get_mut(&key), overlay_value) {
            (Some(Value::Table(base_table)), Value::Table(overlay_table)) => {
                merge_into(base_table, overlay_table);
            }
            (_, overlay_value) => {
                base.insert(key, overlay_value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(toml: &str) -> Table {
        toml::from_str(toml).expect("valid toml fixture")
    }

    #[test]
    fn higher_layer_overrides_same_key() {
        let mut base = table(r#"model = "default""#);
        merge_into(&mut base, table(r#"model = "override""#));
        assert_eq!(base["model"].as_str(), Some("override"));
    }

    #[test]
    fn unrelated_lower_keys_survive() {
        // Key A only in the lower layer, key B only in the higher layer: both must survive.
        let mut base = table(r#"a = "from-low""#);
        merge_into(&mut base, table(r#"b = "from-high""#));
        assert_eq!(base["a"].as_str(), Some("from-low"));
        assert_eq!(base["b"].as_str(), Some("from-high"));
    }

    #[test]
    fn nested_tables_merge_per_key() {
        let mut base = table(
            r#"
            [server]
            host = "localhost"
            port = 18765
            "#,
        );
        merge_into(
            &mut base,
            table(
                r#"
                [server]
                port = 3210
                "#,
            ),
        );
        let server = base["server"].as_table().expect("server table");
        // The overridden key changes ...
        assert_eq!(server["port"].as_integer(), Some(3210));
        // ... while the sibling key set only in the lower layer survives.
        assert_eq!(server["host"].as_str(), Some("localhost"));
    }

    #[test]
    fn scalar_replaces_table_when_higher_layer_sets_a_scalar() {
        let mut base = table(
            r#"
            [model]
            name = "x"
            "#,
        );
        merge_into(&mut base, table(r#"model = "flat""#));
        assert_eq!(base["model"].as_str(), Some("flat"));
    }

    #[test]
    fn empty_overlay_is_a_no_op() {
        let mut base = table(r#"model = "kept""#);
        merge_into(&mut base, Table::new());
        assert_eq!(base["model"].as_str(), Some("kept"));
        assert_eq!(base.len(), 1);
    }
}
