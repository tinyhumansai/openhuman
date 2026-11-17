use super::*;

use crate::memory::lifecycle::jobs;
use tinymemory_api::{
    ItemKind, LearningKind, MemoryEngine, MemoryMeta, MetaFilter, Role, StoreItem, Turn,
};

#[path = "adapter_tests.rs"]
mod adapter_tests;

use crate::memory::test_fixtures::{bind_reference, config_in, stored};

type PreTurnArgs = (
    String,
    u32,
    String,
    u32,
    DateTime<Utc>,
    bool,
    Option<tinymemory_api::ObservedActor>,
);

type PostTurnArgs = (String, u32, String, Vec<ToolCallSummary>, DateTime<Utc>);

fn input(thread: &str, index: u32, text: &str) -> PreTurnArgs {
    (
        thread.to_string(),
        index,
        text.to_string(),
        0,
        Utc::now(),
        false,
        None,
    )
}

fn reply(thread: &str, index: u32, text: &str) -> PostTurnArgs {
    (
        thread.to_string(),
        index,
        text.to_string(),
        Vec::new(),
        Utc::now(),
    )
}

async fn run_pre_turn(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    identity: &tinymemory_tools::layout::identity::ResolvedIdentity,
    (thread, index, text, in_prompt_from, at, resumed, observed_actor): PreTurnArgs,
) -> Option<TurnPack> {
    pre_turn_work(
        ctx,
        config,
        identity,
        thread,
        index,
        text,
        in_prompt_from,
        at,
        resumed,
        observed_actor,
        None,
    )
    .await
}

async fn run_post_turn(
    config: &crate::config::Config,
    identity: &tinymemory_tools::layout::identity::ResolvedIdentity,
    (thread, index, text, calls, at): PostTurnArgs,
) {
    post_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        identity,
        thread,
        index,
        text,
        calls,
        at,
        None,
    )
    .await;
}

#[test]
fn turn_indices_follow_the_committed_transcript() {
    assert_eq!(user_turn_index(0), 0);
    assert_eq!(user_turn_index(3), 6);
}

#[path = "hooks_channel_tests.rs"]
mod channel_tests;

#[test]
fn a_logged_reply_keeps_one_line_per_tool_result() {
    let calls = vec![
        ToolCallSummary {
            name: "web_search".into(),
            id: Some("c1".into()),
            result: Some("  Rust 1.90\n released   today ".into()),
        },
        ToolCallSummary {
            name: "noop".into(),
            id: None,
            result: None,
        },
        ToolCallSummary {
            name: "huge".into(),
            id: None,
            result: Some("x".repeat(5000)),
        },
    ];
    let text = logged_reply(" Done. ", &calls);
    assert!(text.starts_with("Done.\n\nTools:\n- web_search → Rust 1.90 released today"));
    assert!(!text.contains("noop"));
    let huge = text.lines().last().unwrap();
    assert!(huge.chars().count() <= MAX_TOOL_LINE_CHARS + "- huge → ".chars().count());
    assert_eq!(logged_reply("plain", &[]), "plain");
}

#[tokio::test]
async fn pre_turn_logs_the_user_turn_and_injects_what_memory_holds() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    engine
        .store(StoreItem::learning(
            "The user's favourite colour is teal",
            LearningKind::Preference,
            0.9,
            MemoryMeta::default(),
        ))
        .await
        .unwrap();
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));

    let pack = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t1", 0, "what colour do I like?"),
    )
    .await
    .expect("a pack");
    assert!(pack.markdown.contains("teal"), "{}", pack.markdown);
    assert!(!pack.refs.is_empty());
    assert!(
        pack.citations
            .iter()
            .any(|citation| citation.snippet.contains("teal")),
        "the pack carries its citations"
    );
    assert!(pack.injection().starts_with(OPEN_TAG));
    assert!(pack.injection().ends_with(CLOSE_TAG));

    let turns = stored(&engine, MetaFilter::kinds([ItemKind::Conversation])).await;
    assert_eq!(turns.len(), 1, "the user turn was logged");
    assert_eq!(turns[0].meta.agent_id.as_deref(), Some("orchestrator"));
    assert_eq!(turns[0].meta.namespace.to_string(), "agent:orchestrator");
}

