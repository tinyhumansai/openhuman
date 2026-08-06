//! `OpenHumanMemory`: the host adapter backing the tinyflows `memory` node's
//! `tinyflows::caps::MemoryProvider` capability (PR2 of the memory-node
//! feature — tracking issue #5226; see
//! `my_docs/memory_access_in_workflows/08-memory-node.md` for the design).
//!
//! **No new permission path.** Every operation routes through the exact same
//! [`enforce_node_tier_gate`] / [`gate_call_for_tier`] pair every other acting
//! adapter in [`super::caps`] uses (`OpenHumanTools`, `OpenHumanHttp`,
//! `OpenHumanCode`) — `CommandClass::Read` for `recall`/`search`/`flavour`/
//! `people`, `CommandClass::Write` for `remember`/`forget`.
//!
//! **Coherence with `flow_memory_recall`/`flow_memory_remember` (#5176).**
//! `scope: "flow"` reads and writes the *same* `flow_<id>` namespace those
//! agent tools use — both derive it from
//! [`crate::openhuman::flows::flow_namespace`], and `scope: "flows"` recall
//! delegates to the exact same [`crate::openhuman::flows::cross_flow_recall`]
//! those tools now share. A `memory[remember·flow]` node, the
//! `flow_memory_agent`, and the post-run digest subscriber therefore all read
//! and write one consistent store — never three namespace conventions that
//! happen to overlap by convention.
//!
//! **Defense-in-depth on writes.** The engine's own `validate_all` already
//! rejects `remember`/`forget` nodes authored with `scope: "user"` before a
//! run ever starts (`vendor/tinyflows/src/validate.rs`). `recall` hard-refuses
//! any `scope` it doesn't recognise; `remember` and `forget` separately
//! hard-refuse anything other than `scope: "flow"` — never `"user"`, never
//! `"flows"` — so even a compromised or out-of-band caller of this adapter
//! (bypassing the validator entirely) can never write outside a flow's own
//! sandboxed namespace.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::MemoryProvider;
use tinyflows::error::{EngineError, Result};

use crate::openhuman::agent::harness::memory_context_safety::{
    is_potentially_untrusted, wrap_untrusted_for_agent,
};
use crate::openhuman::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::config::Config;
use crate::openhuman::flows::{cross_flow_recall, flow_namespace};
use crate::openhuman::memory::tools::flavour::{lookup_flavour, FlavourLookup};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry, MemoryTaint, RecallOpts};
use crate::openhuman::security::approval::{
    redact_args, summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome,
};
use crate::openhuman::security::{CommandClass, SecurityPolicy};

use super::caps::{enforce_node_tier_gate, gate_call_for_tier};

/// Stable `tracing` grep prefix for every log line this adapter emits. Never
/// logs memory *content* or PII — only operation names, scopes, resolved
/// namespaces, tier-gate decisions, hit/miss, and result sizes.
const LOG_PREFIX: &str = "[memory-node-host]";

/// The user's durable, cross-flow memory namespace `scope: "user"` reads from
/// — the same default namespace `memory_recall`/`memory_hybrid_search`
/// resolve to when the agent doesn't name one explicitly
/// (`RecallOpts.namespace: None` falls back to this same constant inside the
/// store). Named here explicitly (rather than passing `None`) purely so it
/// shows up in the debug log.
const USER_NAMESPACE: &str = tinycortex::memory::GLOBAL_NAMESPACE;

/// Host-injected memory access for `memory` nodes. See the module doc for the
/// security contract; see [`super::caps::OpenHumanAgentRunner`] for the
/// sibling adapter this one's tier-gate wiring mirrors.
pub struct OpenHumanMemory {
    pub(crate) config: Arc<Config>,
    pub(crate) security: Arc<SecurityPolicy>,
}

