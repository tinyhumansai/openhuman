//! Unit tests for the Composio contract gate (#4853).
//!
//! These exercise the gate purely against the process-level live-catalog cache
//! (seeded via [`seed_live_catalog_cache`]), so no Composio client is built and
//! no network call is made. Each test uses a unique toolkit slug so the shared
//! `LIVE_CATALOG_CACHE` can't cross-contaminate between tests.

use super::{consult, ContractGate, GateDecision};
use crate::openhuman::config::Config;
use crate::openhuman::integrations::composio::catalog::{seed_live_catalog_cache, ToolContract};

/// Build a full contract for `slug` in `toolkit` with a REQUIRED `query` input
/// field and a description that spells out the quoting rule — the exact detail
/// the model misses when it only sees the thin spawn-time schema.
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

/// Build a contract with only OPTIONAL, typed args — mirrors the real
/// `GMAIL_FETCH_EMAILS` shape at the heart of #5119 (no required args; the model
/// supplies `label_ids`/`max_results`/`verbose`). Used to prove validate-then-pass.
fn fetch_contract(slug: &str, toolkit: &str) -> ToolContract {
    ToolContract {
        slug: slug.to_string(),
        toolkit: toolkit.to_string(),
        description: Some("Fetch emails from the inbox.".to_string()),
        required_args: Vec::new(),
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {
                "label_ids": { "type": "array", "items": { "type": "string" } },
                "max_results": { "type": "integer" },
                "verbose": { "type": "boolean" },
                "query": { "type": "string" }
            }
        })),
        output_fields: Vec::new(),
        output_schema: None,
        primary_array_path: None,
        is_curated: false,
    }
}

/// Args that do NOT satisfy [`full_contract`] (its required `query` is absent),
/// so the gate surfaces the contract — the "model guessed / needs the schema"
/// path these legacy tests exercise.
fn guessing_args() -> serde_json::Value {
    serde_json::json!({})
}

