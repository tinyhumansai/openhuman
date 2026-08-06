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

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::OnceLock;

use crate::openhuman::config::Config;
use crate::openhuman::integrations::composio::catalog::{fetch_live_toolkit_catalog, ToolContract};
use crate::openhuman::integrations::composio::providers::toolkit_from_slug;

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
///
/// ## Auto-proceed safety net (#5119)
///
/// When the main agent re-delegates to a fresh `integrations_agent` sub-agent,
/// each spawn creates a new tool with a fresh `ContractGate`. Without a
/// process-wide cross-instance consult counter, every fresh gate would surface
/// the same contract and the action would never execute — causing an infinite
/// loop ("same tool call 3× in a row" guard).
///
/// A global [`OnceLock`] map tracks how many *unique gate instances* have
/// consulted each slug for the "first time". After 3+ fresh instances have all
/// surfaced the same contract, the next instance auto-proceeds: the model has
/// clearly been given the schema and needs execution, not another schema dump.
///
/// The threshold is generous (3+ instances = at least 3 surfaced contracts in
/// different sub-agent iterations) so that the normal surface-once-then-execute
/// path within a single spawn is never disrupted.
#[derive(Default)]
pub struct ContractGate {
    seen: Mutex<HashSet<String>>,
}

/// Process-wide consult counter: tracks how many unique [`ContractGate`]
/// instances have consulted each slug for the first time. Used by the
/// auto-proceed safety net (#5119) to detect the re-delegation pattern where
/// fresh tools keep surfacing the same contract without ever executing.
static GLOBAL_FIRST_CONSULT_COUNT: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();

/// After this many unique fresh gate instances have all surfaced the same
/// contract as "first time", the next instance auto-proceeds. Set conservatively
/// high (3+ instances) so the normal surface-once-then-execute pattern within a
/// single spawn is never affected: the threshold fires only when the model has
/// been shown the schema in at least 3 separate sub-agent iterations without
/// any of them advancing to execution.
const AUTO_PROCEED_THRESHOLD: u32 = 3;

impl ContractGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consult the gate for `slug`. Returns the consult outcome with the
    /// auto-proceed safety net applied.
    ///
    /// On the first consult of `slug` by THIS gate instance:
    /// 1. The slug is recorded in the instance-local seen-set.
    /// 2. The global first-time consult counter for this slug is incremented.
    /// 3. If the global counter exceeds [`AUTO_PROCEED_THRESHOLD`], the gate
    ///    returns [`GateConsultOutcome::AutoProceed`] — too many fresh instances
    ///    have seen this contract without executing it.
    /// 4. Otherwise, returns [`GateConsultOutcome::FirstTime`] so the caller
    ///    can surface the contract.
    ///
    /// On subsequent consults of `slug` by THIS gate instance (the slug is
    /// already in the instance-local seen-set): returns
    /// [`GateConsultOutcome::Proceed`] — the contract was already surfaced.
    ///
    /// The lock is taken and released entirely within this call, so no guard is
    /// held across the caller's later `await`.
    fn gate_consult(&self, slug: &str) -> GateConsultOutcome {
        let norm = slug.to_ascii_uppercase();

        // 1. Check instance-local set first.
        let is_first = {
            let mut guard = self
                .seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.insert(norm.clone())
        };

        if !is_first {
            // Already seen by this instance → proceed.
            return GateConsultOutcome::Proceed;
        }

        // 2. Increment the global first-time consult counter.
        let global_count = {
            let mut map = GLOBAL_FIRST_CONSULT_COUNT
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let entry = map.entry(norm).or_insert(0);
            *entry += 1;
            *entry
        };

        // 3. Auto-proceed safety net.
        if global_count > AUTO_PROCEED_THRESHOLD {
            tracing::warn!(
                target: "composio",
                slug = %slug,
                global_count,
                "[composio][contract-gate] auto-proceeding after {global_count} fresh instances surfaced this contract without execution"
            );
            return GateConsultOutcome::AutoProceed;
        }

        GateConsultOutcome::FirstTime
    }
}

/// Outcome from [`ContractGate::gate_consult`].
enum GateConsultOutcome {
    /// This is the first time this gate instance has seen this slug. The
    /// caller should surface the full contract (when available).
    FirstTime,
    /// This slug has already been seen by this gate instance. Proceed with
    /// execution.
    Proceed,
    /// The global auto-proceed safety net has fired: too many unique fresh
    /// gate instances have all seen this contract without any executing.
    /// Proceed with execution regardless of local state.
    AutoProceed,
}