impl OpenHumanMemory {
    /// Resolves the process-global memory store, matching how every other
    /// memory-backed agent tool (`flow_memory_recall`, `memory_recall`, the
    /// post-run digest subscriber) reaches it —
    /// `memory::ops::helpers::active_memory_client()` reuses the already-
    /// initialised global client when ready, else lazily initialises it for
    /// the current workspace. No adapter-local memory instance is ever
    /// constructed, so there is exactly one on-disk store in play.
    async fn memory(&self) -> Result<Arc<dyn Memory>> {
        crate::openhuman::memory::ops::helpers::active_memory_client()
            .await
            .map(|client| client.memory_handle())
            .map_err(EngineError::Capability)
    }

    /// The running flow's own id, from the run's trusted
    /// `TrustedAutomation { Workflow }` turn origin — the ONLY authoritative
    /// source. Mirrors `flows::memory_tools::trusted_flow_id`'s security
    /// invariant exactly: this adapter's `scope` argument is a fixed
    /// three-value enum (never a caller-supplied namespace string), but the
    /// *flow id* half of the namespace must still come from the trusted run
    /// context, not be re-derivable from anything model- or config-supplied,
    /// so a `scope: "flow"` node can never be pointed at another flow's
    /// namespace by a crafted `=`-binding.
    fn trusted_flow_id(&self) -> Result<String> {
        match turn_origin::current() {
            Some(AgentTurnOrigin::TrustedAutomation {
                job_id,
                source: TrustedAutomationSource::Workflow { .. },
            }) => Ok(job_id),
            _ => Err(EngineError::Capability(
                "memory node: scope \"flow\"/\"flows\" requires the node to run inside a saved \
                 flow's own run (no trusted Workflow-scoped origin found)"
                    .to_string(),
            )),
        }
    }

    /// [`flow_namespace`] for the running flow — the same `flow_<id>`
    /// namespace `flow_memory_recall`/`flow_memory_remember` read and write.
    fn flow_memory_namespace(&self) -> Result<String> {
        self.trusted_flow_id().map(|id| flow_namespace(&id))
    }

    /// Read-side tier gate: `CommandClass::Read` is `Allow` at every autonomy
    /// tier (`SecurityPolicy::gate_decision`), so this can only ever succeed
    /// or `Block` outright (`ReadOnly`/`Supervised`/`Full` all `Allow` Read).
    /// Still routed through [`enforce_node_tier_gate`] — rather than skipped
    /// — so the tier-gate debug log line and the `Block` floor stay uniform
    /// across every acting/reading adapter in this module. No
    /// [`gate_call_for_tier`] round-trip: unlike `remember`/`forget`, a read
    /// never needs the HITL escalation that function exists for, and — like
    /// the curated-Read short-circuit in `OpenHumanTools::invoke` — routing
    /// a guaranteed-`Allow` decision through `intercept_audited` anyway would
    /// only reintroduce the "reads wait for approval" bug that short-circuit
    /// was added to close.
    fn tier_gate_read(&self, op: &str) -> Result<()> {
        enforce_node_tier_gate(&self.security, CommandClass::Read, op)?;
        Ok(())
    }

    /// Write-side tier gate: `CommandClass::Write` is `Block` under
    /// `ReadOnly` (returns `Err` from [`enforce_node_tier_gate`] before this
    /// function is even entered — see call sites), `Prompt` under
    /// `Supervised`, and `Allow` under `Full`. Unlike [`Self::tier_gate_read`]
    /// this ALWAYS calls [`gate_call_for_tier`] — never short-circuits on
    /// `Allow` — because a `Full`-tier run can still have the *flow's own*
    /// `require_approval: true` toggle set, and only `intercept_audited`
    /// (reached via `gate_call_for_tier`) consults that. Mirrors
    /// `OpenHumanCode::run`'s gating exactly.
    ///
    /// Returns the approval audit request id (`None` when no
    /// [`ApprovalGate`] is installed, or the tier decision never went
    /// through a `Prompt` round-trip). Callers MUST thread it into
    /// [`Self::record_write_execution`] once the actual write resolves —
    /// see that function's doc for why (E-m1).
    async fn tier_gate_write(&self, op: &str, action: &Value) -> Result<Option<String>> {
        let tool_name = format!("flows_memory_{op}");
        let tier_decision = enforce_node_tier_gate(&self.security, CommandClass::Write, op)?;
        let summary = summarize_action(&tool_name, action);
        let redacted = redact_args(action);
        let (outcome, audit_id) =
            gate_call_for_tier(tier_decision, &tool_name, &summary, redacted).await;
        if let GateOutcome::Deny { reason } = outcome {
            tracing::warn!(target: "flows", op, "{LOG_PREFIX} write: approval gate denied");
            return Err(EngineError::Capability(reason));
        }
        Ok(audit_id)
    }

