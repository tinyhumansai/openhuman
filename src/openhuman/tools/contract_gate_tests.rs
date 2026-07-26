//! Unit tests for the contract gate (#4853).
//!
//! Hermetic by construction, per target kind: the Composio tests run against the
//! process-level live-catalog cache (seeded via [`seed_live_catalog_cache`], each
//! with a unique toolkit slug so the shared `LIVE_CATALOG_CACHE` can't
//! cross-contaminate); the MCP tests resolve against a synthetic connected-tools
//! snapshot; the workflow tests read a workflow seeded into a temp workspace. No
//! client is built and no network call is made anywhere here.

use super::{consult, ContractGate, GateDecision, GateTarget, GatedContract};
use crate::openhuman::config::Config;
use crate::openhuman::integrations::composio::catalog::{seed_live_catalog_cache, ToolContract};

/// Consult the gate for a Composio action slug — the shape every test in the
/// Composio block below uses.
async fn consult_composio(
    gate: &ContractGate,
    config: &Config,
    slug: &str,
    args: &serde_json::Value,
) -> GateDecision {
    consult(
        gate,
        Some(config),
        &GateTarget::Composio(slug.to_string()),
        args,
    )
    .await
}

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
    match consult_composio(&gate, &config, slug, &guessing_args()).await {
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
            consult_composio(&gate, &config, slug, &guessing_args()).await,
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
            consult_composio(&gate, &config, "PARTIALKIT_FETCH_EMAILS", &guessing_args()).await,
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
        consult_composio(&gate, &config, fetch, &guessing_args()).await,
        GateDecision::Surface(_)
    ));
    assert!(matches!(
        consult_composio(&gate, &config, send, &guessing_args()).await,
        GateDecision::Surface(_)
    ));
    assert!(matches!(
        consult_composio(&gate, &config, fetch, &guessing_args()).await,
        GateDecision::Proceed
    ));
    assert!(matches!(
        consult_composio(&gate, &config, send, &guessing_args()).await,
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
            consult_composio(&gate, &config, slug, &valid).await,
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
            consult_composio(&gate, &config, slug, &valid).await,
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
            consult_composio(&gate, &config, slug, &valid).await,
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
            consult_composio(&gate, &config, slug, &missing).await,
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
            consult_composio(&gate, &config, unknown_slug, &invented).await,
            GateDecision::Surface(_)
        ),
        "an unknown/hallucinated key must surface the contract"
    );

    // `max_results` is an integer; an array is a genuine type error → surface.
    let mistyped = serde_json::json!({ "max_results": [1, 2, 3] });
    assert!(
        matches!(
            consult_composio(&gate, &config, mistyped_slug, &mistyped).await,
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
        let decision = consult_composio(&gate, &config, slug, &guessing_args()).await;
        assert!(
            matches!(decision, GateDecision::Surface(_)),
            "fresh gate {i} must surface the contract (count {i})"
        );
    }

    // Gate 4: auto-proceeds because 3+ fresh instances have already surfaced
    // this contract without any of them executing.
    let gate4 = ContractGate::new();
    let decision = consult_composio(&gate4, &config, slug, &guessing_args()).await;
    assert!(
        matches!(decision, GateDecision::Proceed),
        "gate 4 must auto-proceed after 3+ fresh instances surfaced the same slug"
    );
}

// ── target identity (kind-agnostic, compiled in every configuration) ────────

/// The seen-set and the process-wide consult counter are keyed by
/// [`GateTarget::key`], so a workflow, an MCP tool, and a Composio action that
/// happen to share a name must never credit each other's gate.
#[test]
fn target_keys_are_namespaced_per_kind() {
    let composio = GateTarget::Composio("shared".to_string());
    let mcp = GateTarget::McpRegistry {
        server: "srv".to_string(),
        tool: "shared".to_string(),
    };
    let workflow = GateTarget::Workflow("shared".to_string());

    let keys = [composio.key(), mcp.key(), workflow.key()];
    assert_eq!(
        keys.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "each kind must own its key namespace, got: {keys:?}"
    );

    // A Composio slug is case-folded (the model's casing varies, the action
    // does not); the other kinds address exact ids and must not be folded.
    assert_eq!(
        GateTarget::Composio("gmail_fetch".to_string()).key(),
        GateTarget::Composio("GMAIL_FETCH".to_string()).key()
    );
    assert_ne!(
        GateTarget::Workflow("Triage".to_string()).key(),
        GateTarget::Workflow("triage".to_string()).key()
    );
}

