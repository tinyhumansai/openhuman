//! Contract gate for late-bound Composio actions (#4853).
//!
//! Per-action Composio tools handed to `integrations_agent` are built from
//! the lightweight `list_tools` response — a one-line description with a
//! parameter schema that is often thin or absent (see
//! `fetch_toolkit_actions`, consumed in the
//! sub-agent runner). The model therefore composes calls before the action's
//! FULL contract is in context and guesses argument formats — most visibly, it
//! sends Gmail `query` strings without the quoting Gmail search syntax requires,
//! so `GMAIL_FETCH_EMAILS` returns zero results.
//!
//! The gate makes the full contract enter context BEFORE execution: on the
//! first call to an action this turn, if a fuller live contract is available
//! (via the cached `fetch_live_toolkit_catalog`), it is returned as a
//! recoverable tool error instead of executing. The retry — now with the
//! schema/description in context — proceeds normally. This mirrors the
//! discover-then-call discipline the generic `composio_execute` dispatcher
//! already expects (`composio_list_tools` → `composio_execute`), but enforces
//! it on the per-action surface where the model never sees the full schema.
//!
//! Scope: this pass gates the per-action Composio surface
//! ([`super::action_tool::ComposioActionTool`]). Generalising the same gate to
//! the `composio_execute` dispatcher, the MCP bridges, and the Workflow
//! dispatchers — plus resetting the per-turn state on context compaction — is
//! tracked as follow-up (a shared `ToolMiddleware` at the turn-harness seam is
//! the natural home; see the PR description).

use std::collections::HashSet;
use std::sync::Mutex;

use crate::openhuman::config::Config;
// The live-contract lookup is sourced from the flows/tinyflows caps catalog,
// which is compiled out when the `flows` feature is off (#4912). The gate then
// simply has no fuller contract to surface and always proceeds, so the import
// and the lookup/format helpers below are gated in lockstep.
#[cfg(feature = "flows")]
use crate::openhuman::composio::providers::toolkit_from_slug;
#[cfg(feature = "flows")]
use crate::openhuman::tinyflows::caps::{fetch_live_toolkit_catalog, ToolContract};

/// Record of which action contracts have already been surfaced to the model,
/// so the gate blocks a given action at most once per gate instance.
///
/// One [`ContractGate`] is held per [`super::action_tool::ComposioActionTool`]
/// instance; those tools are constructed fresh per `integrations_agent` spawn
/// and live for that spawn's tool loop. That loop is a single agent turn in the
/// common case, so "seen" behaves as per-turn state without any task-local
/// plumbing — but a long-lived spawn can span multiple turns, and this gate
/// does NOT reset when the surfaced schema drops out of context via compaction
/// (tracked as follow-up; see the module-level note). Interior-mutable so the
/// gate can record state through the tool's `&self` `execute`.
#[derive(Default)]
pub struct ContractGate {
    seen: Mutex<HashSet<String>>,
}

impl ContractGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `slug` (normalised to upper-case) into the seen-set. Returns
    /// `true` when it was NOT already present — i.e. this is the first time the
    /// gate has been consulted for this action for this gate's lifetime.
    ///
    /// The lock is taken and released entirely within this call, so no guard is
    /// held across the caller's later `await`.
    fn mark_seen(&self, slug: &str) -> bool {
        let mut guard = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.insert(slug.to_ascii_uppercase())
    }
}

/// Outcome of consulting the gate for one action call.
pub enum GateDecision {
    /// Return this text to the model as a recoverable tool error; the model
    /// retries with the contract in context.
    Surface(String),
    /// Execute the action normally.
    Proceed,
}

/// Consult the gate before executing `action_slug`.
///
/// On the FIRST consult for a slug this turn, if a fuller live contract can be
/// resolved, returns [`GateDecision::Surface`] with the formatted contract and
/// marks the slug seen. Every later consult — and any consult where no live
/// contract is available (unconfigured client, unknown action, network miss) —
/// returns [`GateDecision::Proceed`], so the gate never blocks an action more
/// than once and never blocks when it cannot help.
pub async fn consult(gate: &ContractGate, config: &Config, action_slug: &str) -> GateDecision {
    // Mark first (releasing the lock) so the retry — and any concurrent
    // sibling call — proceeds even if the contract lookup below is slow.
    let first_time = gate.mark_seen(action_slug);
    if !first_time {
        tracing::debug!(
            target: "composio",
            slug = %action_slug,
            "[composio][contract-gate] contract already surfaced this turn; proceeding"
        );
        return GateDecision::Proceed;
    }

    // The live catalog lives in the flows/tinyflows caps layer. With `flows`
    // compiled out there is no catalog source, so the gate can never surface a
    // fuller contract and always proceeds (the per-action tool still runs; it
    // just does not get the pre-execute contract nudge).
    #[cfg(feature = "flows")]
    if let Some(contract) = lookup_contract(config, action_slug).await {
        tracing::debug!(
            target: "composio",
            slug = %action_slug,
            has_input_schema = contract.input_schema.is_some(),
            required_arg_count = contract.required_args.len(),
            "[composio][contract-gate] surfacing full contract before first execute"
        );
        return GateDecision::Surface(format_contract(action_slug, &contract));
    }

    // `config` is only consulted through the flows-gated lookup above.
    #[cfg(not(feature = "flows"))]
    let _ = config;

    tracing::debug!(
        target: "composio",
        slug = %action_slug,
        "[composio][contract-gate] no live contract available; proceeding without gating"
    );
    GateDecision::Proceed
}

/// Resolve the full live contract for `action_slug` from the process-cached
/// live toolkit catalog. Returns `None` when the toolkit can't be derived, the
/// catalog can't be fetched (unconfigured / offline — `fetch_live_toolkit_catalog`
/// degrades to `None`), or the action isn't in it.
#[cfg(feature = "flows")]
async fn lookup_contract(config: &Config, action_slug: &str) -> Option<ToolContract> {
    let toolkit = toolkit_from_slug(action_slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    contracts
        .into_iter()
        .find(|c| c.slug.eq_ignore_ascii_case(action_slug))
}

/// Render the contract into a compact instruction for the model. Contains only
/// the provider's own action description + JSON schema — no user data / PII.
#[cfg(feature = "flows")]
fn format_contract(action_slug: &str, contract: &ToolContract) -> String {
    let mut out = format!(
        "Before running `{action_slug}`, read its full contract below and then re-issue \
         the call with arguments that match it exactly.\n\n"
    );

    if let Some(desc) = contract
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        out.push_str("Description:\n");
        out.push_str(desc);
        out.push_str("\n\n");
    }

    match contract.input_schema.as_ref() {
        Some(schema) => {
            let pretty =
                serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
            out.push_str("Input JSON schema:\n");
            out.push_str(&pretty);
            out.push('\n');
        }
        None => out.push_str("Input JSON schema: not published by the provider for this action.\n"),
    }

    if !contract.required_args.is_empty() {
        out.push_str(&format!(
            "\nRequired arguments: {}\n",
            contract.required_args.join(", ")
        ));
    }

    out.push_str(
        "\nCompose every argument to match this schema and any format rules in the \
         description. Text-search fields in particular often require the provider's exact \
         query syntax (for example, Gmail needs multi-word phrases quoted, like \
         subject:\"quarterly report\"). Then call the action again with the corrected \
         arguments.",
    );
    out
}

// The gate's unit tests seed the flows/tinyflows live-catalog cache, so they
// only compile and run with the `flows` feature on.
#[cfg(all(test, feature = "flows"))]
#[path = "contract_gate_tests.rs"]
mod tests;