    /// Closes out the approval audit row opened by [`Self::tier_gate_write`]
    /// (E-m1): unlike the http/code/Composio node paths, this used to
    /// discard the request id (`_audit_id`) and never call
    /// `record_execution`, so an approved `remember`/`forget` left its audit
    /// row stuck open with no terminal Success/Failure outcome. A no-op when
    /// `audit_id` is `None` (no gate installed, or the decision never
    /// prompted). Takes `success`/`error` rather than a `Result` so callers
    /// don't need `EngineError: Clone` to both record the outcome and
    /// propagate the original error.
    fn record_write_execution(
        op: &str,
        audit_id: Option<&str>,
        success: bool,
        error: Option<&str>,
    ) {
        let Some(id) = audit_id else { return };
        let Some(gate) = ApprovalGate::try_global() else {
            return;
        };
        let exec = if success {
            ExecutionOutcome::Success
        } else {
            ExecutionOutcome::Failure
        };
        tracing::debug!(
            target: "flows",
            op,
            audit_id = %id,
            success,
            "{LOG_PREFIX} write: recording execution outcome on the approval audit trail"
        );
        gate.record_execution(id, exec, error);
    }

    /// Shapes a batch of [`MemoryEntry`] hits into the node's `recall`/
    /// `search` output: `{ scope, query, results: [{ id, key, text, score,
    /// namespace, category }] }`. Each entry's `text` passes through
    /// [`wrap_untrusted_for_agent`] when [`is_potentially_untrusted`] flags
    /// it — a connector-synced row (email, Slack, …) reaching downstream
    /// `agent`/`condition` bindings must carry the same untrusted-source
    /// marking a normal agent-prompt recall would give it.
    fn shape_recall_result(scope: &str, query: &str, entries: &[MemoryEntry]) -> Value {
        let results: Vec<Value> = entries
            .iter()
            .map(|entry| {
                let text = if is_potentially_untrusted(entry) {
                    let hint = entry.namespace.as_deref().unwrap_or(scope);
                    wrap_untrusted_for_agent(&entry.content, hint)
                } else {
                    entry.content.clone()
                };
                json!({
                    "id": entry.id,
                    "key": entry.key,
                    "text": text,
                    "score": entry.score,
                    "namespace": entry.namespace,
                    "category": entry.category.to_string(),
                })
            })
            .collect();
        json!({ "scope": scope, "query": query, "results": results })
    }
}