/// A surfaced contract must name the exact call to re-issue. A model told to
/// "call the action again" after a blocked `run_workflow` has no idea which
/// tool that means.
#[test]
fn surfaced_contract_names_the_call_to_re_issue() {
    let contract = GatedContract {
        description: Some("Summarise the inbox.".to_string()),
        required_args: vec!["mailbox".to_string()],
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": { "mailbox": { "type": "string" } },
            "required": ["mailbox"]
        })),
    };

    let workflow = super::format_contract(&GateTarget::Workflow("triage".to_string()), &contract);
    assert!(workflow.contains("workflow `triage`"), "got: {workflow}");
    assert!(workflow.contains("`run_workflow`"), "got: {workflow}");
    assert!(workflow.contains("Required arguments: mailbox"));

    let mcp = super::format_contract(
        &GateTarget::McpRegistry {
            server: "srv".to_string(),
            tool: "search".to_string(),
        },
        &contract,
    );
    assert!(
        mcp.contains("MCP tool `search` on server `srv`"),
        "got: {mcp}"
    );
    assert!(mcp.contains("`mcp_registry_tool_call`"), "got: {mcp}");
}

/// `connection_id` is injected by OpenHuman onto Composio calls and is absent
/// from the provider's published schema, so it must not read as an invented
/// key — but that exemption is Composio's alone. Nothing injects extra keys
/// onto an MCP or workflow call, so an unknown key there is a real guess.
#[test]
fn the_injected_arg_exemption_is_scoped_to_composio() {
    let contract = GatedContract {
        description: None,
        required_args: Vec::new(),
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string" } }
        })),
    };
    let args = serde_json::json!({ "query": "hi", "connection_id": "ca_1" });

    assert!(super::args_satisfy_contract(
        &args,
        &contract,
        GateTarget::Composio("X_Y".to_string()).injected_arg_keys()
    ));
    assert!(
        !super::args_satisfy_contract(
            &args,
            &contract,
            GateTarget::Workflow("w".to_string()).injected_arg_keys()
        ),
        "a workflow has no injected keys, so `connection_id` is an unknown arg"
    );
}

// ── MCP registry surface ────────────────────────────────────────────────────

/// The remote server publishes a JSON Schema; the gate must read its `required`
/// array as the required-arg list so a call missing one is surfaced rather than
/// dispatched into a server-side validation error.
#[cfg(feature = "mcp")]
#[test]
fn an_mcp_contract_is_resolved_from_the_advertised_schema() {
    use crate::openhuman::mcp::registry::types::McpTool;

    let connected = vec![(
        "srv".to_string(),
        "srv".to_string(),
        McpTool {
            name: "search".to_string(),
            description: Some("Search the corpus.".to_string()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "q": { "type": "string" }, "limit": { "type": "integer" } },
                "required": ["q"]
            }),
        },
    )];

    let contract = super::resolve_mcp_contract(connected.clone(), "srv", "search")
        .expect("the advertised tool resolves");
    assert_eq!(contract.required_args, vec!["q".to_string()]);
    assert_eq!(contract.description.as_deref(), Some("Search the corpus."));

    let injected = GateTarget::McpRegistry {
        server: "srv".to_string(),
        tool: "search".to_string(),
    };
    // Missing the required `q` → the model guessed → surface.
    assert!(!super::args_satisfy_contract(
        &serde_json::json!({ "limit": 5 }),
        &contract,
        injected.injected_arg_keys()
    ));
    // Conforming args → execute directly, no bounce.
    assert!(super::args_satisfy_contract(
        &serde_json::json!({ "q": "rust", "limit": 5 }),
        &contract,
        injected.injected_arg_keys()
    ));

    // A tool on another server, or an unknown tool, resolves to nothing — the
    // gate then proceeds instead of blocking on a contract it cannot show.
    assert!(super::resolve_mcp_contract(connected.clone(), "other", "search").is_none());
    assert!(super::resolve_mcp_contract(connected, "srv", "nope").is_none());
}

