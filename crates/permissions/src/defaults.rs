use crate::{Action, Rule};

/// Default safety rules applied across surfaces for external paths and dotenv files.
#[must_use]
pub fn default_external_and_env_rules() -> Vec<Rule> {
    vec![
        Rule::new("external_directory", "*", Action::Ask),
        Rule::new("read", "*.env", Action::Ask),
        Rule::new("read", "*.env.*", Action::Ask),
        Rule::new("read", "*.env.example", Action::Allow),
    ]
}

/// Permissive base defaults with targeted asks for sensitive reads and external directories.
#[must_use]
pub fn permissive_default_rules() -> Vec<Rule> {
    let mut rules = vec![Rule::new("*", "*", Action::Allow)];
    rules.extend(default_external_and_env_rules());
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissive_defaults_keep_general_actions_allowed() {
        let rules = permissive_default_rules();
        assert_eq!(rules[0], Rule::new("*", "*", Action::Allow));
        assert!(
            rules
                .iter()
                .any(|rule| rule.tool == "external_directory" && rule.action == Action::Ask)
        );
        assert!(
            rules
                .iter()
                .any(|rule| rule.tool == "read" && rule.pattern == "*.env.example")
        );
    }
}