#[async_trait]
impl MemoryProvider for OpenHumanMemory {
    /// Backs both `recall` and `search` (`opts.operation` distinguishes them
    /// only for the `tracing::debug!` logs below — [`Self::shape_recall_result`]
    /// returns `{ scope, query, results }` with no `operation` field, so the
    /// two ops are otherwise indistinguishable in the response; both
    /// currently route through the same [`Memory::recall`] call, as there is
    /// no separate hybrid-search path reachable through the generic
    /// `Arc<dyn Memory>` trait object this adapter holds).
    async fn recall(&self, scope: &str, query: &str, opts: Value) -> Result<Value> {
        let operation: &str = opts
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or("recall");
        #[allow(clippy::cast_possible_truncation)]
        let limit = opts
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(5, |v| v as usize);
        let min_score = opts.get("min_score").and_then(Value::as_f64);

        tracing::debug!(
            target: "flows",
            operation,
            scope,
            query_chars = query.chars().count(),
            limit,
            ?min_score,
            "{LOG_PREFIX} recall: entry"
        );

        self.tier_gate_read(operation)?;

        let entries = match scope {
            "user" => {
                let memory = self.memory().await?;
                let recall_opts = RecallOpts {
                    namespace: Some(USER_NAMESPACE),
                    min_score,
                    ..RecallOpts::default()
                };
                tracing::debug!(
                    target: "flows",
                    operation,
                    namespace = USER_NAMESPACE,
                    "{LOG_PREFIX} recall: querying user-scope namespace"
                );
                memory
                    .recall(query, limit, recall_opts)
                    .await
                    .map_err(|e| {
                        EngineError::Capability(format!("memory node: recall failed: {e}"))
                    })?
            }
            "flow" => {
                let namespace = self.flow_memory_namespace()?;
                let memory = self.memory().await?;
                let recall_opts = RecallOpts {
                    namespace: Some(namespace.as_str()),
                    min_score,
                    ..RecallOpts::default()
                };
                tracing::debug!(
                    target: "flows",
                    operation,
                    namespace = %namespace,
                    "{LOG_PREFIX} recall: querying this flow's own namespace"
                );
                memory
                    .recall(query, limit, recall_opts)
                    .await
                    .map_err(|e| {
                        EngineError::Capability(format!("memory node: recall failed: {e}"))
                    })?
            }
            "flows" => {
                // Read-only, and — via `cross_flow_recall` — confined to the
                // shared `flow_*` namespace prefix. This is intentionally
                // NOT a broad cross-namespace scan: it can never reach the
                // user's personal/global memory, only other flows' own
                // automation output, exactly like `flow_memory_recall`'s
                // `scope: "flows"` arm it delegates to.
                let memory = self.memory().await?;
                tracing::debug!(
                    target: "flows",
                    operation,
                    "{LOG_PREFIX} recall: querying cross-flow (flow_* prefix only)"
                );
                cross_flow_recall(&memory, query, limit, min_score)
                    .await
                    .map_err(|e| {
                        EngineError::Capability(format!(
                            "memory node: cross-flow recall failed: {e}"
                        ))
                    })?
            }
            other => {
                return Err(EngineError::Capability(format!(
                    "memory node: unknown scope \"{other}\" (expected \"user\", \"flow\", or \"flows\")"
                )));
            }
        };

        tracing::debug!(
            target: "flows",
            operation,
            scope,
            hit_count = entries.len(),
            "{LOG_PREFIX} recall: store returned"
        );

        Ok(Self::shape_recall_result(scope, query, &entries))
    }

    /// Delegates to [`lookup_flavour`] — the exact same flavoured-tree read
    /// path `memory_flavour` (`MemoryFlavourTool`, #5175) uses.
    async fn flavour(&self, slug: &str) -> Result<Value> {
        tracing::debug!(target: "flows", flavour = slug, "{LOG_PREFIX} flavour: entry");
        self.tier_gate_read("flavour")?;

        match lookup_flavour(&self.config, slug) {
            Err(hard) => {
                tracing::debug!(target: "flows", flavour = slug, "{LOG_PREFIX} flavour: rejected (bad slug)");
                Err(EngineError::Capability(format!("memory node: {hard}")))
            }
            Ok(FlavourLookup::Profile(body)) => {
                tracing::debug!(
                    target: "flows",
                    flavour = slug,
                    profile_chars = body.chars().count(),
                    "{LOG_PREFIX} flavour: hit"
                );
                Ok(json!({ "flavour": slug, "found": true, "profile": body }))
            }
            Ok(FlavourLookup::NotBuilt(message)) => {
                tracing::debug!(target: "flows", flavour = slug, "{LOG_PREFIX} flavour: no profile built yet");
                Ok(
                    json!({ "flavour": slug, "found": false, "profile": Value::Null, "message": message }),
                )
            }
            Ok(FlavourLookup::Failed(message)) => {
                tracing::warn!(target: "flows", flavour = slug, "{LOG_PREFIX} flavour: lookup failed");
                Err(EngineError::Capability(format!("memory node: {message}")))
            }
        }
    }

