//! Unit tests for the live Composio catalog and the real-output probe.
//!
//! Moved here with the code they exercise. Two of them
//! (`cache_probe_result_*`, `resolve_composio_action_scope_*`) reach private
//! helpers, which is the reason they must live in this module rather than be
//! reachable from the adapter seam: widening a helper to `pub(crate)` purely so
//! a foreign test can call it would leak implementation surface to the whole
//! crate to satisfy a test.

// `super` is `catalog` itself — this file is included as its child module — so
// the glob reaches its private helpers as well as its imports.
use super::*;
use serde_json::json;

// ── compute_composio_array_path (B1: the `data` wrapper prefix) ─────────

#[test]
fn compute_composio_array_path_prefixes_data_for_an_unwrapped_payload_schema() {
    // The real shape: Composio's `output_parameters` for GMAIL_FETCH_EMAILS
    // describes the payload directly — no `data` key in the schema — but
    // the tool_call's real runtime output nests that payload one level
    // deeper under `data` (`ComposioExecuteResponse`). The array path must
    // account for that even though the schema itself never mentions `data`.
    let schema = json!({
        "type": "object",
        "properties": {
            "messages": { "type": "array" },
            "nextPageToken": { "type": "string" }
        }
    });
    assert_eq!(
        compute_composio_array_path(Some(&schema)),
        Some("data.messages".to_string())
    );
}

#[test]
fn compute_composio_array_path_still_prefixes_data_when_the_payload_schema_itself_has_a_data_key() {
    // A payload whose own real shape happens to have a top-level `data`
    // key (unrelated to Composio's wrapper — e.g. a provider that
    // itself returns `{data: {messages: [...]}}`) must NOT be mistaken
    // for "this schema already models the envelope". `output_parameters`
    // always describes the payload only (see `ToolContract::output_fields`'s
    // doc) — the real runtime path still needs the wrapper's `data.`
    // prefix stacked on top, landing on `data.data.messages`, not
    // `data.messages`.
    let schema = json!({
        "type": "object",
        "properties": {
            "data": {
                "type": "object",
                "properties": { "messages": { "type": "array" } }
            }
        }
    });
    assert_eq!(
        compute_composio_array_path(Some(&schema)),
        Some("data.data.messages".to_string())
    );
}

#[test]
fn compute_composio_array_path_none_when_the_bare_walk_finds_nothing() {
    assert_eq!(compute_composio_array_path(None), None);
    assert_eq!(
        compute_composio_array_path(Some(
            &json!({ "type": "object", "properties": { "id": { "type": "string" } } })
        )),
        None
    );
}

// ── compute_primary_array_path_from_value (B12: the real-output probe) ──

#[test]
fn compute_primary_array_path_from_value_finds_a_named_array_under_data() {
    // The exact GITHUB_LIST_REPOSITORY_ISSUES shape observed live: the
    // real array lives at `data.issues` (a NAMED field), not `data.items`
    // — and there is no schema at all to derive this from (verified live:
    // `output_schema: null` for this action), so only a real-value probe
    // can find it.
    let value = json!({
        "data": { "issues": [ { "id": 1 }, { "id": 2 } ], "total_count": 2 },
        "successful": true,
        "error": null,
        "costUsd": 0.0,
        "markdownFormatted": null
    });
    assert_eq!(
        compute_primary_array_path_from_value(&value, COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT),
        Some("data.issues".to_string())
    );
}

#[test]
fn compute_primary_array_path_from_value_skips_envelope_metadata_at_the_root() {
    // None of the envelope's OTHER top-level fields are ever arrays in
    // practice, but the skip-list is explicit so one never wins a
    // shallowest-wins tie against a real nested array.
    let value = json!({
        "successful": true,
        "error": null,
        "costUsd": 0.0,
        "markdownFormatted": null,
        "data": { "messages": ["a", "b"] }
    });
    assert_eq!(
        compute_primary_array_path_from_value(&value, COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT),
        Some("data.messages".to_string())
    );
}

