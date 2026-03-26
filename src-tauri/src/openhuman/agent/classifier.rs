use crate::openhuman::config::schema::QueryClassificationConfig;

/// Classify a user message against the configured rules and return the
/// matching hint string, if any.
///
/// Returns `None` when classification is disabled, no rules are configured,
/// or no rule matches the message.
pub fn classify(config: &QueryClassificationConfig, message: &str) -> Option<String> {
    if !config.enabled || config.rules.is_empty() {
        return None;
    }

    let lower = message.to_lowercase();
    let len = message.len();

    let mut rules: Vec<_> = config.rules.iter().collect();
    rules.sort_by(|a, b| b.priority.cmp(&a.priority));

    for rule in rules {
        // Length constraints
        if let Some(min) = rule.min_length {
            if len < min {
                continue;
            }
        }
        if let Some(max) = rule.max_length {
            if len > max {
                continue;
            }
        }

        // Check keywords (case-insensitive) and patterns (case-sensitive)
        let keyword_hit = rule
            .keywords
            .iter()
            .any(|kw: &String| lower.contains(&kw.to_lowercase()));
        let pattern_hit = rule
            .patterns
            .iter()
            .any(|pat: &String| message.contains(pat.as_str()));

        if keyword_hit || pattern_hit {
            return Some(rule.hint.clone());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::schema::{ClassificationRule, QueryClassificationConfig};

    fn make_config(enabled: bool, rules: Vec<ClassificationRule>) -> QueryClassificationConfig {
        QueryClassificationConfig { enabled, rules }
    }

    #[test]
    fn disabled_returns_none() {
        let config = make_config(
            false,
            vec![ClassificationRule {
                hint: "fast".into(),
                keywords: vec!["hello".into()],
                ..Default::default()
            }],
        );
        assert_eq!(classify(&config, "hello"), None);
    }

    #[test]
    fn empty_rules_returns_none() {
        let config = make_config(true, vec![]);
        assert_eq!(classify(&config, "hello"), None);
    }

    #[test]
    fn keyword_match_case_insensitive() {
        let config = make_config(
            true,
            vec![ClassificationRule {
                hint: "fast".into(),
                keywords: vec!["hello".into()],
                ..Default::default()
            }],
        );
        assert_eq!(classify(&config, "HELLO world"), Some("fast".into()));
    }

    #[test]
    fn pattern_match_case_sensitive() {
        let config = make_config(
            true,
            vec![ClassificationRule {
                hint: "code".into(),
                patterns: vec!["fn ".into()],
                ..Default::default()
            }],
        );
        assert_eq!(classify(&config, "fn main()"), Some("code".into()));
        assert_eq!(classify(&config, "FN MAIN()"), None);
    }

    #[test]
    fn length_constraints() {
        let config = make_config(
            true,
            vec![ClassificationRule {
                hint: "fast".into(),
                keywords: vec!["hi".into()],
                max_length: Some(10),
                ..Default::default()
            }],
        );
        assert_eq!(classify(&config, "hi"), Some("fast".into()));
        assert_eq!(
            classify(&config, "hi there, how are you doing today?"),
            None
        );

        let config2 = make_config(
            true,
            vec![ClassificationRule {
                hint: "reasoning".into(),
                keywords: vec!["explain".into()],
                min_length: Some(20),
                ..Default::default()
            }],
        );
        assert_eq!(classify(&config2, "explain"), None);
        assert_eq!(
            classify(&config2, "explain how this works in detail"),
            Some("reasoning".into())
        );
    }

    #[test]
    fn priority_ordering() {
        let config = make_config(
            true,
            vec![
                ClassificationRule {
                    hint: "fast".into(),
                    keywords: vec!["code".into()],
                    priority: 1,
                    ..Default::default()
                },
                ClassificationRule {
                    hint: "code".into(),
                    keywords: vec!["code".into()],
                    priority: 10,
                    ..Default::default()
                },
            ],
        );
        assert_eq!(classify(&config, "write some code"), Some("code".into()));
    }

    #[test]
    fn no_match_returns_none() {
        let config = make_config(
            true,
            vec![ClassificationRule {
                hint: "fast".into(),
                keywords: vec!["hello".into()],
                ..Default::default()
            }],
        );
        assert_eq!(classify(&config, "something completely different"), None);
    }
}