    /// Delegates to the same op the `people_list` agent tool calls
    /// (`people::rpc::handle_list` over the current-context `PeopleStore`),
    /// then narrows the ranked listing to entries whose display name,
    /// primary email/phone, or any handle value contains `query`
    /// (case-insensitive substring — this adapter has no access to a richer
    /// query DSL than that).
    async fn people(&self, query: Option<&str>) -> Result<Value> {
        let query = query.map(str::trim).filter(|q| !q.is_empty());
        tracing::debug!(target: "flows", has_query = query.is_some(), "{LOG_PREFIX} people: entry");
        self.tier_gate_read("people")?;

        let store = crate::core::runtime::context::CoreContext::current()
            .ok_or_else(|| {
                EngineError::Capability(
                    "memory node: people store unavailable: core context not initialized"
                        .to_string(),
                )
            })?
            .people()
            .map_err(|e| {
                EngineError::Capability(format!("memory node: people store unavailable: {e}"))
            })?;

        const DEFAULT_PEOPLE_LIMIT: usize = 100;
        let outcome =
            crate::openhuman::memory::people::rpc::handle_list(&store, DEFAULT_PEOPLE_LIMIT)
                .await
                .map_err(EngineError::Capability)?;

        let shaped = match query {
            None => outcome.value,
            Some(q) => filter_people_by_query(outcome.value, q),
        };

        tracing::debug!(
            target: "flows",
            result_count = shaped.get("people").and_then(serde_json::Value::as_array).map_or(0, Vec::len),
            "{LOG_PREFIX} people: done"
        );
        Ok(shaped)
    }

    /// Writes `value` under `key` into this flow's own `flow_<id>`
    /// namespace — see the module doc's defense-in-depth note. Never
    /// reachable for `scope != "flow"`, regardless of what the caller
    /// passes: this is checked here independently of the engine's own
    /// `validate_all` rejection of `scope: "user"` writes.
    async fn remember(&self, scope: &str, key: &str, value: Value) -> Result<()> {
        if scope != "flow" {
            tracing::warn!(
                target: "flows",
                scope,
                "{LOG_PREFIX} remember: REFUSED — scope must be \"flow\" (defense-in-depth; a \
                 remember/forget node targeting scope \"user\" is already a hard reject in \
                 tinyflows' own validate_all, but this adapter never trusts that alone)"
            );
            return Err(EngineError::Capability(format!(
                "memory node: remember only supports scope \"flow\" (got \"{scope}\") — writing \
                 to the user's personal memory, or to the read-only \"flows\" scope, is never \
                 permitted from a memory node"
            )));
        }
        let key = key.trim();
        if key.is_empty() {
            return Err(EngineError::Capability(
                "memory node: remember requires a non-empty key".to_string(),
            ));
        }

        // Secret check MUST run before the tier gate / HITL approval prompt
        // below: `tier_gate_write` can park the run for human approval
        // (`gate_call_for_tier`), and a likely-secret value must be refused
        // up front rather than spend that approval round-trip on a write
        // that was always going to be rejected (review fix — see #5227).
        let content = value_to_content(&value);
        if crate::openhuman::memory::store::safety::has_likely_secret(&content) {
            tracing::warn!(
                target: "flows",
                key_chars = key.chars().count(),
                content_chars = content.chars().count(),
                "{LOG_PREFIX} remember: REFUSED — content looks like a secret"
            );
            return Err(EngineError::Capability(
                "memory node: refusing to store content that looks like a secret".to_string(),
            ));
        }

        let action = json!({ "operation": "remember", "scope": scope, "key": key });
        let audit_id = self.tier_gate_write("remember", &action).await?;

        let namespace = self.flow_memory_namespace()?;
        let memory = self.memory().await?;
        let store_result = memory
            .store_with_taint(
                &namespace,
                key,
                &content,
                MemoryCategory::Core,
                None,
                MemoryTaint::ExternalSync,
            )
            .await
            .map_err(|e| EngineError::Capability(format!("memory node: remember failed: {e}")));
        Self::record_write_execution(
            "remember",
            audit_id.as_deref(),
            store_result.is_ok(),
            store_result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .as_deref(),
        );
        store_result?;

        tracing::debug!(
            target: "flows",
            namespace = %namespace,
            key_chars = key.chars().count(),
            content_chars = content.chars().count(),
            "{LOG_PREFIX} remember: stored"
        );
        Ok(())
    }