#[test]
fn compute_primary_array_path_from_value_none_when_no_array_anywhere() {
    let value = json!({
        "data": { "id": "abc123", "name": "octocat" },
        "successful": true
    });
    assert_eq!(
        compute_primary_array_path_from_value(&value, COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT),
        None
    );
    assert_eq!(
        compute_primary_array_path_from_value(&json!(null), COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT),
        None
    );
    assert_eq!(
        compute_primary_array_path_from_value(
            &json!("scalar"),
            COMPOSIO_ENVELOPE_META_KEYS_AT_ROOT
        ),
        None
    );
}

// ── apply_probe_override (B12) ───────────────────────────────────────────

fn bare_contract(slug: &str) -> ToolContract {
    ToolContract {
        slug: slug.to_string(),
        toolkit: "github".to_string(),
        description: None,
        required_args: vec![],
        input_schema: None,
        output_fields: vec![],
        output_schema: None,
        primary_array_path: None,
        is_curated: true,
    }
}

#[test]
fn apply_probe_override_overlays_a_cached_probe_onto_a_schemaless_contract() {
    seed_probe_cache(
        "PROBETEST_LIST_REPOSITORY_ISSUES",
        ProbedOutputSample {
            primary_array_path: Some("data.issues".to_string()),
            output_fields: vec!["issues".to_string(), "total_count".to_string()],
            sample: json!({ "data": { "issues": [], "total_count": 0 } }),
        },
    );
    let contract = bare_contract("PROBETEST_LIST_REPOSITORY_ISSUES");
    assert_eq!(contract.primary_array_path, None);
    let overridden = apply_probe_override(contract);
    assert_eq!(
        overridden.primary_array_path,
        Some("data.issues".to_string())
    );
    assert_eq!(
        overridden.output_fields,
        vec!["issues".to_string(), "total_count".to_string()]
    );
}

#[test]
fn apply_probe_override_passes_through_unchanged_without_a_cached_probe() {
    let contract = bare_contract("PROBETEST_SOME_UNPROBED_ACTION");
    let overridden = apply_probe_override(contract.clone());
    assert_eq!(overridden.primary_array_path, contract.primary_array_path);
    assert_eq!(overridden.output_fields, contract.output_fields);
}

/// E-m8: an EXPIRED `PROBE_CACHE` entry must behave exactly like "never
/// probed" — `apply_probe_override` must NOT apply it. Before the TTL
/// fix a probe result was permanent for the process's lifetime, so a
/// corrected/changed real response stayed masked by the first-ever probe
/// until restart.
#[test]
fn apply_probe_override_ignores_an_expired_cached_probe() {
    seed_probe_cache_expired(
        "PROBETEST_EXPIRED_ACTION",
        ProbedOutputSample {
            primary_array_path: Some("data.issues".to_string()),
            output_fields: vec!["issues".to_string()],
            sample: json!({ "data": { "issues": [] } }),
        },
    );
    let contract = bare_contract("PROBETEST_EXPIRED_ACTION");
    let overridden = apply_probe_override(contract.clone());
    assert_eq!(
        overridden.primary_array_path, contract.primary_array_path,
        "an expired probe must not overlay onto the contract"
    );
    assert_eq!(overridden.output_fields, contract.output_fields);
    assert!(probed_output_sample("PROBETEST_EXPIRED_ACTION").is_none());
}

/// CodeRabbit (PR #4702 review): a probe that OBSERVED the real response
/// and found no array anywhere must CLEAR a stale schema-derived
/// `primary_array_path`, not merely leave it in place because the probe's
/// own path happens to be `None`. A schema-derived path a real
/// observation just disproved is worse than no path at all — it would
/// otherwise keep suggesting a `split_out.path` the probe itself showed
/// is wrong.
#[test]
fn apply_probe_override_clears_a_stale_schema_path_when_the_probe_finds_no_array() {
    seed_probe_cache(
        "PROBETEST_CLEARS_STALE_PATH",
        ProbedOutputSample {
            primary_array_path: None,
            output_fields: vec![],
            sample: json!({ "data": { "id": "abc123" } }),
        },
    );
    let mut contract = bare_contract("PROBETEST_CLEARS_STALE_PATH");
    contract.primary_array_path = Some("data.items".to_string());
    let overridden = apply_probe_override(contract);
    assert_eq!(overridden.primary_array_path, None);
}

