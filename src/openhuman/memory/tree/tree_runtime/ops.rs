//! RPC operation wrappers for the tree summarizer.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use std::sync::Arc;

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::error::MemoryError;
use crate::openhuman::memory::api::provider::{MemoryProvider, MemoryTree};
// The summary-tree node vocabulary, named at the **contract** (#5560).
//
// This was `use tinycortex::memory::tree::runtime::*;`, a glob whose entire
// live contribution to this file was two names: `estimate_tokens` (the token
// figure reported on an ingest) and `QueryResult` (the node + children envelope
// `tree_summarizer_query` serialises). Both are `tinymemory-bus` items that the
// engine crate merely re-exported — `tinycortex::memory::tree::runtime` aliases
// its `types` module to `tinycortex_api::tree`, and `tinycortex-api` is a
// deprecated re-export of `tinymemory-bus` — so this names the *same items*
// under the path the module contract already uses, and no wire byte changes.
// The sibling `tree_runtime/mod.rs` re-exports the same set for the same
// reason; see its comment on the node model.
use crate::openhuman::memory::api::tree::{estimate_tokens, QueryResult};
use crate::openhuman::memory::guard::MemoryGuard;
use crate::rpc::RpcOutcome;

// ── How these handlers reach the tree ───────────────────────────────────────
//
// Every one of them used to call `tree_runtime::{engine, store}` — the glob
// over `tinymemory_core::tree::tree_runtime` — and run the markdown time tree
// in *this* process. They ask the bound driver now, through the six runtime
// doors the contract grew for exactly this surface (#5560):
//
// | was                          | is                             |
// | ---------------------------- | ------------------------------ |
// | `store::buffer_write`        | `MemoryTree::runtime_buffer_write` |
// | `store::read_node`           | `MemoryTree::runtime_read_node`    |
// | `store::read_children`       | `MemoryTree::runtime_read_children`|
// | `store::get_tree_status`     | `MemoryTree::runtime_tree_status`  |
// | `engine::run_summarization`  | `MemoryTree::runtime_summarize`    |
// | `engine::rebuild_tree`       | `MemoryTree::runtime_rebuild`      |
//
// The doors are shaped to *this* reply, which is why they are six rather than
// the four coarser members that already existed. `seal` runs the same pass as
// `runtime_summarize` and answers with tree state; `drill_down` folds "no such
// node" into a `NotFound` of its own wording and always pays for the child
// read. Either would have changed what this RPC reports, and a door that
// changes what the host reports is a new surface rather than a migration.
//
// `store::validate_namespace` / `validate_node_id` went with them: the driver
// makes the same two refusals, in the same order, with the same message —
// which is why [`driver_error`] unwraps `Invalid` rather than rendering it.

/// The guarded driver for `config`'s workspace.
///
/// Returned as the owning `Arc` because [`MemoryProvider::as_tree`] hands back
/// a *borrow* of the guard; a helper that resolved the family in one step would
/// be returning a reference to a temporary. Each handler binds this, then asks
/// [`tree_of`] for the family.
///
/// Guarded rather than raw: these five handlers now have typed contract twins,
/// which is the condition `memory::ops::guard` states for taking the guarded
/// door — an ingest that carries user prose and two passes that spend on a
/// model are exactly the calls the seven policy steps exist for. Resolved from
/// the caller's `Config` (as `memory::tree::health::report::run_doctor` does)
/// rather than from the ambient context, because the `tree-summarizer` CLI
/// loads its own config and has no context to be ambient in.
fn tree_guard(config: &Config) -> Result<Arc<MemoryGuard>, String> {
    crate::openhuman::memory::binding::for_config(config).map(|binding| binding.guard())
}

/// The `Tree` family on a bound guard.
fn tree_of(guard: &MemoryGuard) -> Result<&dyn MemoryTree, String> {
    guard
        .as_tree()
        .ok_or_else(|| format!("driver '{}' does not serve Tree", guard.driver_id()))
}