    /// Deletes `key` from this flow's own `flow_<id>` namespace — same
    /// `scope`-lockdown as [`Self::remember`].
    async fn forget(&self, scope: &str, key: &str) -> Result<()> {
        if scope != "flow" {
            tracing::warn!(
                target: "flows",
                scope,
                "{LOG_PREFIX} forget: REFUSED — scope must be \"flow\" (defense-in-depth)"
            );
            return Err(EngineError::Capability(format!(
                "memory node: forget only supports scope \"flow\" (got \"{scope}\") — the \
                 user's personal memory and the read-only \"flows\" scope are never reachable \
                 from a memory node"
            )));
        }
        let key = key.trim();
        if key.is_empty() {
            return Err(EngineError::Capability(
                "memory node: forget requires a non-empty key".to_string(),
            ));
        }

        let action = json!({ "operation": "forget", "scope": scope, "key": key });
        let audit_id = self.tier_gate_write("forget", &action).await?;

        let namespace = self.flow_memory_namespace()?;
        let memory = self.memory().await?;
        let forget_result = memory
            .forget(&namespace, key)
            .await
            .map_err(|e| EngineError::Capability(format!("memory node: forget failed: {e}")));
        Self::record_write_execution(
            "forget",
            audit_id.as_deref(),
            forget_result.is_ok(),
            forget_result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .as_deref(),
        );
        let removed = forget_result?;

        tracing::debug!(
            target: "flows",
            namespace = %namespace,
            key_chars = key.chars().count(),
            removed,
            "{LOG_PREFIX} forget: done"
        );
        Ok(())
    }
}

/// Renders a `remember` node's `value` (arbitrary JSON — a plain string when
/// the author wrote a literal, or any resolved `=`-expression result
/// otherwise) into the `&str` content [`Memory::store_with_taint`] persists.
/// A JSON string is stored verbatim (not re-quoted); anything else is
/// serialized to its compact JSON form so structured values round-trip
/// losslessly through recall.
fn value_to_content(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Narrows a `{"people": [...]}` listing (as returned by
/// `people::rpc::handle_list`) to entries whose display name, primary email,
/// primary phone, or any handle value contains `query` — case-insensitive
/// substring match. Malformed/missing fields on an entry just don't match
/// (never panics on an unexpected shape).
fn filter_people_by_query(listing: Value, query: &str) -> Value {
    let needle = query.to_ascii_lowercase();
    let people = listing
        .get("people")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let matches = |field: Option<&str>| {
        field
            .map(|s| s.to_ascii_lowercase().contains(&needle))
            .unwrap_or(false)
    };
    let filtered: Vec<Value> = people
        .into_iter()
        .filter(|person| {
            matches(person.get("display_name").and_then(Value::as_str))
                || matches(person.get("primary_email").and_then(Value::as_str))
                || matches(person.get("primary_phone").and_then(Value::as_str))
                || person
                    .get("handles")
                    .and_then(Value::as_array)
                    .is_some_and(|handles| {
                        handles
                            .iter()
                            .any(|h| matches(h.get("value").and_then(Value::as_str)))
                    })
        })
        .collect();
    json!({ "people": filtered })
}

#[cfg(test)]
#[path = "memory_adapter_tests.rs"]
mod tests;
