//! `OpenHumanMemory`: the host adapter backing the tinyflows `memory` node's
//! `tinyflows::caps::MemoryProvider` capability, over memory v2.
//!
//! **No new permission path.** Every operation routes through the same
//! [`enforce_node_tier_gate`] / [`gate_call_for_tier`] pair every other acting
//! adapter in [`super::caps`] uses — `CommandClass::Read` for `recall`/
//! `search`/`flavour`/`people`, `CommandClass::Write` for `remember`/`forget`.
//!
//! **Scopes map onto tag filters.** `scope: "user"` reads the whole store
//! (no filter); `scope: "flow"` reads the running flow's own items
//! ([`crate::flows::flow_filter`], tag `flow:<id>`); `scope: "flows"` reads
//! every flow's items ([`crate::flows::cross_flow_filter`], tag `flows`).
//! `recall` asks the engine for a synthesised answer with citations
//! (`memory::ops::recall`); `search` returns raw ranked hits
//! (`memory::ops::fetch`). These are the same tags `flow_memory_recall`/
//! `flow_memory_remember` and the post-run digest use, so all three read and
//! write one consistent slice of memory.
//!
//! **Keys.** `remember(key, value)` stores a learning tagged
//! [`crate::flows::flow_meta`] plus the per-key tag, replacing an earlier
//! value under the same key ([`crate::flows::remember_keyed`]); `forget(key)`
//! forgets by that per-key tag filter. Forgetting an absent key forgets
//! nothing and is not an error.
//!
//! **`flavour` and `people`.** Memory v2 has neither flavoured profiles nor a
//! people store: `flavour` reports every slug as unknown (the trait's error
//! case) and `people` returns an empty listing marked `supported: false`.
//!
//! **Defense-in-depth on writes.** The engine's own `validate_all` already
//! rejects `remember`/`forget` nodes authored with `scope: "user"` before a
//! run starts; this adapter separately hard-refuses anything other than
//! `scope: "flow"`, and takes the flow id only from the run's trusted origin.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::MemoryProvider;
use tinyflows::error::{EngineError, Result};

use crate::agent::harness::memory_context_safety::wrap_untrusted_for_agent;
use crate::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use crate::config::Config;
use crate::flows::{
    cross_flow_filter, flow_filter, flow_key_of, flow_key_tag, forget_matching, remember_keyed,
    FLOWS_TAG,
};
use crate::memory::types::{FetchParams, RecallParams};
use crate::memory::MemoryError;
use crate::security::approval::{
    redact_args, summarize_action, ApprovalGate, ExecutionOutcome, GateOutcome,
};
use crate::security::{CommandClass, SecurityPolicy};
use tinymemory::{Hit, LearningKind, MemoryMeta, MetaFilter, SourceKind};

use super::caps::{enforce_node_tier_gate, gate_call_for_tier};

/// Stable `tracing` grep prefix for every log line this adapter emits. Never
/// logs memory *content* or PII — only operation names, scopes, resolved
/// namespaces, tier-gate decisions, hit/miss, and result sizes.
const LOG_PREFIX: &str = "[memory-node-host]";

/// Host-injected memory access for `memory` nodes. See the module doc for the
/// security contract; see [`super::caps::OpenHumanAgentRunner`] for the
/// sibling adapter this one's tier-gate wiring mirrors.
pub struct OpenHumanMemory {
    pub(crate) config: Arc<Config>,
    pub(crate) security: Arc<SecurityPolicy>,
}

impl OpenHumanMemory {
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

    /// The tag filter a read in `scope` applies; `None` reads everything.
    fn scope_filter(&self, scope: &str) -> Result<Option<MetaFilter>> {
        match scope {
            "user" => Ok(None),
            "flow" => self.trusted_flow_id().map(|id| Some(flow_filter(&id))),
            "flows" => Ok(Some(cross_flow_filter())),
            other => Err(EngineError::Capability(format!(
                "memory node: unknown scope \"{other}\" (expected \"user\", \"flow\", or \"flows\")"
            ))),
        }
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

    /// Shapes one result row. Text that did not come from a plain agent
    /// learning (a synced source, a conversation, or any flow's own
    /// automation output) passes through [`wrap_untrusted_for_agent`], so it
    /// reaches downstream `agent`/`condition` bindings marked as data.
    fn shape_row(id: &str, text: &str, meta: &MemoryMeta, score: Option<f32>) -> Value {
        let text = if is_untrusted(meta) {
            wrap_untrusted_for_agent(text, meta.source.kind.as_str())
        } else {
            text.to_string()
        };
        json!({
            "id": id,
            "key": flow_key_of(meta),
            "text": text,
            "score": score,
            "source": meta.source.kind.as_str(),
        })
    }
}

/// Whether an item's text must be marked untrusted before it reaches an
/// agent: anything but a plain (non-flow) agent learning.
fn is_untrusted(meta: &MemoryMeta) -> bool {
    meta.source.kind != SourceKind::Agent || meta.tags.iter().any(|tag| tag == FLOWS_TAG)
}

/// Maps a memory failure onto the node's capability error.
fn capability_error(op: &str, error: &MemoryError) -> EngineError {
    EngineError::Capability(format!("memory node: {op} failed: {error}"))
}

#[async_trait]
impl MemoryProvider for OpenHumanMemory {
    /// Backs both `recall` and `search`. `recall` returns the engine's
    /// synthesised `answer` plus its citations as `results`; `search` returns
    /// raw ranked hits as `results`. Each row is `{ id, key, text, score,
    /// source }`; rows under the node's `min_score` are dropped.
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
        let filter = self.scope_filter(scope)?;