#[tokio::test]
async fn recall_off_still_logs_and_logging_off_still_recalls() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    let engine = bind_reference(&config);
    engine
        .store(StoreItem::learning(
            "Deploys happen on Fridays",
            LearningKind::Fact,
            0.9,
            MemoryMeta::default(),
        ))
        .await
        .unwrap();

    config.memory.recall.enabled = false;
    let quiet = tinymemory_tools::layout::identity::Identity::agent("a")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert!(run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &quiet,
        input("t", 0, "when do deploys happen?")
    )
    .await
    .is_none());
    assert_eq!(
        stored(&engine, MetaFilter::kinds([ItemKind::Conversation]))
            .await
            .len(),
        1
    );

    config.memory.recall.enabled = true;
    config.memory.conversations.enabled = false;
    let reader = tinymemory_tools::layout::identity::Identity::agent("a")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let pack = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &reader,
        input("t", 2, "when do deploys happen?"),
    )
    .await;
    assert!(pack.expect("a pack").markdown.contains("Fridays"));
    assert_eq!(
        stored(&engine, MetaFilter::kinds([ItemKind::Conversation]))
            .await
            .len(),
        1,
        "nothing new was logged"
    );

    config.memory.recall.enabled = false;
    let none = tinymemory_tools::layout::identity::Identity::agent("a")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert!(run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &none,
        input("t", 4, "hi")
    )
    .await
    .is_none());
}

#[tokio::test]
async fn pre_turn_without_an_engine_runs_the_turn_without_a_pack() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let identity = tinymemory_tools::layout::identity::Identity::agent("a")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert!(run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t", 0, "hello")
    )
    .await
    .is_none());
    run_post_turn(&config, &identity, reply("t", 1, "hi")).await;
}

#[tokio::test]
async fn post_turn_logs_the_reply_and_queues_belief_builds() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.recall.build_beliefs_every = 2;
    let engine = bind_reference(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("writer")
        .resolve(&crate::memory::scope::identity_pins(&config));

    let mut first = reply("t", 1, "Here is the draft.");
    first.3.push(ToolCallSummary {
        name: "read_file".into(),
        id: Some("call-1".into()),
        result: Some("outline.md: three sections".into()),
    });
    run_post_turn(&config, &identity, first).await;

    let turns = stored(&engine, MetaFilter::kinds([ItemKind::Conversation])).await;
    assert_eq!(turns.len(), 1);
    assert!(turns[0].text.contains("read_file → outline.md"));
    assert_eq!(turns[0].meta.agent_id.as_deref(), Some("writer"));
    let pending = jobs::snapshot(&config).await.pending;
    assert_eq!(
        pending.len(),
        1,
        "turn index 1 is the 2nd turn: a build is due"
    );

    config.memory.conversations.enabled = false;
    run_post_turn(&config, &identity, reply("t", 3, "Again.")).await;
    assert_eq!(
        stored(&engine, MetaFilter::kinds([ItemKind::Conversation]))
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn compaction_recalls_from_the_dropped_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("a")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let _ = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t", 0, "the project codename is Heron"),
    )
    .await;
    run_post_turn(&config, &identity, reply("t", 1, "Noted: Heron.")).await;

    let dropped = vec![
        Turn::new(Role::User, "the project codename is Heron"),
        Turn::new(Role::Assistant, "Noted: Heron."),
    ];
    let pack = compaction_work(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        "t",
        dropped,
    )
    .await;
    assert!(pack.expect("a pack").markdown.contains("Heron"));
    assert!(compaction_work(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        "t",
        Vec::new()
    )
    .await
    .is_none());
}

