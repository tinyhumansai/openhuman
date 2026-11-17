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
use async_trait::async_trait;
use std::sync::{Arc, OnceLock};
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;
use tinyskills::Skill as Workflow;

// ── Trigger pattern ───────────────────────────────────────────────────────────

/// Pattern grammar (`domain` or `domain/event_slug`) is owned by `tinyskills`.
///
/// [`DomainEvent`] exposes no stable slug yet, so matching passes an empty
/// slug: bare-domain patterns match every event in the domain and
/// slug-qualified patterns stay inert rather than firing for the whole domain.
use tinyskills::TriggerPattern;

fn pattern_matches_event(pattern: &TriggerPattern, event: &DomainEvent) -> bool {
    // TODO(#skills-triggers): pass the real slug once DomainEvent exposes one.
    pattern.matches(event.domain(), "")
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
            .filter(|(_, patterns)| patterns.iter().any(|p| pattern_matches_event(p, event)))
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
/// Both [`crate::channels::start_channels`] (messaging cores) and
/// `crate::core::runtime::bootstrap::bootstrap_core_runtime` (always-run serve boot)
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
        let workflows = crate::skills::load_workflow_metadata(workspace);
        register_triggered_workflow_subscriber(&workflows)
    });
}

/// Idempotently install the subscriber from the exact startup Config and
/// profile context after async builtin prewarming has completed.
pub(crate) fn ensure_triggered_workflow_subscriber_for(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    domains: Option<crate::core::runtime::DomainSet>,
) {
    TRIGGERED_WORKFLOW_HANDLE.get_or_init(|| {
        let workflows =
            crate::skills::ops_discover::load_workflow_metadata_for_domains(ctx, config, domains);
        register_triggered_workflow_subscriber(&workflows)
    });
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