/// A driver error in the string shape this RPC surface has always returned.
///
/// [`MemoryError::Invalid`] is **unwrapped, not rendered**. The refusals this
/// surface used to make host-side — a namespace `validate_namespace` rejects,
/// content that is blank after trimming, a node id that is not `root` or
/// `YYYY[/MM[/DD[/HH]]]` — are the same refusals the doors answer `Invalid`
/// for, carrying the same message, because the driver calls the same
/// validators. Rendering it would prefix every one of them with "invalid
/// input: " and change strings the callers and their tests match on.
///
/// Everything else keeps the handler's own context prefix, which is what the
/// `map_err` at each call site used to supply.
fn driver_error(context: &str, error: MemoryError) -> String {
    match error {
        MemoryError::Invalid(message) => message,
        other => format!("{context}: {other}"),
    }
}

/// Append raw content to the ingestion buffer.
pub async fn tree_summarizer_ingest(
    config: &Config,
    namespace: &str,
    content: &str,
    timestamp: Option<DateTime<Utc>>,
    metadata: Option<&Value>,
) -> Result<RpcOutcome<Value>, String> {
    // Defaulted here rather than driver-side, exactly as before: the reply
    // echoes the instant the content was filed under, and a timestamp the
    // driver resolved would disagree with the one reported here by however
    // long the call took to cross. The contract makes the parameter required
    // for that reason.
    let ts = timestamp.unwrap_or_else(Utc::now);

    let guard = tree_guard(config)?;
    let path = tree_of(&guard)?
        .runtime_buffer_write(namespace, content, ts, metadata.cloned())
        .await
        .map_err(|error| driver_error("buffer write failed", error))?;

    Ok(RpcOutcome::single_log(
        json!({
            "buffered": true,
            "namespace": namespace.trim(),
            "timestamp": ts.to_rfc3339(),
            "tokens": estimate_tokens(content),
            // The driver's own `display()` of the `PathBuf` it wrote — the same
            // string this handler produced when it held the path itself.
            "path": path,
            "has_metadata": metadata.is_some(),
        }),
        format!("content buffered for namespace '{}'", namespace.trim()),
    ))
}

/// Trigger the summarization job for a namespace (drain buffer + summarize + propagate).
pub async fn tree_summarizer_run(
    config: &Config,
    namespace: &str,
) -> Result<RpcOutcome<Value>, String> {
    // #002 FR-007's consent gate stays **host-side**, and this is the one thing
    // the door does not carry across. `runtime_summarize` builds the fold's
    // provider driver-side, "the way every scheduled seal builds it" — which is
    // `chat_host::create_chat_model_with_model_id("summarization", …)`, this
    // host's ordinary role routing. That routing has no notion of
    // `memory_tree.cloud_summarization_opt_in`, so dropping this call would
    // turn an explicit privacy refusal into a silent cloud send. Resolving here
    // (and discarding the model — construction is cheap and network-free, as
    // `summarizer_available` already documents) keeps the refusal, and its
    // exact wording, unchanged.
    let _ = create_provider(config)?;

    let ts = Utc::now();
    let guard = tree_guard(config)?;

    match tree_of(&guard)?.runtime_summarize(namespace, ts).await {
        Ok(Some(node)) => Ok(RpcOutcome::single_log(
            serde_json::to_value(&node).map_err(|e| e.to_string())?,
            format!(
                "summarization completed for '{}': node {} ({} tokens)",
                namespace.trim(),
                node.node_id,
                node.token_count
            ),
        )),
        Ok(None) => Ok(RpcOutcome::single_log(
            json!({ "skipped": true, "reason": "no buffered data" }),
            format!(
                "summarization skipped for '{}': no buffered data",
                namespace.trim()
            ),
        )),
        Err(error) => Err(driver_error("summarization failed", error)),
    }
}