/// Outcome of consulting the gate for one action call.
pub enum GateDecision {
    /// Return this text to the model as a recoverable tool error; the model
    /// retries with the contract in context.
    Surface(String),
    /// Execute the action normally.
    Proceed,
}

/// Consult the gate before executing `action_slug` with the model's `args`.
///
/// On the FIRST consult for a slug this turn, if a fuller live contract can be
/// resolved, the gate compares the model's supplied `args` against it:
///
/// - **Args already satisfy the contract** (all required present, every supplied
///   key a known property, types compatible) → [`GateDecision::Proceed`]. The
///   model did not need the schema, so bouncing would be pure overhead — and, on
///   the weak text-mode `integrations_agent` path, forcing a needless retry lets
///   a Kimi-family model corrupt the re-issued call (`<|"|>` sentinel-token leak)
///   and loop forever without ever executing (#5119).
/// - **Args do NOT satisfy the contract** (missing required, unknown key, wrong
///   type — i.e. the model *guessed*) → [`GateDecision::Surface`] with the
///   formatted contract, exactly the case the gate exists for (#4853).
///
/// The slug is marked seen on this first consult either way, so every later
/// consult — and any consult where no live contract is available (unconfigured
/// client, unknown action, network miss) — returns [`GateDecision::Proceed`]:
/// the gate never blocks an action more than once and never blocks when it
/// cannot help.
///
/// ## Auto-proceed safety net (#5119)
///
/// When the main agent re-delegates to a fresh `integrations_agent` sub-agent,
/// each new spawn builds fresh tools with fresh [`ContractGate`] instances.
/// Every fresh gate sees each slug for the "first time" and surfaces the
/// full contract — so the action never executes, looping forever.
///
/// A process-wide consult counter tracks how many fresh gate instances have
/// consulted each slug. After [`AUTO_PROCEED_THRESHOLD`] (3+) fresh instances
/// have surfaced the same contract, the gate auto-proceeds: the model has
/// been given the schema across multiple iterations without advancing, and
/// the next call should execute instead of surfacing the contract again.
pub async fn consult(
    gate: &ContractGate,
    config: &Config,
    action_slug: &str,
    args: &serde_json::Value,
) -> GateDecision {
    // Consult the gate (instance-local seen-set + global auto-proceed check).
    // The lock is released before any await, so concurrent sibling calls and
    // the retry proceed without contention.
    match gate.gate_consult(action_slug) {
        // Auto-proceed safety net has fired: too many fresh instances have
        // surfaced this contract. Execute immediately.
        GateConsultOutcome::AutoProceed => {
            tracing::warn!(
                target: "composio",
                slug = %action_slug,
                "[composio][contract-gate] auto-proceeding after threshold; executing without surfacing"
            );
            return GateDecision::Proceed;
        }
        // Already surfaced by this gate instance → proceed.
        GateConsultOutcome::Proceed => {
            tracing::debug!(
                target: "composio",
                slug = %action_slug,
                "[composio][contract-gate] contract already surfaced this turn; proceeding"
            );
            return GateDecision::Proceed;
        }
        // First time for this gate instance → check if we need to surface.
        GateConsultOutcome::FirstTime => {}
    }

    if let Some(contract) = lookup_contract(config, action_slug).await {
        // Validate-then-pass (#5119): only surface when the model actually needs
        // the schema. A call whose args already conform is executed directly.
        if args_satisfy_contract(args, &contract) {
            tracing::debug!(
                target: "composio",
                slug = %action_slug,
                "[composio][contract-gate] args already satisfy the live contract; proceeding without surfacing"
            );
            return GateDecision::Proceed;
        }
        tracing::debug!(
            target: "composio",
            slug = %action_slug,
            has_input_schema = contract.input_schema.is_some(),
            required_arg_count = contract.required_args.len(),
            "[composio][contract-gate] surfacing full contract before first execute"
        );
        return GateDecision::Surface(format_contract(action_slug, &contract));
    }

    tracing::debug!(
        target: "composio",
        slug = %action_slug,
        "[composio][contract-gate] no live contract available; proceeding without gating"
    );
    GateDecision::Proceed
}

