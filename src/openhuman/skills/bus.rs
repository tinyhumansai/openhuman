//! Event bus subscriber for event-triggered skills.
//!
//! Skills that declare a `triggers:` list in their `SKILL.md` frontmatter are
//! indexed at startup by [`TriggeredSkillIndex`]. A [`TriggeredSkillSubscriber`]
//! is then registered on the global event bus; when a matching [`DomainEvent`]
//! arrives it logs which skill(s) should be activated.
//!
//! The actual agent-session launch for triggered skills is intentionally out of
//! scope here — it requires the full harness context (provider, memory, config)
//! that is wired up by the channel runtime after bus initialization. This module
//! provides the **type plumbing and observer** so the integration layer can hook
//! in without touching the bus machinery.

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::skills::Skill;
use async_trait::async_trait;
use std::sync::Arc;

// ── Trigger pattern ───────────────────────────────────────────────────────────

/// A parsed trigger pattern from a skill's `triggers:` frontmatter list.
///
/// Patterns take the form `"domain"` or `"domain/event_slug"`.  A bare domain
/// (no `/`) matches **any** event in that domain; with a slug only events whose
/// discriminant name (lower-kebab-cased) equals the slug are matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPattern {
    /// The event domain, e.g. `"composio"`, `"cron"`, `"channel"`.
    pub domain: String,
    /// Optional event slug; `None` means match the entire domain.
    pub event_slug: Option<String>,
}

impl TriggerPattern {
    /// Parse a raw trigger string like `"composio/trigger_received"` or `"cron"`.
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        match raw.split_once('/') {
            Some((domain, slug)) => {
                let domain = domain.trim().to_ascii_lowercase();
                let slug = slug.trim().to_ascii_lowercase();
                if domain.is_empty() {
                    return None;
                }
                Some(Self {
                    domain,
                    event_slug: if slug.is_empty() || slug == "*" {
                        None
                    } else {
                        Some(slug)
                    },
                })
            }
            None => Some(Self {
                domain: raw.to_ascii_lowercase(),
                event_slug: None,
            }),
        }
    }

    /// Returns true when this pattern matches the given event.
    ///
    /// Slug-qualified patterns (e.g. `"agent/task_complete"`) are rejected
    /// until [`DomainEvent`] exposes a stable `slug()` method — returning
    /// `true` here would silently match the entire domain, firing for every
    /// event regardless of the declared slug.
    pub fn matches(&self, event: &DomainEvent) -> bool {
        if event.domain() != self.domain {
            return false;
        }
        // Slug-qualified patterns cannot be matched precisely yet.
        // TODO(#skills-triggers): replace with `event.slug() == slug` once
        // DomainEvent exposes slug().
        if self.event_slug.is_some() {
            return false;
        }
        true
    }
}

// ── Triggered skill index ─────────────────────────────────────────────────────

/// Index of skills that declare event triggers, built from discovered skills.
///
/// Call [`TriggeredSkillIndex::build`] after the skill discovery pass, then
/// pass the result to [`register_triggered_skill_subscriber`].
#[derive(Debug, Default)]
pub struct TriggeredSkillIndex {
    /// Sorted `(skill_name, patterns)` pairs. Sorted for deterministic logging.
    entries: Vec<(String, Vec<TriggerPattern>)>,
}

impl TriggeredSkillIndex {
    /// Build an index from a slice of discovered skills.
    ///
    /// Skills with an empty `triggers:` list are skipped.
    pub fn build(skills: &[Skill]) -> Self {
        let mut entries: Vec<(String, Vec<TriggerPattern>)> = skills
            .iter()
            .filter_map(|skill| {
                let patterns: Vec<TriggerPattern> = skill
                    .frontmatter
                    .triggers
                    .iter()
                    .filter_map(|t| {
                        let p = TriggerPattern::parse(t);
                        if p.is_none() {
                            log::warn!(
                                "[skills::triggered] skill '{}': malformed trigger {:?} — skipping",
                                skill.name,
                                t
                            );
                        }
                        p
                    })
                    .collect();
                if patterns.is_empty() {
                    None
                } else {
                    Some((skill.name.clone(), patterns))
                }
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Self { entries }
    }

    /// Returns `true` when no skills have declared triggers.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of skills with at least one trigger pattern.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns all unique domain strings across every trigger pattern.
    pub fn domains(&self) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        for (_, patterns) in &self.entries {
            for p in patterns {
                seen.insert(p.domain.clone());
            }
        }
        seen.into_iter().collect()
    }

    /// Returns the names of skills whose trigger patterns match `event`.
    pub fn matching_skills<'a>(&'a self, event: &DomainEvent) -> Vec<&'a str> {
        self.entries
            .iter()
            .filter(|(_, patterns)| patterns.iter().any(|p| p.matches(event)))
            .map(|(name, _)| name.as_str())
            .collect()
    }
}