/// Query the tree at a specific node or level.
pub async fn tree_summarizer_query(
    config: &Config,
    namespace: &str,
    node_id: Option<&str>,
) -> Result<RpcOutcome<Value>, String> {
    let target_id = node_id.unwrap_or("root");

    let guard = tree_guard(config)?;
    let tree = tree_of(&guard)?;

    // Absence is `Ok(None)` on the door, so the "not found" message is shaped
    // here — where it always was. `drill_down` would have raised its own
    // `NotFound` instead, which is the reason this pair of reads is two members
    // rather than that one.
    let node = tree
        .runtime_read_node(namespace, target_id)
        .await
        .map_err(|error| driver_error("read node", error))?
        .ok_or_else(|| {
            format!(
                "node '{}' not found in namespace '{}'",
                target_id,
                namespace.trim()
            )
        })?;

    let children = tree
        .runtime_read_children(namespace, target_id)
        .await
        .map_err(|error| driver_error("read children", error))?;

    let result = QueryResult { node, children };
    Ok(RpcOutcome::single_log(
        serde_json::to_value(&result).map_err(|e| e.to_string())?,
        format!(
            "queried node '{}' in namespace '{}'",
            target_id,
            namespace.trim()
        ),
    ))
}

/// Get tree status/metadata for a namespace.
pub async fn tree_summarizer_status(
    config: &Config,
    namespace: &str,
) -> Result<RpcOutcome<Value>, String> {
    let guard = tree_guard(config)?;
    let status = tree_of(&guard)?
        .runtime_tree_status(namespace)
        .await
        .map_err(|error| driver_error("get status", error))?;

    Ok(RpcOutcome::single_log(
        serde_json::to_value(&status).map_err(|e| e.to_string())?,
        format!("tree status for namespace '{}'", namespace.trim()),
    ))
}

/// Rebuild the entire tree from hour leaves (background task).
pub async fn tree_summarizer_rebuild(
    config: &Config,
    namespace: &str,
) -> Result<RpcOutcome<Value>, String> {
    // The consent gate, for the reason `tree_summarizer_run` gives.
    let _ = create_provider(config)?;

    let guard = tree_guard(config)?;
    let status = tree_of(&guard)?
        .runtime_rebuild(namespace)
        .await
        .map_err(|error| driver_error("rebuild failed", error))?;

    Ok(RpcOutcome::single_log(
        serde_json::to_value(&status).map_err(|e| e.to_string())?,
        format!(
            "tree rebuilt for '{}': {} nodes",
            namespace.trim(),
            status.total_nodes
        ),
    ))
}

// ── Helper ─────────────────────────────────────────────────────────────

/// Build the (provider, model) pair the summarizer runs on (#002 FR-007).
///
/// Historically this hard-required local AI ("private + offline"), which left
/// "Build Summary Trees" dead for cloud-only setups (Tencent/OpenRouter with
/// no local Ollama). It now falls back to the **configured cloud chat
/// provider** for the summarization role when local AI is off, returning that
/// provider's model id alongside it so the engine targets the right model
/// (the engine no longer assumes the local model id). The UI shows a
/// Resolve the summarization provider.
///
/// Priority:
/// 1. Local Ollama when `local_ai.runtime_enabled = true`.
/// 2. Cloud via `create_chat_provider` when
///    `memory_tree.cloud_summarization_opt_in = true` — the user has
///    explicitly acknowledged that memory summaries will be sent to an
///    external provider.
/// 3. Error otherwise — "Build Summary Trees" is local-only by default;
///    the user must opt in to cloud summarization via the
///    `memory_tree.cloud_summarization_opt_in` setting.
///
/// # What this still decides, and what it no longer does (#5560)
///
/// It used to *be* the summariser: the model it built ran the fold. It does not
/// any more. `tree_summarizer_run` and `tree_summarizer_rebuild` go through
/// `MemoryTree::{runtime_summarize, runtime_rebuild}`, and the contract is
/// explicit that the fold runs on **the driver's** chat provider, built the way
/// every scheduled `seal` builds it — `chat_host::create_chat_model_with_model_id`,
/// i.e. this host's `"summarization"` role route.
///
/// So what survives here is the *precondition*, and it survives because the
/// driver's route cannot express it. The role ladder resolves
/// `config.memory_provider` and knows nothing about
/// `memory_tree.cloud_summarization_opt_in`; without this call an opted-out
/// user's memory summaries would go to a cloud provider on the first "run now".
/// The two handlers therefore resolve a provider and drop it, purely for the
/// refusal.
///
/// **The fold does consult this ladder — through the seam, not this call.**
/// The model resolved here is dropped; the one the fold actually runs on is
/// resolved when the driver's `"summarization"`-role chat call crosses back
/// into `modules::memory_host::resolve_chat_model`, which routes that role
/// through this same function. So an explicit run folds locally when local AI
/// is enabled, the scheduled `seal`/`cascade` passes do too, and the cloud
/// route stays behind `memory_tree.cloud_summarization_opt_in` everywhere.
/// The precondition here is still worth its construction cost: it refuses
/// before any driver work begins, with an error that names the setting.
///
/// Visibility note: `pub(crate)` is load-bearing — `modules::memory_host`'s
/// chat seam is the second caller, so the module's folds and this handler's
/// precondition resolve through one ladder that cannot drift apart.
pub(crate) fn create_provider(
    config: &Config,
) -> Result<
    (
        std::sync::Arc<dyn tinyinference::model::ChatModel<()>>,
        String,
    ),
    String,
