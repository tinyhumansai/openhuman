use super::*;
use crate::openhuman::integrations::task_sources::types::ProviderSlug;
use crate::openhuman::integrations::task_sources::NormalizedTask;
use chrono::Utc;

#[test]
fn provider_label_titlecases_known_and_unknown() {
    assert_eq!(provider_label("github"), "GitHub");
    assert_eq!(provider_label("clickup"), "ClickUp");
    assert_eq!(provider_label("asana"), "Asana");
    assert_eq!(provider_label(""), "");
}

fn github_source(repo: Option<&str>) -> TaskSource {
    TaskSource {
        id: "ts-1".into(),
        provider: ProviderSlug::Github,
        connection_id: None,
        name: None,
        enabled: true,
        filter: FilterSpec::Github {
            repo: repo.map(str::to_string),
            labels: vec![],
            assignee_is_me: true,
            state: None,
            fetch_mode: Default::default(),
            extra: json!({}),
        },
        interval_secs: 1800,
        target: SourceTarget::AgentTodoProactive,
        max_tasks_per_fetch: 25,
        assigned_executor: None,
        created_at: Utc::now(),
        last_fetch_at: None,
        last_status: None,
    }
}

fn enriched(external_id: &str, url: Option<&str>, urgency: f32) -> EnrichedTask {
    let task = NormalizedTask {
        external_id: external_id.into(),
        provider: "github".into(),
        title: "Fix the bug".into(),
        url: url.map(str::to_string),
        ..Default::default()
    };
    // Objective is derived in enrichment — mirror that here so the helper
    // stays truthful (generic kind → bare title).
    let objective = crate::openhuman::integrations::task_sources::enrich::derive_objective(&task);
    EnrichedTask {
        task,
        summary: "Fix the bug".into(),
        urgency,
        linked_people: vec![],
        linked_memory_ids: vec![],
        agent_prompt: "do it".into(),
        objective,
        enriched_at: Utc::now(),
    }
}

#[test]
fn source_metadata_carries_github_repo_and_identifiers() {
    let src = github_source(Some("octo/repo"));
    let e = enriched("123", Some("https://github.com/octo/repo/issues/123"), 0.7);
    let meta = build_source_metadata(&src, &e);
    assert_eq!(meta["provider"], json!("github"));
    assert_eq!(meta["source_id"], json!("ts-1"));
    assert_eq!(meta["external_id"], json!("123"));
    assert_eq!(meta["repo"], json!("octo/repo"));
    assert_eq!(
        meta["url"],
        json!("https://github.com/octo/repo/issues/123")
    );
    let urgency = meta["urgency"].as_f64().expect("urgency is a number");
    assert!((urgency - 0.7).abs() < 1e-6, "urgency was {urgency}");
}

#[test]
fn source_metadata_omits_absent_repo_and_url() {
    let src = github_source(None);
    let e = enriched("9", None, 0.4);
    let meta = build_source_metadata(&src, &e);
    assert!(meta.get("repo").is_none());
    assert!(meta.get("url").is_none());
    assert_eq!(meta["external_id"], json!("9"));
    let urgency = meta["urgency"].as_f64().expect("urgency is a number");
    assert!((urgency - 0.4).abs() < 1e-6, "urgency was {urgency}");
}

fn temp_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    (tmp, config)
}

#[tokio::test]
async fn add_card_stamps_objective_assigned_agent_and_metadata() {
    let (_tmp, config) = temp_config();
    let mut src = github_source(Some("octo/repo"));
    // Whitespace around the executor must be trimmed into assigned_agent.
    src.assigned_executor = Some("  agent-x  ".into());
    let e = enriched("123", Some("https://github.com/octo/repo/issues/123"), 0.7);

    add_card(&config, &src, &e, None)
        .await
        .expect("add_card succeeds");

    let cards = board_cards(&config).await.expect("board_cards");
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    // Display title is the `[provider] title` form; objective is the bare title.
    assert_eq!(card.title, "[GitHub] Fix the bug");
    assert_eq!(card.objective.as_deref(), Some("Fix the bug"));
    assert_eq!(card.assigned_agent.as_deref(), Some("agent-x"));
    let meta = card
        .source_metadata
        .as_ref()
        .expect("source_metadata present");
    assert_eq!(meta["external_id"], json!("123"));
    assert_eq!(meta["repo"], json!("octo/repo"));
    // Generic kind is not stamped onto metadata.
    assert!(meta.get("kind").is_none());
}

#[tokio::test]
async fn pull_request_card_carries_review_objective_and_kind_metadata() {
    let (_tmp, config) = temp_config();
    let src = github_source(Some("octo/repo"));
    let mut task = NormalizedTask {
        external_id: "55".into(),
        provider: "github".into(),
        title: "Add retry".into(),
        ..Default::default()
    };
    task.kind = TaskKind::PullRequest;
    let objective = crate::openhuman::integrations::task_sources::enrich::derive_objective(&task);
    let e = EnrichedTask {
        task,
        summary: "Add retry".into(),
        urgency: 0.5,
        linked_people: vec![],
        linked_memory_ids: vec![],
        agent_prompt: "review it".into(),
        objective,
        enriched_at: Utc::now(),
    };

    add_card(&config, &src, &e, None)
        .await
        .expect("add_card succeeds");

    let cards = board_cards(&config).await.expect("board_cards");
    let card = &cards[0];
    // The objective tells the picking agent (and triage) the job is a review.
    assert_eq!(
        card.objective.as_deref(),
        Some("Review pull request: Add retry")
    );
    let meta = card
        .source_metadata
        .as_ref()
        .expect("source_metadata present");
    assert_eq!(meta["kind"], json!("pull_request"));
}

#[tokio::test]
async fn add_card_drops_whitespace_only_assigned_executor() {
    let (_tmp, config) = temp_config();
    let mut src = github_source(None);
    src.assigned_executor = Some("   ".into());
    let e = enriched("9", None, 0.4);

    add_card(&config, &src, &e, None)
        .await
        .expect("add_card succeeds");

    let cards = board_cards(&config).await.expect("board_cards");
    assert_eq!(cards.len(), 1);
    assert!(
        cards[0].assigned_agent.is_none(),
        "whitespace-only executor should not assign the card"
    );
}

#[test]
fn source_metadata_has_no_repo_for_non_github_provider() {
    let mut src = github_source(Some("octo/repo"));
    // A non-GitHub filter carries no repo concept.
    src.provider = ProviderSlug::Linear;
    src.filter = FilterSpec::Linear {
        team_id: None,
        assignee_is_me: true,
        state: None,
        extra: json!({}),
    };
    let e = enriched("LIN-5", None, 0.5);
    let meta = build_source_metadata(&src, &e);
    assert!(meta.get("repo").is_none());
    assert_eq!(meta["source_id"], json!("ts-1"));
}
