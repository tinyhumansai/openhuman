use std::sync::Arc;

use tinyagents_harness::tool::discover::DiscoveryRankMode;
use tinytools::{Bm25Ranker, ToolRanker};

use super::*;

/// The slots are process globals; serialise the tests that touch them.
fn guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn default_policy_is_jev_with_no_ranker_and_top_three() {
    let _g = guard();
    clear_tool_ranker();
    apply_tool_search_config(&ToolSearchConfig::default());
    let policy = discovery_policy();
    assert!(policy.ranker.is_none());
    assert_eq!(policy.rank_mode, DiscoveryRankMode::Ranker);
    assert_eq!(policy.default_limit, 3);
    assert!(policy.enabled);
}

#[test]
fn settings_select_the_mode_and_clamp_top_k() {
    let _g = guard();
    let ranker: Arc<dyn ToolRanker> = Arc::new(Bm25Ranker);
    install_tool_ranker(ranker);
    for (setting, mode) in [
        ("bm25", DiscoveryRankMode::Bm25),
        ("compare", DiscoveryRankMode::Compare),
        ("jev", DiscoveryRankMode::Ranker),
        ("nonsense", DiscoveryRankMode::Ranker),
    ] {
        apply_tool_search_config(&ToolSearchConfig {
            ranker: setting.into(),
            top_k: 500,
            ..ToolSearchConfig::default()
        });
        let policy = discovery_policy();
        assert_eq!(policy.rank_mode, mode, "{setting}");
        assert_eq!(policy.default_limit, policy.max_limit, "{setting}");
        assert_eq!(policy.ranker.as_ref().map(|r| r.kind()), Some("bm25"));
    }
    clear_tool_ranker();
    apply_tool_search_config(&ToolSearchConfig::default());
}

#[tokio::test]
async fn overlap_ranker_ranks_by_token_overlap_and_names_its_kind() {
    let ranker = OverlapRanker;
    assert_eq!(ranker.kind(), "overlap");
    let candidates = vec![
        tinytools::RankCandidate::new(
            "SLACK_SEND_MESSAGE",
            "SLACK_SEND_MESSAGE send message to a channel",
        ),
        tinytools::RankCandidate::new(
            "GMAIL_FETCH_EMAILS",
            "GMAIL_FETCH_EMAILS fetch emails from inbox",
        ),
    ];
    let hits = ranker
        .rank(
            "send a message to the channel",
            &tinytools::RankContext::empty(),
            &candidates,
            3,
        )
        .await
        .unwrap();
    assert_eq!(
        hits.first().map(|h| h.key.as_str()),
        Some("SLACK_SEND_MESSAGE")
    );
    assert!(ranker
        .rank(" ", &tinytools::RankContext::empty(), &candidates, 3)
        .await
        .is_err());
}

/// A text dialect renders the prompt catalogue and the harness clears
/// `request.tools`, so the bridge only reaches the model through the host's
/// own catalogue. Without this entry the model has no signature for
/// `tool_search` — observed live as a turn that narrates the call it is about
/// to make and then stops. There is no `tool_call` wrapper: a found tool is
/// called by its own name.
#[test]
fn bridge_prompt_tools_advertise_only_search_when_something_is_deferred() {
    let _g = guard();
    apply_tool_search_config(&ToolSearchConfig::default());
    let bridge = bridge_prompt_tools(12);
    let names: Vec<&str> = bridge.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, vec!["tool_search"]);
    for tool in &bridge {
        assert!(
            tool.parameters_schema
                .as_deref()
                .is_some_and(|schema| schema.contains("\"type\":\"object\"")),
            "{} must carry a callable schema, got {:?}",
            tool.name,
            tool.parameters_schema
        );
    }
}

/// The manifest is left out on purpose: naming every deferred tool in the
/// prompt is the cost deferral exists to avoid.
#[test]
fn bridge_prompt_tools_do_not_enumerate_the_deferred_catalogue() {
    let _g = guard();
    apply_tool_search_config(&ToolSearchConfig::default());
    let search = bridge_prompt_tools(3).into_iter().next().expect("bridge");
    assert!(
        search.description.len() < 1_000,
        "the bridge description must stay a pointer to the search, not a catalogue: {} bytes",
        search.description.len()
    );
}

/// Nothing deferred, nothing to advertise: a belt that did not opt into
/// discovery must not pay two schemas for a bridge it cannot use.
#[test]
fn bridge_prompt_tools_are_empty_without_a_deferred_catalogue() {
    let _g = guard();
    assert!(bridge_prompt_tools(0).is_empty());
}

/// The live policy carries no per-tool manifest on any dialect: the harness
/// default (4,000 tokens) listed every deferred tool in `tool_search`'s
/// description on every native request, ~3.9k tokens on a workspace with a
/// few integrations connected.
#[test]
fn discovery_policy_renders_no_manifest() {
    let _g = guard();
    clear_tool_ranker();
    apply_tool_search_config(&ToolSearchConfig::default());
    assert_eq!(discovery_policy().manifest_token_budget, 0);
}

#[test]
fn a_turn_without_a_chat_gives_the_ranker_an_empty_context() {
    assert_eq!(rank_context_for(None), tinytools::RankContext::empty());
}

#[test]
fn a_chat_turn_names_its_thread_client_and_request_to_the_ranker() {
    let chat = crate::security::approval::ApprovalChatContext {
        thread_id: "thread-7".to_owned(),
        client_id: "client-3".to_owned(),
        request_id: Some("req-1".to_owned()),
    };
    let context = rank_context_for(Some(&chat));
    let routed = context.chat.expect("chat is carried");
    assert_eq!(routed.thread_id, "thread-7");
    assert_eq!(routed.client_id, "client-3");
    assert_eq!(routed.request_id.as_deref(), Some("req-1"));

    let without_request = crate::security::approval::ApprovalChatContext {
        request_id: None,
        ..chat
    };
    let routed = rank_context_for(Some(&without_request))
        .chat
        .expect("chat is carried");
    assert_eq!(routed.request_id, None);
}