/// PR #4702 review (security): the process-wide [`PROBE_CACHE`] must
/// never retain the raw observed payload — only derived metadata. A real
/// probe response can carry one user/connection/args' actual private
/// data (repo issues, messages, …), and nothing that reads the CACHE
/// (only [`apply_probe_override`], via [`probed_output_sample`]) ever
/// needs the raw payload.
#[test]
fn cache_probe_result_redacts_the_raw_sample_before_caching() {
    cache_probe_result(
        "PROBETEST_REDACTS_SAMPLE",
        ProbedOutputSample {
            primary_array_path: Some("data.issues".to_string()),
            output_fields: vec!["issues".to_string()],
            sample: json!({ "data": { "issues": [{"secret": "do-not-retain"}] } }),
        },
    );
    let cached = probed_output_sample("PROBETEST_REDACTS_SAMPLE").expect("just cached this slug");
    assert_eq!(cached.sample, Value::Null);
    // The derived metadata is still cached faithfully — only the raw
    // payload is redacted.
    assert_eq!(cached.primary_array_path, Some("data.issues".to_string()));
}

// ── resolve_composio_action_scope (B12: hard Read-only gate) ─────────────

#[test]
fn resolve_composio_action_scope_uses_the_curated_catalog_when_available() {
    use crate::openhuman::integrations::composio::providers::ToolScope;
    // GITHUB_LIST_REPOSITORY_ISSUES is curated as Read (github/tools.rs).
    assert_eq!(
        resolve_composio_action_scope("GITHUB_LIST_REPOSITORY_ISSUES"),
        Some(ToolScope::Read)
    );
    // A curated Write action must classify as Write, not Read — the probe
    // must refuse it regardless of the verb heuristic agreeing or not.
    assert_eq!(
        resolve_composio_action_scope("GMAIL_SEND_EMAIL"),
        Some(ToolScope::Write)
    );
}

/// PR #4702 review (P1): a toolkit with a static curated catalog (like
/// `github`) must NOT fall through to the `classify_unknown` verb
/// heuristic for a slug that isn't actually one of its curated actions —
/// `GITHUB_LIST_WORKFLOWS` is a REAL GitHub action name (reads as
/// Read-scope by its `LIST` verb) that was deliberately left uncurated
/// (see the commented-out entry in `github/tools.rs`), so this must
/// resolve to `None` (fail closed), not `Some(ToolScope::Read)` — the
/// heuristic agreeing with the "looks safe" name is exactly the
/// misclassification hole this guards against.
#[test]
fn resolve_composio_action_scope_rejects_an_uncurated_slug_on_a_cataloged_toolkit() {
    assert_eq!(resolve_composio_action_scope("GITHUB_LIST_WORKFLOWS"), None);
}

#[test]
fn resolve_composio_action_scope_falls_back_to_the_verb_heuristic_only_without_a_static_catalog() {
    use crate::openhuman::integrations::composio::providers::ToolScope;
    assert_eq!(
        resolve_composio_action_scope("MADEUPTOOLKIT_LIST_THINGS"),
        Some(ToolScope::Read)
    );
    assert_eq!(
        resolve_composio_action_scope("MADEUPTOOLKIT_DELETE_THING"),
        Some(ToolScope::Admin)
    );
}

// ── probe_tool_output_sample (B12: gates) ────────────────────────────────

#[tokio::test]
async fn probe_tool_output_sample_refuses_a_non_read_action_before_any_client_call() {
    let config = Config::default();
    let result = probe_tool_output_sample(&config, "GMAIL_SEND_EMAIL", json!({})).await;
    let err = result.expect_err("a Write action must be refused");
    assert!(err.contains("READ-only"), "{err}");
}

/// PR #4702 review (P1): the probe entry point itself must refuse an
/// uncurated-but-read-sounding slug on a cataloged toolkit BEFORE any
/// client call — not just `resolve_composio_action_scope` in isolation.
#[tokio::test]
async fn probe_tool_output_sample_refuses_an_uncurated_slug_on_a_cataloged_toolkit_before_any_client_call(
) {
    let config = Config::default();
    let result = probe_tool_output_sample(&config, "GITHUB_LIST_WORKFLOWS", json!({})).await;
    let err = result.expect_err("an uncurated slug on a cataloged toolkit must be refused");
    assert!(err.contains("could not confirm"), "{err}");
}