// ── Workflow surface ────────────────────────────────────────────────────────

/// Seed a trusted project-scope workflow with one required and one optional
/// input, and return a `Config` whose workspace is that temp dir. Inputs are
/// declared in `skill.toml` (the SKILL.md body is the system prompt), matching
/// how `load_workflows` reads a workflow directory.
#[cfg(feature = "skills")]
fn seed_workflow(ws: &std::path::Path, id: &str) -> Config {
    let skill_dir = ws.join(".openhuman").join("skills").join(id);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    std::fs::write(ws.join(".openhuman").join("trust"), "").expect("trust the workspace");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        format!("---\nname: {id}\ndescription: Summarise the inbox.\n---\n\nSummarise.\n"),
    )
    .expect("write SKILL.md");
    std::fs::write(
        skill_dir.join("skill.toml"),
        format!(
            "id = \"{id}\"\nwhen_to_use = \"Use to triage a mailbox.\"\n\n\
             [[inputs]]\nname = \"mailbox\"\ndescription = \"Which mailbox to triage.\"\n\
             required = true\ntype = \"string\"\n\n\
             [[inputs]]\nname = \"limit\"\ndescription = \"How many threads.\"\n\
             required = false\ntype = \"integer\"\n"
        ),
    )
    .expect("write skill.toml");

    let mut config = Config::default();
    config.workspace_dir = ws.to_path_buf();
    config
}

/// A `run_workflow` that omits a required input must be handed the workflow's
/// declared `[[inputs]]` contract — the same treatment a Composio action gets —
/// and the retry that supplies it must execute.
#[cfg(feature = "skills")]
#[tokio::test]
async fn a_workflow_run_missing_a_required_input_surfaces_the_contract() {
    let ws = tempfile::tempdir().expect("tempdir");
    let config = seed_workflow(ws.path(), "triage-inbox");
    let target = GateTarget::Workflow("triage-inbox".to_string());
    let gate = ContractGate::new();

    match consult(&gate, Some(&config), &target, &serde_json::json!({})).await {
        GateDecision::Surface(message) => {
            assert!(
                message.contains("workflow `triage-inbox`"),
                "contract names the workflow, got: {message}"
            );
            assert!(
                message.contains("mailbox"),
                "contract carries the declared inputs, got: {message}"
            );
            assert!(
                message.contains("Required arguments: mailbox"),
                "contract lists the required inputs, got: {message}"
            );
            assert!(
                message.contains("`run_workflow`"),
                "contract names the call to re-issue, got: {message}"
            );
        }
        GateDecision::Proceed => panic!("a run missing a required input must be surfaced"),
    }

    // Same gate instance, retry with the input supplied: already seen → run.
    assert!(
        matches!(
            consult(
                &gate,
                Some(&config),
                &target,
                &serde_json::json!({ "mailbox": "INBOX" })
            )
            .await,
            GateDecision::Proceed
        ),
        "the retry must execute"
    );
}

/// Validate-then-pass on the workflow surface: a first call that already covers
/// every required input runs immediately. Bouncing it would cost a turn and
/// teach the model nothing.
#[cfg(feature = "skills")]
#[tokio::test]
async fn a_workflow_run_with_satisfying_inputs_proceeds_without_surfacing() {
    let ws = tempfile::tempdir().expect("tempdir");
    let config = seed_workflow(ws.path(), "triage-direct");
    let gate = ContractGate::new();

    assert!(
        matches!(
            consult(
                &gate,
                Some(&config),
                &GateTarget::Workflow("triage-direct".to_string()),
                &serde_json::json!({ "mailbox": "INBOX", "limit": 5 })
            )
            .await,
            GateDecision::Proceed
        ),
        "conforming inputs must dispatch on the first call"
    );
}

/// Without a config the gate has no workspace to resolve a workflow against.
/// It must degrade to "proceed", never block a run it cannot explain.
#[cfg(feature = "skills")]
#[tokio::test]
async fn a_workflow_run_without_a_config_proceeds() {
    let gate = ContractGate::new();
    assert!(matches!(
        consult(
            &gate,
            None,
            &GateTarget::Workflow("anything".to_string()),
            &serde_json::json!({})
        )
        .await,
        GateDecision::Proceed
    ));
}