#[tokio::test]
async fn an_out_of_credits_recall_tells_the_turn_memory_is_unavailable() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::out_of_credits().bind(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));

    let pack = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t", 0, "what colour do I like?"),
    )
    .await
    .expect("a refused recall still gives the turn a notice");

    assert_eq!(pack.refusal, Some("INSUFFICIENT_CREDITS"));
    assert!(
        pack.markdown.contains("out of credits"),
        "{}",
        pack.markdown
    );
    assert!(pack.markdown.contains("does not mean nothing is stored"));
    assert!(pack.refs.is_empty() && pack.citations.is_empty());
    assert!(pack.tokens > 0);
}

#[tokio::test]
async fn an_unreachable_engine_is_named_as_unreachable() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::with(tinymemory_api::Error::Unavailable(
        "connection refused".into(),
    ))
    .bind(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));

    let pack = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t", 0, "hello"),
    )
    .await
    .expect("notice");

    assert_eq!(pack.refusal, Some("UNAVAILABLE"));
    assert!(
        pack.markdown.contains("could not be reached"),
        "{}",
        pack.markdown
    );
}

#[tokio::test]
async fn an_engine_fault_that_is_not_account_wide_injects_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::with(tinymemory_api::Error::Engine(
        "index corrupt".into(),
    ))
    .bind(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));

    assert!(run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t", 0, "hello")
    )
    .await
    .is_none());
}

// ── the pack's token budget against a large store (#6718, #7023) ────────────

async fn fill_with_learnings(engine: &tinymemory_api::conformance::ReferenceEngine, count: usize) {
    for i in 0..count {
        engine
            .store(StoreItem::learning(
                format!(
                    "Project note {i}: the Lisbon office ships release {i} on a Thursday, \
                     reviewed by team {} with a rollback window of {} hours.",
                    i % 17,
                    i % 9 + 1
                ),
                LearningKind::Fact,
                0.8,
                MemoryMeta::default(),
            ))
            .await
            .unwrap();
    }
}

fn budget(config: &crate::config::Config) -> usize {
    config.memory.recall.budget_tokens as usize
}

/// Lines that appear more than once in a rendered pack (headings aside).
fn repeated_lines(markdown: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    markdown
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("- "))
        .filter(|line| !seen.insert(line.to_string()))
        .map(str::to_string)
        .collect()
}

#[tokio::test]
async fn a_turn_pack_stays_within_its_budget_against_a_large_store() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    fill_with_learnings(&engine, 1500).await;
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));

    let pack = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t-large", 0, "when does the Lisbon release ship?"),
    )
    .await
    .expect("a pack");
    eprintln!(
        "large store: 1500 learnings -> pack {} tokens (budget {}), {} refs",
        pack.tokens,
        budget(&config),
        pack.refs.len()
    );
    // The budget check only means something if real learnings were recalled.
    assert!(
        pack.refusal.is_none(),
        "not a refusal notice: {}",
        pack.markdown
    );
    assert!(!pack.refs.is_empty(), "the pack recalled something");
    assert!(pack.markdown.contains("Project note"), "{}", pack.markdown);
    assert!(
        pack.tokens <= budget(&config),
        "{} > {}",
        pack.tokens,
        budget(&config)
    );
    assert!(repeated_lines(&pack.markdown).is_empty());
}

#[tokio::test]
async fn a_resumed_session_pack_stays_within_one_budget_and_repeats_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    fill_with_learnings(&engine, 1500).await;
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));
    // Earlier turns of the thread, logged the way a session logs them. Each
    // line is distinct, so a repeat can only be one item injected twice.
    for turn in 0..6u32 {
        let _ = run_pre_turn(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            &identity,
            input(
                "t-resumed",
                turn * 2,
                &format!("how is Lisbon release step {turn} going?"),
            ),
        )
        .await;
        run_post_turn(
            &config,
            &identity,
            reply(
                "t-resumed",
                turn * 2 + 1,
                &format!("Step {turn} is done; it ships on Thursday."),
            ),
        )
        .await;
    }

    let mut resumed = input("t-resumed", 12, "when does the Lisbon release ship?");
    resumed.5 = true;
    resumed.3 = 12;
    let pack = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        resumed,
    )
    .await
    .expect("a pack");
    eprintln!(
        "resumed session: pack {} tokens (budget {}), repeated lines {}",
        pack.tokens,
        budget(&config),
        repeated_lines(&pack.markdown).len()
    );
    assert!(
        pack.refusal.is_none(),
        "not a refusal notice: {}",
        pack.markdown
    );
    // The compacted-out turns lead the pack, so the session can resume.
    assert!(
        pack.markdown.contains("it ships on Thursday."),
        "{}",
        pack.markdown
    );
    assert!(
        pack.tokens <= budget(&config),
        "{} > {}",
        pack.tokens,
        budget(&config)
    );
    let repeated = repeated_lines(&pack.markdown);
    assert!(repeated.is_empty(), "lines injected twice: {repeated:?}");
}

