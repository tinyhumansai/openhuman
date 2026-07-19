//! Unit tests for the Composio contract gate (#4853).
//!
//! These exercise the gate purely against the process-level live-catalog cache
//! (seeded via [`seed_live_catalog_cache`]), so no Composio client is built and
//! no network call is made. Each test uses a unique toolkit slug so the shared
//! `LIVE_CATALOG_CACHE` can't cross-contaminate between tests.

use super::{consult, ContractGate, GateDecision};
use crate::openhuman::config::Config;
use crate::openhuman::tinyflows::caps::{seed_live_catalog_cache, ToolContract};

/// Build a full contract for `slug` in `toolkit` with a `query` input field and
/// a description that spells out the quoting rule — the exact detail the model
/// misses when it only sees the thin spawn-time schema.
fn full_contract(slug: &str, toolkit: &str) -> ToolContract {
    ToolContract {
        slug: slug.to_string(),
        toolkit: toolkit.to_string(),
        description: Some(
            "Search the mailbox. Multi-word phrases in `query` must be quoted, \
             e.g. subject:\"quarterly report\"."
                .to_string(),
        ),
        required_args: vec!["query".to_string()],
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Gmail search query (quote multi-word phrases)."
                }
            },
            "required": ["query"]
        })),
        output_fields: Vec::new(),
        output_schema: None,
        primary_array_path: None,
        is_curated: false,
    }
}

#[tokio::test]
async fn first_call_surfaces_full_contract_then_retry_proceeds() {
    // Toolkit derived from the slug prefix: `GMAILGATE_...` -> `gmailgate`.
    let toolkit = "gmailgate";
    let slug = "GMAILGATE_FETCH_EMAILS";
    seed_live_catalog_cache(toolkit, vec![full_contract(slug, toolkit)]);

    let config = Config::default();
    let gate = ContractGate::new();

    // First call: the gate short-circuits execution and hands back the full
    // contract (this is the behaviour that is ABSENT before the fix — the thin
    // per-action tool would execute immediately with a guessed query).
    match consult(&gate, &config, slug).await {
        GateDecision::Surface(message) => {
            assert!(message.contains(slug), "contract names the action slug");
            assert!(
                message.contains("query"),
                "contract carries the input schema"
            );
            assert!(
                message.contains("Required arguments: query"),
                "contract lists required args"
            );
            assert!(
                message.contains("quoted"),
                "contract carries the provider description explaining quoting"
            );
        }
        GateDecision::Proceed => panic!("first call must surface the contract, not execute"),
    }

    // The retry — now with the contract in context — proceeds to execution.
    assert!(
        matches!(consult(&gate, &config, slug).await, GateDecision::Proceed),
        "retry must proceed once the contract has been surfaced this turn"
    );
}

#[tokio::test]
async fn known_toolkit_but_unknown_action_proceeds_without_blocking() {
    // Toolkit is cached but does NOT contain the requested action, so no
    // fuller contract can be surfaced. The gate must degrade to Proceed rather
    // than block the call forever.
    let toolkit = "partialkit";
    seed_live_catalog_cache(
        toolkit,
        vec![full_contract("PARTIALKIT_OTHER_ACTION", toolkit)],
    );

    let config = Config::default();
    let gate = ContractGate::new();

    assert!(
        matches!(
            consult(&gate, &config, "PARTIALKIT_FETCH_EMAILS").await,
            GateDecision::Proceed
        ),
        "an action missing from the live catalog must not be gated"
    );
}

#[tokio::test]
async fn distinct_actions_are_gated_independently() {
    let toolkit = "multikit";
    let fetch = "MULTIKIT_FETCH_EMAILS";
    let send = "MULTIKIT_SEND_EMAIL";
    seed_live_catalog_cache(
        toolkit,
        vec![full_contract(fetch, toolkit), full_contract(send, toolkit)],
    );

    let config = Config::default();
    let gate = ContractGate::new();

    // Each action surfaces its own contract exactly once, independently.
    assert!(matches!(
        consult(&gate, &config, fetch).await,
        GateDecision::Surface(_)
    ));
    assert!(matches!(
        consult(&gate, &config, send).await,
        GateDecision::Surface(_)
    ));
    assert!(matches!(
        consult(&gate, &config, fetch).await,
        GateDecision::Proceed
    ));
    assert!(matches!(
        consult(&gate, &config, send).await,
        GateDecision::Proceed
    ));
}
