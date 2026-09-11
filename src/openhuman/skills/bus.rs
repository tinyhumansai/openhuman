//! Event bus subscriber for event-triggered skills.
//!
//! Skills that declare a `triggers:` list in their `SKILL.md` frontmatter are
//! indexed at startup by [`TriggeredWorkflowIndex`]. A [`TriggeredSkillSubscriber`]
//! is then registered on the global event bus; when a matching [`DomainEvent`]
//! arrives it logs which skill(s) should be activated.
//!
//! The actual agent-session launch for triggered skills is intentionally out of
//! scope here — it requires the full harness context (provider, memory, config)
//! that is wired up by the channel runtime after bus initialization. This module
//! provides the **type plumbing and observer** so the integration layer can hook
//! in without touching the bus machinery.

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::skills::Workflow;
use async_trait::async_trait;
use std::sync::{Arc, OnceLock};
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

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
/// Call [`TriggeredWorkflowIndex::build`] after the skill discovery pass, then
/// pass the result to [`register_triggered_workflow_subscriber`].
#[derive(Debug, Default)]
pub struct TriggeredWorkflowIndex {
    /// Sorted `(skill_name, patterns)` pairs. Sorted for deterministic logging.
    entries: Vec<(String, Vec<TriggerPattern>)>,
}

impl TriggeredWorkflowIndex {
    /// Build an index from a slice of discovered skills.
    ///
    /// Skills with an empty `triggers:` list are skipped.
    pub fn build(skills: &[Workflow]) -> Self {
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
                                "[workflows::triggered] skill '{}': malformed trigger {:?} — skipping",
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
    pub fn matching_workflows<'a>(&'a self, event: &DomainEvent) -> Vec<&'a str> {
        self.entries
            .iter()
            .filter(|(_, patterns)| patterns.iter().any(|p| p.matches(event)))
            .map(|(name, _)| name.as_str())
            .collect()
    }
}

// ── Subscriber ────────────────────────────────────────────────────────────────

struct TriggeredSkillSubscriber {
    index: Arc<TriggeredWorkflowIndex>,
}

#[async_trait]
impl EventHandler<DomainEvent> for TriggeredSkillSubscriber {
    fn name(&self) -> &str {
        "skills::triggered_skill"
    }

    // No `domains()` filter — the domain list is dynamic (built from skill
    // triggers at startup) and the `EventHandler` trait returns `&[&str]`
    // which cannot point into an owned Vec<String>. Filtering in `handle()`
    // is equivalent and avoids an unsafe lifetime trick.

    async fn handle(&self, event: &DomainEvent) {
        let matched = self.index.matching_workflows(event);
        if matched.is_empty() {
            return;
        }
        tracing::debug!(
            domain = event.domain(),
            skills = ?matched,
            "[workflows::triggered] event matches {} skill trigger(s); \
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
/// // In channel runtime startup, after load_workflow_metadata():
/// static SKILL_TRIGGER_HANDLE: OnceLock<Option<SubscriptionHandle>> = OnceLock::new();
/// SKILL_TRIGGER_HANDLE.get_or_init(|| {
///     skills::bus::register_triggered_workflow_subscriber(&discovered_skills)
/// });
/// ```
pub fn register_triggered_workflow_subscriber(skills: &[Workflow]) -> Option<SubscriptionHandle> {
    let index = TriggeredWorkflowIndex::build(skills);
    if index.is_empty() {
        return None;
    }
    log::info!(
        "[workflows::triggered] registering subscriber for {} skill(s) with event triggers (domains: {:?})",
        index.len(),
        index.domains()
    );
    BUS.subscribe(Arc::new(TriggeredSkillSubscriber {
        index: Arc::new(index),
    }))
}

/// Process-global parking spot for the triggered-workflow subscription
/// handle. The RAII [`SubscriptionHandle`] must outlive the process (dropping
/// it cancels the subscription), and registration must happen exactly once no
/// matter how many startup paths reach it.
static TRIGGERED_WORKFLOW_HANDLE: OnceLock<Option<SubscriptionHandle>> = OnceLock::new();

/// Idempotently install the triggered-workflow subscriber.
///
/// Loads workflow metadata from `workspace` and registers the subscriber on the
/// **first** call; subsequent calls are no-ops (the handle is parked in
/// [`TRIGGERED_WORKFLOW_HANDLE`] so the RAII guard isn't dropped). Safe to call
/// from every startup path.
///
/// Both [`crate::openhuman::channels::start_channels`] (messaging cores) and
/// [`crate::core::jsonrpc::bootstrap_core_runtime`] (always-run serve boot)
/// invoke this. `start_channels` is skipped for web-chat-only desktop installs
/// (no messaging integration connected) and when
/// `OPENHUMAN_DISABLE_CHANNEL_LISTENERS=1`; registering from
/// `bootstrap_core_runtime` too means those cores still honour workflow
/// `triggers:`. The shared `OnceLock` guarantees a single registration
/// regardless of which path runs first.
///
/// NOTE: the subscriber currently only *matches* triggers and logs — the
/// activation handoff to the integration layer is still pending (see
/// [`TriggeredSkillSubscriber::handle`]). Registering on web-chat-only cores
/// enables matching, not yet activation.
pub fn ensure_triggered_workflow_subscriber(workspace: &std::path::Path) {
    TRIGGERED_WORKFLOW_HANDLE.get_or_init(|| {
        let workflows = crate::openhuman::skills::load_workflow_metadata(workspace);
        register_triggered_workflow_subscriber(&workflows)
    });
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