// ── transcript presence ─────────────────────────────────────────────────────

/// A delivered contract is creditable only while its payload survives
/// byte-for-byte. Reformatting it — a summarizer, a size cap, the sub-agent
/// handoff's whitespace collapse — must drop it back to absent so the gate
/// re-delivers, rather than let the model call a tool whose schema it can no
/// longer read.
#[test]
fn a_rewritten_payload_stops_counting_as_present() {
    let key = "composio:PRESENCEKIT_FETCH".to_string();
    let (delivered, _) = super::seed_delivery(&key, "the full contract body");

    assert!(
        super::credited_slugs([delivered.clone()]).contains(&key),
        "the delivery as-sent must credit its slug"
    );

    // Same marker, payload whitespace-collapsed the way the handoff cleaner
    // rewrites it. The hash no longer matches, so it must not credit.
    let mangled = delivered.replace("\n\n", " ");
    assert!(
        !super::credited_slugs([mangled]).contains(&key),
        "a rewritten payload must not be credited"
    );
}

/// A message that merely *looks* like a delivery must not be able to veto a
/// genuine one. If the scan treated the first hash miss as "this target is
/// absent" and stopped, one lookalike anywhere in the transcript would make the
/// contract permanently un-creditable and the gate would re-deliver it on every
/// single call — an infinite retry loop.
#[test]
fn a_lookalike_marker_cannot_suppress_a_genuine_delivery() {
    let key = "composio:LOOKALIKEKIT_FETCH".to_string();
    let (delivered, slug_list) = super::seed_delivery(&key, "the full contract body");

    // Same slug list, wrong payload: a tool echoing the marker syntax, or a
    // stale copy from an earlier delivery.
    let lookalike = format!("[contract-gate:{slug_list}]\n\nnot the contract that was delivered");
    assert!(
        !super::credited_slugs([lookalike.clone()]).contains(&key),
        "the lookalike alone must not credit"
    );

    // Scanned together, in either order, the genuine copy still credits.
    for (label, texts) in [
        (
            "lookalike first",
            vec![lookalike.clone(), delivered.clone()],
        ),
        ("genuine first", vec![delivered.clone(), lookalike.clone()]),
    ] {
        assert!(
            super::credited_slugs(texts).contains(&key),
            "{label}: a genuine delivery must still credit past a lookalike"
        );
    }
}

/// One marker can carry several slugs (a full-schema discovery listing that
/// described many targets), and crediting it must credit every one of them —
/// that is what lets a `describe_workflow` / `*_list_tools` before the real call
/// skip a redundant re-delivery.
#[test]
fn a_multi_slug_marker_credits_every_slug_it_names() {
    let body = "two full contracts";
    let marked = super::prefix_with_present_marker(
        [
            "workflow:beta".to_string(),
            "workflow:alpha".to_string(),
            // Duplicated + empty entries must not corrupt the list.
            "workflow:alpha".to_string(),
            String::new(),
        ],
        body,
    );

    // Slugs are sorted and de-duplicated, and the marker still leads the message
    // so the fixed-prefix scan finds it at byte 0.
    assert!(
        marked.starts_with("[contract-gate:workflow:alpha,workflow:beta]"),
        "got: {marked}"
    );
    assert!(marked.contains(body), "the body still follows the marker");

    let credited = super::credited_slugs([marked]);
    assert!(credited.contains("workflow:alpha"));
    assert!(credited.contains("workflow:beta"));
}

/// Presence is decided by a fixed-prefix compare at byte 0, so a marker the
/// model quotes mid-sentence cannot claim a contract is in context.
#[test]
fn a_marker_that_does_not_lead_the_message_is_ignored() {
    let key = "composio:PREFIXKIT_FETCH".to_string();
    let (delivered, _) = super::seed_delivery(&key, "the full contract body");

    let quoted = format!("Here is what I saw earlier: {delivered}");
    assert!(
        !super::credited_slugs([quoted]).contains(&key),
        "a marker not at byte 0 must not credit"
    );
}