        let (answer, results): (Option<String>, Vec<Value>) = if operation == "search" {
            let page = crate::memory::ops::fetch(
                &self.config,
                FetchParams {
                    query: query.to_string(),
                    mode: None,
                    filter,
                    limit: Some(limit),
                    cursor: None,
                },
            )
            .await
            .map_err(|e| capability_error(operation, &e))?;
            let rows = page
                .hits
                .iter()
                .filter(|hit| passes_min_score(Some(hit.score), min_score))
                .map(|hit: &Hit| Self::shape_row(&hit.id.0, &hit.text, &hit.meta, Some(hit.score)))
                .collect();
            (None, rows)
        } else {
            let view = crate::memory::ops::recall(
                &self.config,
                RecallParams {
                    question: query.to_string(),
                    filter,
                    limit: Some(limit),
                },
            )
            .await
            .map_err(|e| capability_error(operation, &e))?;
            let rows = view
                .citations
                .iter()
                .filter(|c| passes_min_score(c.score, min_score))
                .map(|c| Self::shape_row(&c.id.0, &c.snippet, &c.meta, c.score))
                .collect();
            (Some(view.answer), rows)
        };

        tracing::debug!(
            target: "flows",
            operation,
            scope,
            hit_count = results.len(),
            "{LOG_PREFIX} recall: engine returned"
        );

        let mut out = json!({ "scope": scope, "query": query, "results": results });
        if let Some(answer) = answer {
            out["answer"] = Value::String(answer);
        }
        Ok(out)
    }

    /// Memory v2 keeps no flavoured profiles, so every slug is unknown.
    async fn flavour(&self, slug: &str) -> Result<Value> {
        tracing::debug!(target: "flows", flavour = slug, "{LOG_PREFIX} flavour: unsupported");
        self.tier_gate_read("flavour")?;
        Err(EngineError::Capability(format!(
            "memory node: unknown flavour \"{slug}\" (this host's memory keeps no flavour profiles)"
        )))
    }

    /// Memory v2 keeps no people store: an empty listing marked unsupported.
    async fn people(&self, query: Option<&str>) -> Result<Value> {
        tracing::debug!(
            target: "flows",
            has_query = query.is_some_and(|q| !q.trim().is_empty()),
            "{LOG_PREFIX} people: unsupported, empty listing"
        );
        self.tier_gate_read("people")?;
        Ok(json!({ "people": [], "supported": false }))
    }

    /// Writes `value` under `key` into this flow's own memory — see the module doc's defense-in-depth note. Never
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
        if crate::security::scrub::has_likely_secret(&content) {
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

        let flow_id = self.trusted_flow_id()?;
        let store_result =
            remember_keyed(&self.config, &flow_id, key, &content, LearningKind::Fact)
                .await
                .map_err(|e| capability_error("remember", &e));
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
            flow_id = %flow_id,
            key_chars = key.chars().count(),
            content_chars = content.chars().count(),
            "{LOG_PREFIX} remember: stored"
        );
        Ok(())
    }

    /// Forgets this flow's value for `key` (by its per-key tag) — same
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

        let flow_id = self.trusted_flow_id()?;
        let filter = MetaFilter {
            tags_any: vec![flow_key_tag(&flow_id, key)],
            ..MetaFilter::default()
        };
        let forget_result = forget_matching(&self.config, filter)
            .await
            .map_err(|e| capability_error("forget", &e));
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
            flow_id = %flow_id,
            key_chars = key.chars().count(),
            removed,
            "{LOG_PREFIX} forget: done"
        );
        Ok(())
    }
}

/// Renders a `remember` node's `value` (arbitrary JSON — a plain string when
/// the author wrote a literal, or any resolved `=`-expression result
/// otherwise) into the learning text it is stored as.
/// A JSON string is stored verbatim (not re-quoted); anything else is
/// serialized to its compact JSON form so structured values round-trip
/// losslessly through recall.
fn value_to_content(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Whether a row's score clears the node's optional `min_score`. A row the
/// engine did not score passes.
fn passes_min_score(score: Option<f32>, min_score: Option<f64>) -> bool {
    match (score, min_score) {
        (Some(score), Some(min)) => f64::from(score) >= min,
        _ => true,
    }
}

#[cfg(test)]
#[path = "memory_adapter_tests.rs"]
mod tests;