#[tokio::test]
async fn first_call_surfaces_full_contract_then_retry_proceeds() {
    // Toolkit derived from the slug prefix: `GMAILGATE_...` -> `gmailgate`.
    let toolkit = "gmailgate";
    let slug = "GMAILGATE_FETCH_EMAILS";
    seed_live_catalog_cache(toolkit, vec![full_contract(slug, toolkit)]);

    let config = Config::default();
    let gate = ContractGate::new();

    // First call with args that MISS the required `query`: the gate
    // short-circuits execution and hands back the full contract.
    match consult(&gate, &config, slug, &guessing_args()).await {
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
        matches!(
            consult(&gate, &config, slug, &guessing_args()).await,
            GateDecision::Proceed
        ),
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
            consult(&gate, &config, "PARTIALKIT_FETCH_EMAILS", &guessing_args()).await,
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

    // Each action (args miss the required `query`) surfaces its own contract
    // exactly once, independently.
    assert!(matches!(
        consult(&gate, &config, fetch, &guessing_args()).await,
        GateDecision::Surface(_)
    ));
    assert!(matches!(
        consult(&gate, &config, send, &guessing_args()).await,
        GateDecision::Surface(_)
    ));
    assert!(matches!(
        consult(&gate, &config, fetch, &guessing_args()).await,
        GateDecision::Proceed
    ));
    assert!(matches!(
        consult(&gate, &config, send, &guessing_args()).await,
        GateDecision::Proceed
    ));
}

// ── #5119: validate-then-pass — a well-formed first call must NOT be bounced ──

#[tokio::test]
async fn first_call_with_satisfying_args_proceeds_without_surfacing() {
    // The exact #5119 scenario: "fetch my latest email" → the model's FIRST
    // call already carries schema-valid args. Bouncing it forces a needless
    // retry that a weak text-mode model corrupts, looping forever. The gate must
    // execute immediately instead.
    let toolkit = "fetchok";
    let slug = "FETCHOK_FETCH_EMAILS";
    seed_live_catalog_cache(toolkit, vec![fetch_contract(slug, toolkit)]);

    let config = Config::default();
    let gate = ContractGate::new();

    let valid = serde_json::json!({ "label_ids": ["INBOX"], "max_results": 1, "verbose": true });
    assert!(
        matches!(
            consult(&gate, &config, slug, &valid).await,
            GateDecision::Proceed
        ),
        "a first call whose args already satisfy the contract must execute, not surface"
    );
}

#[tokio::test]
async fn satisfied_required_arg_executes_immediately() {
    // A required arg that IS present (and typed correctly) also passes on the
    // first call — the gate only surfaces when the model actually guessed.
    let toolkit = "reqok";
    let slug = "REQOK_SEARCH";
    seed_live_catalog_cache(toolkit, vec![full_contract(slug, toolkit)]);

    let config = Config::default();
    let gate = ContractGate::new();

    let valid = serde_json::json!({ "query": "subject:\"quarterly report\"" });
    assert!(
        matches!(
            consult(&gate, &config, slug, &valid).await,
            GateDecision::Proceed
        ),
        "a satisfied required arg must proceed on the first call"
    );
}

#[tokio::test]
async fn synthetic_connection_id_does_not_bounce_a_valid_call() {
    // #5119 review: `connection_id` is an OpenHuman-injected routing parameter
    // (added by `ComposioActionTool::parameters_schema` / `ComposioExecuteTool`
    // and consumed before dispatch), NOT a field in Composio's live catalog
    // `input_schema`. A valid multi-account first call carries it, so the
    // unknown-key check must skip it rather than bounce the call into the retry
    // path this gate exists to avoid.
    let toolkit = "connkit";
    let slug = "CONNKIT_FETCH_EMAILS";
    seed_live_catalog_cache(toolkit, vec![fetch_contract(slug, toolkit)]);

    let config = Config::default();
    let gate = ContractGate::new();

    let valid = serde_json::json!({
        "label_ids": ["INBOX"],
        "max_results": 1,
        "connection_id": "conn_abc123"
    });
    assert!(
        matches!(
            consult(&gate, &config, slug, &valid).await,
            GateDecision::Proceed
        ),
        "a valid call carrying the synthetic connection_id must execute, not surface"
    );
}

#[tokio::test]
async fn missing_required_arg_surfaces() {
    let toolkit = "missreq";
    let slug = "MISSREQ_SEARCH";
    seed_live_catalog_cache(toolkit, vec![full_contract(slug, toolkit)]);

    let config = Config::default();
    let gate = ContractGate::new();

    // `query` is required but absent → surface.
    let missing = serde_json::json!({ "verbose": true });
    assert!(
        matches!(
            consult(&gate, &config, slug, &missing).await,
            GateDecision::Surface(_)
        ),
        "a missing required arg must surface the contract"
    );
}

#[tokio::test]
async fn unknown_or_mistyped_args_surface() {
    let toolkit = "guesskit";
    let unknown_slug = "GUESSKIT_FETCH_A";
    let mistyped_slug = "GUESSKIT_FETCH_B";
    seed_live_catalog_cache(
        toolkit,
        vec![
            fetch_contract(unknown_slug, toolkit),
            fetch_contract(mistyped_slug, toolkit),
        ],
    );

    let config = Config::default();
    let gate = ContractGate::new();

    // Invented key the schema never declares → the model guessed → surface.
    let invented = serde_json::json!({ "invented_field": 1 });
    assert!(
        matches!(
            consult(&gate, &config, unknown_slug, &invented).await,
            GateDecision::Surface(_)
        ),
        "an unknown/hallucinated key must surface the contract"
    );

    // `max_results` is an integer; an array is a genuine type error → surface.
    let mistyped = serde_json::json!({ "max_results": [1, 2, 3] });
    assert!(
        matches!(
            consult(&gate, &config, mistyped_slug, &mistyped).await,
            GateDecision::Surface(_)
        ),
        "a wrong-typed arg must surface the contract"
    );
}

// ── #5119: auto-proceed safety net for re-delegated sub-agents ──────────────

#[tokio::test]
async fn fresh_gates_eventually_auto_proceed() {
    // Regression for #5119: when the main agent re-delegates to a fresh
    // integrations_agent sub-agent, each spawn creates a new ComposioActionTool
    // with a fresh ContractGate. Without a process-wide safety net, every fresh
    // gate surfaces the contract again and the action never executes, causing an
    // infinite loop (51x surfacing in the reported log).
    //
    // The safety net tracks how many *fresh gate instances* have been consulted
    // per slug. After the threshold (3+) of fresh instances have all seen the
    // slug as "first time" and surfaced the contract, the next instance
    // auto-proceeds — the model has clearly seen the schema and doesn't need it
    // surfaced again.
    let toolkit = "autopkit";
    let slug = "AUTOPKIT_FETCH_EMAILS";
    seed_live_catalog_cache(toolkit, vec![full_contract(slug, toolkit)]);

    let config = Config::default();

    // Gates 1-3 each surface the contract (fresh instance, first-time consult).
    for i in 1..=3 {
        let gate = ContractGate::new();
        let decision = consult(&gate, &config, slug, &guessing_args()).await;
        assert!(
            matches!(decision, GateDecision::Surface(_)),
            "fresh gate {i} must surface the contract (count {i})"
        );
    }

    // Gate 4: auto-proceeds because 3+ fresh instances have already surfaced
    // this contract without any of them executing.
    let gate4 = ContractGate::new();
    let decision = consult(&gate4, &config, slug, &guessing_args()).await;
    assert!(
        matches!(decision, GateDecision::Proceed),
        "gate 4 must auto-proceed after 3+ fresh instances surfaced the same slug"
    );
}
