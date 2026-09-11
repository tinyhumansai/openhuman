use super::*;
use crate::openhuman::skills::ops_types::{Workflow, WorkflowFrontmatter};

fn skill_with_triggers(name: &str, triggers: Vec<&str>) -> Workflow {
    Workflow {
        name: name.to_string(),
        frontmatter: WorkflowFrontmatter {
            triggers: triggers.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ── ensure_triggered_workflow_subscriber (C1 boot-path helper) ───────────

#[tokio::test]
async fn ensure_triggered_workflow_subscriber_is_idempotent_and_safe() {
    // Covers the boot-path helper called from both `start_channels` and
    // `bootstrap_core_runtime`: it loads workflow metadata from the
    // workspace and registers the subscriber exactly once via the
    // process-global OnceLock. An empty temp workspace yields no triggered
    // workflows, so registration resolves to `None`; the call must not
    // panic and must be safe to repeat (the OnceLock makes the second call
    // a no-op).
    //
    // Runs under a tokio runtime (like the real async boot paths): the
    // process-global OnceLock means whichever test initializes it first runs
    // the registration closure, and if leaked global workspace state makes
    // `load_workflow_metadata` find workflows, `subscribe_global` will
    // `tokio::spawn` the subscriber task — which panics without a runtime.
    let tmp = tempfile::tempdir().expect("tempdir");
    ensure_triggered_workflow_subscriber(tmp.path());
    ensure_triggered_workflow_subscriber(tmp.path());
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

// ── TriggeredWorkflowIndex ──────────────────────────────────────────────────

#[test]
fn build_ignores_skills_without_triggers() {
    let skills = vec![
        skill_with_triggers("no_triggers", vec![]),
        skill_with_triggers("with_trigger", vec!["cron"]),
    ];
    let idx = TriggeredWorkflowIndex::build(&skills);
    assert_eq!(idx.len(), 1);
    assert!(!idx.is_empty());
}

#[test]
fn build_empty_skills_list_is_empty() {
    let idx = TriggeredWorkflowIndex::build(&[]);
    assert!(idx.is_empty());
}

#[test]
fn build_sorts_entries_by_skill_name() {
    let skills = vec![
        skill_with_triggers("zzz_skill", vec!["cron"]),
        skill_with_triggers("aaa_skill", vec!["channel"]),
    ];
    let idx = TriggeredWorkflowIndex::build(&skills);
    assert_eq!(idx.entries[0].0, "aaa_skill");
    assert_eq!(idx.entries[1].0, "zzz_skill");
}

#[test]
fn domains_returns_unique_sorted_set() {
    let skills = vec![
        skill_with_triggers("a", vec!["composio", "cron"]),
        skill_with_triggers("b", vec!["composio", "channel"]),
    ];
    let idx = TriggeredWorkflowIndex::build(&skills);
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
    let idx = TriggeredWorkflowIndex::build(&skills);
    let event = DomainEvent::CronJobTriggered {
        job_id: "j1".into(),
        job_name: "test".into(),
        job_type: "shell".into(),
    };
    let mut matched = idx.matching_workflows(&event);
    matched.sort_unstable();
    assert_eq!(matched, vec!["cron_watcher", "multi_watcher"]);
}

#[test]
fn matching_skills_returns_empty_when_no_match() {
    let skills = vec![skill_with_triggers("composio_watcher", vec!["composio"])];
    let idx = TriggeredWorkflowIndex::build(&skills);
    let event = DomainEvent::SystemStartup {
        component: "core".into(),
    };
    assert!(idx.matching_workflows(&event).is_empty());
}
