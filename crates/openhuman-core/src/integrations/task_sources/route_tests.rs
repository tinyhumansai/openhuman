use super::*;
use crate::integrations::task_sources::types::{FilterSpec, ProviderSlug};
use crate::integrations::task_sources::NormalizedTask;
use chrono::Utc;

fn github_source(target: SourceTarget) -> TaskSource {
    TaskSource {
        id: "ts-1".into(),
        provider: ProviderSlug::Github,
        connection_id: None,
        name: None,
        enabled: true,
        filter: FilterSpec::Github {
            repo: Some("octo/repo".into()),
            labels: vec![],
            assignee_is_me: true,
            state: None,
            fetch_mode: Default::default(),
            extra: json!({}),
        },
        interval_secs: 1800,
        target,
        max_tasks_per_fetch: 25,
        created_at: Utc::now(),
        last_fetch_at: None,
        last_status: None,
    }
}

fn enriched(external_id: &str) -> EnrichedTask {
    let task = NormalizedTask {
        external_id: external_id.into(),
        provider: "github".into(),
        title: "Fix the bug".into(),
        ..Default::default()
    };
    let objective = crate::integrations::task_sources::enrich::derive_objective(&task);
    EnrichedTask {
        task,
        summary: "Fix the bug".into(),
        urgency: 0.7,
        linked_people: vec![],
        linked_memory_ids: vec![],
        agent_prompt: "do it".into(),
        objective,
        enriched_at: Utc::now(),
    }
}

/// A collect-only source stops at the ledger the pipeline writes: no board
/// card (there is no board any more) and no agent turn, so routing needs
/// nothing from the environment and cannot fail.
#[tokio::test]
async fn collect_only_target_routes_without_side_effects() {
    let config = Config::default();
    let src = github_source(SourceTarget::TodoOnly);

    route_enriched(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &src,
        &enriched("123"),
    )
    .await
    .expect("collect-only routing is a no-op");
}