// ── Subscriber ────────────────────────────────────────────────────────────────

struct TriggeredSkillSubscriber {
    index: Arc<TriggeredSkillIndex>,
}

#[async_trait]
impl EventHandler for TriggeredSkillSubscriber {
    fn name(&self) -> &str {
        "skills::triggered_skill"
    }

    // No `domains()` filter — the domain list is dynamic (built from skill
    // triggers at startup) and the `EventHandler` trait returns `&[&str]`
    // which cannot point into an owned Vec<String>. Filtering in `handle()`
    // is equivalent and avoids an unsafe lifetime trick.

    async fn handle(&self, event: &DomainEvent) {
        let matched = self.index.matching_skills(event);
        if matched.is_empty() {
            return;
        }
        tracing::debug!(
            domain = event.domain(),
            skills = ?matched,
            "[skills::triggered] event matches {} skill trigger(s); \
             activation handoff to integration layer pending",
            matched.len()
        );
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Register a subscriber for all skills that declare `triggers:` patterns.
///
/// Call this once at startup **after** skill discovery is complete. Skills with
/// an empty `triggers:` list are ignored. Returns `None` when no skills have
/// triggers (no subscription is created). The returned [`SubscriptionHandle`]
/// must be kept alive for the duration of the process.
///
/// ```text
/// // In channel runtime startup, after load_skills():
/// static SKILL_TRIGGER_HANDLE: OnceLock<Option<SubscriptionHandle>> = OnceLock::new();
/// SKILL_TRIGGER_HANDLE.get_or_init(|| {
///     skills::bus::register_triggered_skill_subscriber(&discovered_skills)
/// });
/// ```
pub fn register_triggered_skill_subscriber(skills: &[Skill]) -> Option<SubscriptionHandle> {
    let index = TriggeredSkillIndex::build(skills);
    if index.is_empty() {
        return None;
    }
    log::info!(
        "[skills::triggered] registering subscriber for {} skill(s) with event triggers (domains: {:?})",
        index.len(),
        index.domains()
    );
    subscribe_global(Arc::new(TriggeredSkillSubscriber {
        index: Arc::new(index),
    }))
}

/// Legacy no-op retained while call-sites migrate to
/// [`register_triggered_skill_subscriber`]. Safe to call multiple times.
pub fn register_skill_cleanup_subscriber() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::skills::ops_types::{Skill, SkillFrontmatter};

    fn skill_with_triggers(name: &str, triggers: Vec<&str>) -> Skill {
        Skill {
            name: name.to_string(),
            frontmatter: SkillFrontmatter {
                triggers: triggers.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    // ── TriggerPattern::parse ────────────────────────────────────────────────

    #[test]
    fn parse_bare_domain() {
        let p = TriggerPattern::parse("composio").unwrap();
        assert_eq!(p.domain, "composio");
        assert!(p.event_slug.is_none());
    }

    #[test]
    fn parse_domain_with_slug() {
        let p = TriggerPattern::parse("composio/trigger_received").unwrap();
        assert_eq!(p.domain, "composio");
        assert_eq!(p.event_slug.as_deref(), Some("trigger_received"));
    }

    #[test]
    fn parse_domain_with_wildcard_slug_is_bare() {
        let p = TriggerPattern::parse("cron/*").unwrap();
        assert_eq!(p.domain, "cron");
        assert!(p.event_slug.is_none());
    }

    #[test]
    fn parse_normalises_to_lowercase() {
        let p = TriggerPattern::parse("Composio/TRIGGER_RECEIVED").unwrap();
        assert_eq!(p.domain, "composio");
        assert_eq!(p.event_slug.as_deref(), Some("trigger_received"));
    }

    #[test]
    fn parse_empty_is_none() {
        assert!(TriggerPattern::parse("").is_none());
        assert!(TriggerPattern::parse("   ").is_none());
    }

    #[test]
    fn parse_empty_domain_with_slug_is_none() {
        assert!(TriggerPattern::parse("/event_slug").is_none());
    }

    // ── TriggerPattern::matches ──────────────────────────────────────────────

    #[test]
    fn bare_domain_matches_any_event_in_domain() {
        let p = TriggerPattern::parse("cron").unwrap();
        let event = DomainEvent::CronJobTriggered {
            job_id: "j1".into(),
            job_name: "test".into(),
            job_type: "shell".into(),
        };
        assert!(p.matches(&event));
    }

    #[test]
    fn bare_domain_does_not_match_other_domain() {
        let p = TriggerPattern::parse("cron").unwrap();
        let event = DomainEvent::SystemStartup {
            component: "core".into(),
        };
        assert!(!p.matches(&event));
    }

    #[test]
    fn slugged_pattern_rejected_until_slug_api_exists() {
        // A slug-qualified pattern like "cron/job_triggered" must NOT match
        // the entire cron domain — returning true here would over-fire for
        // every cron event regardless of the declared slug.
        let p = TriggerPattern::parse("cron/job_triggered").unwrap();
        assert_eq!(p.event_slug.as_deref(), Some("job_triggered"));
        let event = DomainEvent::CronJobTriggered {
            job_id: "j1".into(),
            job_name: "test".into(),
            job_type: "shell".into(),
        };
        assert!(
            !p.matches(&event),
            "slugged pattern must not match until DomainEvent::slug() exists"
        );
    }

    // ── TriggeredSkillIndex ──────────────────────────────────────────────────

    #[test]
    fn build_ignores_skills_without_triggers() {
        let skills = vec![
            skill_with_triggers("no_triggers", vec![]),
            skill_with_triggers("with_trigger", vec!["cron"]),
        ];
        let idx = TriggeredSkillIndex::build(&skills);
        assert_eq!(idx.len(), 1);
        assert!(!idx.is_empty());
    }

    #[test]
    fn build_empty_skills_list_is_empty() {
        let idx = TriggeredSkillIndex::build(&[]);
        assert!(idx.is_empty());
    }

    #[test]
    fn build_sorts_entries_by_skill_name() {
        let skills = vec![
            skill_with_triggers("zzz_skill", vec!["cron"]),
            skill_with_triggers("aaa_skill", vec!["channel"]),
        ];
        let idx = TriggeredSkillIndex::build(&skills);
        assert_eq!(idx.entries[0].0, "aaa_skill");
        assert_eq!(idx.entries[1].0, "zzz_skill");
    }

    #[test]
    fn domains_returns_unique_sorted_set() {
        let skills = vec![
            skill_with_triggers("a", vec!["composio", "cron"]),
            skill_with_triggers("b", vec!["composio", "channel"]),
        ];
        let idx = TriggeredSkillIndex::build(&skills);
        let domains = idx.domains();
        assert_eq!(domains, vec!["channel", "composio", "cron"]);
    }

    #[test]
    fn matching_skills_returns_correct_names() {
        let skills = vec![
            skill_with_triggers("cron_watcher", vec!["cron"]),
            skill_with_triggers("composio_watcher", vec!["composio"]),
            skill_with_triggers("multi_watcher", vec!["cron", "composio"]),
        ];
        let idx = TriggeredSkillIndex::build(&skills);
        let event = DomainEvent::CronJobTriggered {
            job_id: "j1".into(),
            job_name: "test".into(),
            job_type: "shell".into(),
        };
        let mut matched = idx.matching_skills(&event);
        matched.sort_unstable();
        assert_eq!(matched, vec!["cron_watcher", "multi_watcher"]);
    }

    #[test]
    fn matching_skills_returns_empty_when_no_match() {
        let skills = vec![skill_with_triggers("composio_watcher", vec!["composio"])];
        let idx = TriggeredSkillIndex::build(&skills);
        let event = DomainEvent::SystemStartup {
            component: "core".into(),
        };
        assert!(idx.matching_skills(&event).is_empty());
    }

    #[test]
    fn register_skill_cleanup_subscriber_is_a_safe_noop() {
        register_skill_cleanup_subscriber();
        register_skill_cleanup_subscriber();
    }
}