> {
    // The summarizer applies its own temperature per request
    // (`SUMMARIZATION_TEMP` in `engine`), so the construction temperature here is
    // just a default the per-call value overrides.
    if config.local_ai.runtime_enabled {
        let model = config.local_ai.chat_model_id.clone();
        let provider_string = format!("ollama:{model}");
        tracing::debug!(
            model = %model,
            "[tree_summarizer] building crate-native local Ollama model"
        );
        return crate::openhuman::inference::provider::factory::create_local_chat_model_from_string(
            &provider_string,
            config,
        )
        .map_err(|e| format!("tree summarizer: failed to build local model: {e:#}"));
    }

    if !config.memory_tree.cloud_summarization_opt_in {
        return Err("no summarization provider — enable local AI, or opt in to \
             cloud summarization via the memory_tree.cloud_summarization_opt_in setting"
            .to_string());
    }

    // Cloud path — user has explicitly opted in. Build the configured
    // provider for the summarization role (`memory_provider` hint).
    crate::openhuman::inference::provider::create_chat_model_with_model_id(
        "summarization",
        config,
        config.default_temperature,
    )
    .map_err(|e| format!("tree summarizer: failed to build cloud provider: {e:#}"))
}

/// Whether a summarization provider can be resolved for "Build Summary Trees"
/// under the current config — the single source of truth the memory doctor
/// reuses so its `summary_tree` stage matches the runtime path (#002 FR-007).
///
/// Routes through [`create_provider`] (the SAME resolver the runtime uses):
/// - local AI enabled ⇒ available (local Ollama path).
/// - local AI off + `memory_tree.cloud_summarization_opt_in = true` ⇒
///   available iff the configured summarization-role provider resolves.
/// - local AI off + opt-in `false` (default) ⇒ unavailable — explicit
///   consent required before routing workspace memory summaries to a cloud
///   provider. Enable via the `memory_tree.cloud_summarization_opt_in` setting.
///
/// The provider built for the `Ok` check is dropped — construction is cheap
/// (no network) and confirming by build beats guessing.
pub fn summarizer_available(config: &Config) -> (bool, &'static str) {
    let local = config.local_ai.runtime_enabled;
    match create_provider(config) {
        Ok(_) if local => (
            true,
            "local AI enabled — Build Summary Trees runs on the local model",
        ),
        Ok(_) => (
            true,
            "local AI off — Build Summary Trees runs on the configured cloud provider",
        ),
        Err(_) => (
            false,
            "no summarization provider available — enable local AI, or opt in to cloud summarization (memory_tree.cloud_summarization_opt_in) with a provider set in Connections → API keys → LLM",
        ),
    }
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