#[tokio::test]
async fn a_resumed_session_without_logging_still_opens_with_the_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    let _engine = bind_reference(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let _ = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t-quiet", 0, "where is the Lisbon offsite?"),
    )
    .await;
    run_post_turn(
        &config,
        &identity,
        reply("t-quiet", 1, "The offsite is at the Alfama hotel."),
    )
    .await;

    config.memory.conversations.enabled = false;
    let reader = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let mut resumed = input("t-quiet", 2, "remind me where the offsite is");
    resumed.5 = true;
    resumed.3 = 2;
    let pack = run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &reader,
        resumed,
    )
    .await
    .expect("a pack");
    assert!(pack.markdown.contains("Alfama hotel"), "{}", pack.markdown);
    assert!(repeated_lines(&pack.markdown).is_empty());
}

#[tokio::test]
async fn a_blank_turn_is_refused_without_a_pack_with_or_without_logging() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    let engine = bind_reference(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert!(run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        input("t-blank", 0, "   ")
    )
    .await
    .is_none());
    assert!(
        stored(&engine, MetaFilter::kinds([ItemKind::Conversation]))
            .await
            .is_empty(),
        "a refused turn logs nothing"
    );

    config.memory.conversations.enabled = false;
    let reader = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert!(run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &reader,
        input("t-blank", 2, "   ")
    )
    .await
    .is_none());
    let mut resumed = input("  ", 4, "where were we?");
    resumed.5 = true;
    assert!(run_pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &reader,
        resumed
    )
    .await
    .is_none());
}

/// With `date_hint` on, the turn's text goes to the chat model with the date
/// line in the user's zone, and the pack comes back whether the model names a
/// date or answers nonsense: the hint only reorders, it never costs the pack.
#[tokio::test]
async fn a_date_hint_asks_the_model_in_the_users_zone_and_never_costs_the_pack() {
    for answer in ["{\"from\":\"2026-10-03\",\"to\":\"2026-10-03\"}", "no idea"] {
        let model = std::sync::Arc::new(tinyagents_harness::testkit::ScriptedModel::new(vec![
            tinyinference_llm::model::ModelResponse::assistant(answer),
        ]));
        let _override = crate::inference::provider::factory::test_provider_override::install_model(
            model.clone(),
        );
        let tmp = tempfile::tempdir().unwrap();
        let mut config = config_in(&tmp);
        config.memory.recall.date_hint = true;
        config.user_timezone = Some("Pacific/Chatham".into());
        let engine = bind_reference(&config);
        engine
            .store(StoreItem::learning(
                "The user's favourite colour is teal",
                LearningKind::Preference,
                0.9,
                MemoryMeta::default(),
            ))
            .await
            .unwrap();
        let identity = tinymemory_tools::layout::identity::Identity::agent("orchestrator")
            .resolve(&crate::memory::scope::identity_pins(&config));

        let pack = run_pre_turn(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            &identity,
            input("t-dated", 0, "what colour did I pick last Saturday?"),
        )
        .await
        .expect("a pack");
        assert!(
            pack.markdown.contains("teal"),
            "{answer}: {}",
            pack.markdown
        );

        let requests = model.requests();
        assert_eq!(requests.len(), 1, "{answer}: one extraction call per turn");
        let system = requests[0].messages[0].text();
        assert!(system.contains(" Pacific/Chatham ("), "{system}");
        assert!(
            requests[0].messages[1].text().contains("last Saturday"),
            "the turn's own words are what is dated"
        );
    }
}