/// Whether the model's supplied `args` already conform to `contract` — the test
/// that lets the gate execute a well-formed first call instead of bouncing it
/// (#5119). Conservative: an object whose required args are all present, whose
/// every supplied key is a known schema property, and whose values are
/// type-compatible with the schema. Anything short of that is treated as a
/// guess and surfaces the contract (#4853).
///
/// Type checks are intentionally lenient about stringified scalars (a model may
/// send `max_results: "10"`), so only a genuinely wrong shape — a string where
/// an array is required, an unknown/invented key, a missing required arg — fails.
/// When the schema publishes no `properties`, only the required-args presence
/// check applies.
fn args_satisfy_contract(args: &serde_json::Value, contract: &ToolContract) -> bool {
    let obj = match args.as_object() {
        Some(obj) => obj,
        // Non-object args satisfy the contract only when nothing is required
        // (e.g. a no-arg action called with `null`/absent args).
        None => return contract.required_args.is_empty(),
    };

    // Every required argument must be present and non-null.
    for req in &contract.required_args {
        match obj.get(req) {
            Some(v) if !v.is_null() => {}
            _ => return false,
        }
    }

    // If the schema publishes its properties, every supplied key must be known
    // (no invented args) and type-compatible. A hallucinated key or a
    // wrong-typed value is exactly the guess the gate exists to catch.
    if let Some(props) = contract
        .input_schema
        .as_ref()
        .and_then(|s| s.get("properties"))
        .and_then(|p| p.as_object())
    {
        for (key, value) in obj {
            // `connection_id` is an OpenHuman-injected routing parameter
            // (`ComposioActionTool::parameters_schema` / `ComposioExecuteTool`),
            // consumed before dispatch and absent from Composio's live catalog
            // `input_schema`. Skip it so a valid multi-account call isn't bounced
            // as an "unknown key" into the retry path this gate exists to avoid.
            if key == "connection_id" {
                continue;
            }
            match props.get(key) {
                None => return false,
                Some(prop) => {
                    if let Some(expected) = prop.get("type").and_then(|t| t.as_str()) {
                        if !json_value_matches_type(value, expected) {
                            return false;
                        }
                    }
                }
            }
        }
    }

    true
}

/// Loose JSON-Schema scalar/compound `type` check used by
/// [`args_satisfy_contract`]. Numeric/boolean types also accept a string that
/// parses to that type, so a model sending `"10"` for an `integer` field is not
/// treated as a schema violation. An unrecognised or union `type` (the
/// `and_then(as_str)` returns `None` for a `["string","null"]` array) is never
/// reached here, so callers simply skip the check — lenient by construction.
fn json_value_matches_type(value: &serde_json::Value, expected: &str) -> bool {
    match expected {
        "string" => value.is_string(),
        "integer" => {
            value.is_i64()
                || value.is_u64()
                || value
                    .as_str()
                    .is_some_and(|s| s.trim().parse::<i64>().is_ok())
        }
        "number" => {
            value.is_number()
                || value
                    .as_str()
                    .is_some_and(|s| s.trim().parse::<f64>().is_ok())
        }
        "boolean" => {
            value.is_boolean()
                || value
                    .as_str()
                    .is_some_and(|s| matches!(s.trim(), "true" | "false"))
        }
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        // Unknown/unsupported type keyword → don't reject on type grounds.
        _ => true,
    }
}

/// Resolve the full live contract for `action_slug` from the process-cached
/// live toolkit catalog. Returns `None` when the toolkit can't be derived, the
/// catalog can't be fetched (unconfigured / offline — `fetch_live_toolkit_catalog`
/// degrades to `None`), or the action isn't in it.
async fn lookup_contract(config: &Config, action_slug: &str) -> Option<ToolContract> {
    let toolkit = toolkit_from_slug(action_slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    contracts
        .into_iter()
        .find(|c| c.slug.eq_ignore_ascii_case(action_slug))
}

/// Render the contract into a compact instruction for the model. Contains only
/// the provider's own action description + JSON schema — no user data / PII.
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

// The live-catalog cache these tests seed now lives in this domain, which is
// always compiled — so they run in every build, including the `flows`-off lane
// the gate itself used to be absent from.
#[cfg(test)]
#[path = "contract_gate_tests.rs"]
mod tests;