// ── fetch_live_toolkit_catalog / composio_required_args /
//    composio_response_fields delegation ─────────────────────────────────

fn contract(slug: &str, toolkit: &str, required: &[&str], output_fields: &[&str]) -> ToolContract {
    let output_schema = if output_fields.is_empty() {
        None
    } else {
        Some(json!({
            "type": "object",
            "properties": output_fields
                .iter()
                .map(|f| (f.to_string(), json!({ "type": "string" })))
                .collect::<serde_json::Map<String, Value>>()
        }))
    };
    ToolContract {
        slug: slug.to_string(),
        toolkit: toolkit.to_string(),
        description: None,
        required_args: required.iter().map(|s| s.to_string()).collect(),
        input_schema: None,
        output_fields: output_fields.iter().map(|s| s.to_string()).collect(),
        output_schema,
        primary_array_path: None,
        is_curated: false,
    }
}

#[tokio::test]
async fn fetch_live_toolkit_catalog_returns_the_seeded_cache_without_a_network_call() {
    let config = Config::default();
    seed_live_catalog_cache(
        "flowscatalogkit",
        vec![contract(
            "FLOWSCATALOGKIT_DO_THING",
            "flowscatalogkit",
            &["to"],
            &["id", "threadId"],
        )],
    );

    let catalog = fetch_live_toolkit_catalog(&config, "flowscatalogkit")
        .await
        .expect("seeded catalog must be returned without a network call");
    assert_eq!(catalog.len(), 1);
    assert_eq!(catalog[0].slug, "FLOWSCATALOGKIT_DO_THING");

    // Case/whitespace-insensitive on the toolkit key.
    let same = fetch_live_toolkit_catalog(&config, "  FlowsCatalogKit  ")
        .await
        .expect("cache lookup is case/whitespace-insensitive");
    assert_eq!(same.len(), 1);
}

#[tokio::test]
async fn composio_required_args_and_response_fields_delegate_to_the_live_catalog() {
    let config = Config::default();
    seed_live_catalog_cache(
        "flowsreqkit",
        vec![contract(
            "FLOWSREQKIT_SEND",
            "flowsreqkit",
            &["to", "body"],
            &["id", "threadId"],
        )],
    );

    assert_eq!(
        composio_required_args(&config, "FLOWSREQKIT_SEND").await,
        Some(vec!["to".to_string(), "body".to_string()])
    );
    assert_eq!(
        composio_response_fields(&config, "FLOWSREQKIT_SEND").await,
        Some(vec!["id".to_string(), "threadId".to_string()])
    );

    // An unknown slug within a known/seeded toolkit yields None (not a
    // panic, not an empty-vec false positive).
    assert_eq!(
        composio_required_args(&config, "FLOWSREQKIT_UNKNOWN_ACTION").await,
        None
    );
    assert_eq!(
        composio_response_fields(&config, "FLOWSREQKIT_UNKNOWN_ACTION").await,
        None
    );
}

#[tokio::test]
async fn composio_response_fields_distinguishes_unknown_schema_from_empty_fields() {
    let config = Config::default();

    // Schema KNOWN but empty (`properties: {}`) → `Some(vec![])`.
    seed_live_catalog_cache(
        "flowsschemaempty",
        vec![{
            let mut c = contract("FLOWSSCHEMAEMPTY_ACTION", "flowsschemaempty", &[], &[]);
            c.output_schema = Some(json!({ "type": "object", "properties": {} }));
            c
        }],
    );
    assert_eq!(
        composio_response_fields(&config, "FLOWSSCHEMAEMPTY_ACTION").await,
        Some(Vec::new()),
        "schema known but empty must be Some(vec![]), not None"
    );

    // Schema UNKNOWN (`output_schema: None`, the degrade-gracefully case)
    // → `None`, even though the slug itself is found in the catalog.
    seed_live_catalog_cache(
        "flowsschemaunknown",
        vec![contract(
            "FLOWSSCHEMAUNKNOWN_ACTION",
            "flowsschemaunknown",
            &[],
            &[],
        )],
    );
    assert_eq!(
        composio_response_fields(&config, "FLOWSSCHEMAUNKNOWN_ACTION").await,
        None,
        "an action with no published output schema must be None, not Some(vec![])"
    );
}
