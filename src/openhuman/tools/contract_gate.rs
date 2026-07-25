//! Contract gate for late-bound tool calls (#4853).
//!
//! A **late-bound** tool is one whose real contract is not in the tool list the
//! model was given. Four surfaces share that shape:
//!
//! | Surface | What the model sees | Where the real contract lives |
//! |---------|---------------------|-------------------------------|
//! | [`GateTarget::Composio`] per-action tools | the thin spawn-time `list_tools` entry — a one-line description with an often-absent parameter schema | the live toolkit catalog |
//! | [`GateTarget::Composio`] via `composio_execute` | an opaque `{tool, arguments}` dispatcher schema | the live toolkit catalog |
//! | [`GateTarget::McpRegistry`] via `mcp_registry_tool_call` | `{server_id, tool_name, arguments}` — `arguments` is a bare `object` | the connected server's advertised `input_schema` |
//! | [`GateTarget::Workflow`] via `run_workflow` | `{workflow_id, inputs}` — `inputs` is a bare `object` | the workflow's declared `[[inputs]]` block |
//!
//! In every case the model composes the call before the real schema is in
//! context and guesses argument formats — most visibly, it sends Gmail `query`
//! strings without the quoting Gmail search syntax requires, so
//! `GMAIL_FETCH_EMAILS` returns zero results.
//!
//! The gate makes the full contract enter context BEFORE execution: on the
//! first call to a target this turn, if a fuller contract can be resolved, it
//! is returned as a recoverable tool error instead of executing. The retry —
//! now with the schema/description in context — proceeds normally. This mirrors
//! the discover-then-call discipline the dispatchers already expect
//! (`composio_list_tools` → `composio_execute`, `mcp_registry_list_tools` →
//! `mcp_registry_tool_call`, `describe_workflow` → `run_workflow`), but
//! enforces it instead of merely documenting it.
//!
//! Two properties keep the gate from ever standing between the model and a call
//! it could already make correctly:
//!
//! - **Validate-then-pass** (#5119): a first call whose args already conform to
//!   the resolved contract executes directly — the model did not need the
//!   schema, so bouncing it would be pure overhead.
//! - **Auto-proceed safety net** (#5119): a process-wide counter breaks the
//!   re-delegation loop where each fresh sub-agent spawn builds a fresh gate.
//!
//! Both apply uniformly to every target kind, so a workflow or MCP call is
//! gated on exactly the terms a Composio action already is.
//!
//! Lives under `openhuman/tools/` rather than in any one domain because it is
//! consulted from `composio/`, `mcp_registry/`, and `agent/tools/` alike.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::OnceLock;

use crate::openhuman::config::Config;
// Each target kind's contract source lives behind that domain's Cargo feature
// (#4912), except Composio's: the live toolkit catalog moved into the composio
// domain itself and is always compiled. With a domain compiled out the gate has
// no contract to surface for its targets and proceeds, so each remaining lookup
// is gated in lockstep with its source.
use crate::openhuman::integrations::composio::catalog::fetch_live_toolkit_catalog;
use crate::openhuman::integrations::composio::providers::toolkit_from_slug;

/// Record of which target contracts have already been surfaced to the model,
/// so the gate blocks a given target at most once per gate instance.
///
/// One [`ContractGate`] is held per gated tool instance — a
/// [`crate::openhuman::integrations::composio::action_tool::ComposioActionTool`], the
/// `composio_execute` dispatcher, `mcp_registry_tool_call`, or `run_workflow`.
/// Those tools are constructed fresh per agent spawn and live for that spawn's
/// tool loop. That loop is a single agent turn in the common case, so "seen"
/// behaves as per-turn state without any task-local plumbing — but a long-lived
/// spawn can span multiple turns, and this gate does NOT reset when the
/// surfaced schema drops out of context via compaction (tracked as follow-up).
/// Interior-mutable so the gate can record state through the tool's `&self`
/// `execute`.
///
/// Entries are keyed by [`GateTarget::key`], so the same gate instance can
/// track several targets (the dispatchers see many) without cross-kind
/// collisions.
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
    ///
    /// `key` is a [`GateTarget::key`] — already normalised for its kind (a
    /// Composio slug is upper-cased there, an MCP `server/tool` pair and a
    /// workflow id keep their exact case), so no further folding happens here.
    fn gate_consult(&self, key: &str) -> GateConsultOutcome {
        let norm = key.to_string();

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
                target: "contract_gate",
                key = %key,
                global_count,
                "[contract-gate] auto-proceeding after {global_count} fresh instances surfaced this contract without execution"
            );
            return GateConsultOutcome::AutoProceed;
        }

        GateConsultOutcome::FirstTime
    }
}

/// Outcome from [`ContractGate::gate_consult`].
enum GateConsultOutcome {
    /// This is the first time this gate instance has seen this target. The
    /// caller should surface the full contract (when available).
    FirstTime,
    /// This target has already been seen by this gate instance. Proceed with
    /// execution.
    Proceed,
    /// The global auto-proceed safety net has fired: too many unique fresh
    /// gate instances have all seen this contract without any executing.
    /// Proceed with execution regardless of local state.
    AutoProceed,
}

/// A late-bound call the gate can front: what to resolve a contract for.
///
/// Constructed by the calling tool from its own already-validated arguments, so
/// the gate never re-parses a tool's argument shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateTarget {
    /// A Composio action slug — from `composio_execute`'s `tool` argument, or a
    /// per-action tool whose own name IS the slug.
    Composio(String),
    /// A tool on a connected MCP-registry server, addressed by
    /// `mcp_registry_tool_call`'s `(server_id, tool_name)`.
    McpRegistry { server: String, tool: String },
    /// An installed workflow, addressed by `run_workflow`'s `workflow_id`.
    Workflow(String),
}

impl GateTarget {
    /// Stable identity for the seen-set and the process-wide consult counter.
    /// Namespaced by kind so an MCP tool and a workflow that share a name are
    /// never conflated; Composio slugs are upper-cased because the model's
    /// casing varies while the action is the same.
    fn key(&self) -> String {
        match self {
            GateTarget::Composio(slug) => format!("composio:{}", slug.to_ascii_uppercase()),
            GateTarget::McpRegistry { server, tool } => format!("mcp:{server}:{tool}"),
            GateTarget::Workflow(id) => format!("workflow:{id}"),
        }
    }

    /// How the target is named back to the model in the surfaced contract.
    fn label(&self) -> String {
        match self {
            GateTarget::Composio(slug) => format!("`{slug}`"),
            GateTarget::McpRegistry { server, tool } => {
                format!("MCP tool `{tool}` on server `{server}`")
            }
            GateTarget::Workflow(id) => format!("workflow `{id}`"),
        }
    }

    /// The instruction that closes a surfaced contract — names the exact call
    /// the model should re-issue, and the argument field to fill in.
    fn retry_hint(&self) -> &'static str {
        match self {
            GateTarget::Composio(_) => "Then call the action again with the corrected arguments.",
            GateTarget::McpRegistry { .. } => {
                "Then re-issue the SAME `mcp_registry_tool_call` with an `arguments` object that \
                 conforms to this schema."
            }
            GateTarget::Workflow(_) => {
                "Then re-issue the SAME `run_workflow` call with an `inputs` object covering every \
                 required field."
            }
        }
    }

    /// Argument keys OpenHuman injects that are absent from the target's own
    /// published schema, so [`args_satisfy_contract`] must not read them as
    /// invented keys.
    fn injected_arg_keys(&self) -> &'static [&'static str] {
        match self {
            // `connection_id` is an OpenHuman-injected routing parameter
            // (`ComposioActionTool::parameters_schema` / `ComposioExecuteTool`),
            // consumed before dispatch and absent from Composio's live catalog
            // `input_schema`. Skip it so a valid multi-account call isn't bounced
            // as an "unknown key" into the retry path this gate exists to avoid.
            GateTarget::Composio(_) => &["connection_id"],
            GateTarget::McpRegistry { .. } | GateTarget::Workflow(_) => &[],
        }
    }
}

/// A resolved contract, normalised across the target kinds so one validator and
/// one formatter serve all of them.
///
/// Inert: `serde_json` + `String` only, with no reach into any gated domain, so
/// it compiles in every feature configuration (the per-kind *lookups* below are
/// what the domain features gate).
pub struct GatedContract {
    /// The provider's own description of the target, when it publishes one.
    description: Option<String>,
    /// Argument names the target requires.
    required_args: Vec<String>,
    /// The target's full input JSON Schema, when it publishes one.
    input_schema: Option<serde_json::Value>,
}

/// Outcome of consulting the gate for one late-bound call.
pub enum GateDecision {
    /// Return this text to the model as a recoverable tool error; the model
    /// retries with the contract in context.
    Surface(String),
    /// Execute the action normally.
    Proceed,
}

/// Consult the gate before executing `target` with the model's `args`.
///
/// `args` is the target's OWN argument object — `arguments` for
/// `composio_execute` / `mcp_registry_tool_call`, `inputs` for `run_workflow`,
/// the whole call args for a per-action Composio tool — not the dispatcher
/// envelope, whose `tool` / `server_id` / `workflow_id` keys the caller has
/// already consumed to build the target.
///
/// `config` is the caller's live snapshot, so a mid-session `composio.mode` /
/// credential / workspace change routes the gate's lookup and the caller's own
/// dispatch through the SAME config. Pass `None` only when the caller genuinely
/// has none; the config-backed lookups then find nothing and the gate proceeds.
///
/// On the FIRST consult for a target this turn, if a fuller contract can be
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
/// The target is marked seen on this first consult either way, so every later
/// consult — and any consult where no contract is available (unconfigured
/// client, unknown action, disconnected server, uninstalled workflow, network
/// miss) — returns [`GateDecision::Proceed`]: the gate never blocks a call more
/// than once and never blocks when it cannot help.
///
/// ## Auto-proceed safety net (#5119)
///
/// When the main agent re-delegates to a fresh `integrations_agent` sub-agent,
/// each new spawn builds fresh tools with fresh [`ContractGate`] instances.
/// Every fresh gate sees each target for the "first time" and surfaces the
/// full contract — so the call never executes, looping forever.
///
/// A process-wide consult counter tracks how many fresh gate instances have
/// consulted each target. After [`AUTO_PROCEED_THRESHOLD`] (3+) fresh instances
/// have surfaced the same contract, the gate auto-proceeds: the model has
/// been given the schema across multiple iterations without advancing, and
/// the next call should execute instead of surfacing the contract again.
pub async fn consult(
    gate: &ContractGate,
    config: Option<&Config>,
    target: &GateTarget,
    args: &serde_json::Value,
) -> GateDecision {
    let key = target.key();
    // Consult the gate (instance-local seen-set + global auto-proceed check).
    // The lock is released before any await, so concurrent sibling calls and
    // the retry proceed without contention.
    match gate.gate_consult(&key) {
        // Auto-proceed safety net has fired: too many fresh instances have
        // surfaced this contract. Execute immediately.
        GateConsultOutcome::AutoProceed => {
            tracing::warn!(
                target: "contract_gate",
                key = %key,
                "[contract-gate] auto-proceeding after threshold; executing without surfacing"
            );
            return GateDecision::Proceed;
        }
        // Already surfaced by this gate instance → proceed.
        GateConsultOutcome::Proceed => {
            tracing::debug!(
                target: "contract_gate",
                key = %key,
                "[contract-gate] contract already surfaced this turn; proceeding"
            );
            return GateDecision::Proceed;
        }
        // First time for this gate instance → check if we need to surface.
        GateConsultOutcome::FirstTime => {}
    }

    // Each kind's contract source sits behind its domain feature. With that
    // domain compiled out there is nothing fuller to surface, so the gate
    // proceeds — the call still runs, it just misses the pre-execute nudge.
    if let Some(contract) = lookup_contract(config, target).await {
        // Validate-then-pass (#5119): only surface when the model actually needs
        // the schema. A call whose args already conform is executed directly.
        if args_satisfy_contract(args, &contract, target.injected_arg_keys()) {
            tracing::debug!(
                target: "contract_gate",
                key = %key,
                "[contract-gate] args already satisfy the contract; proceeding without surfacing"
            );
            return GateDecision::Proceed;
        }
        tracing::debug!(
            target: "contract_gate",
            key = %key,
            has_input_schema = contract.input_schema.is_some(),
            required_arg_count = contract.required_args.len(),
            "[contract-gate] surfacing full contract before first execute"
        );
        return GateDecision::Surface(format_contract(target, &contract));
    }

    tracing::debug!(
        target: "contract_gate",
        key = %key,
        "[contract-gate] no contract available; proceeding without gating"
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
///
/// `injected_keys` are OpenHuman-added routing arguments that the target's own
/// schema does not publish (see [`GateTarget::injected_arg_keys`]).
fn args_satisfy_contract(
    args: &serde_json::Value,
    contract: &GatedContract,
    injected_keys: &[&str],
) -> bool {
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
            // Skip OpenHuman-injected routing parameters, which the target's own
            // published schema never lists — so a valid call carrying one isn't
            // bounced as an "unknown key" into the retry path this gate exists
            // to avoid.
            if injected_keys.contains(&key.as_str()) {
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

/// Resolve `target`'s full contract from its own source. Returns `None`
/// whenever the source can't answer — the target's domain is compiled out, the
/// config is missing, the catalog can't be fetched (unconfigured / offline), the
/// server isn't connected, or the target simply doesn't exist. Every such case
/// means the gate has nothing better to show the model, so [`consult`] proceeds.
async fn lookup_contract(config: Option<&Config>, target: &GateTarget) -> Option<GatedContract> {
    match target {
        GateTarget::Composio(slug) => lookup_composio_contract(config?, slug).await,
        GateTarget::McpRegistry { server, tool } => lookup_mcp_contract(server, tool).await,
        GateTarget::Workflow(id) => lookup_workflow_contract(config?, id),
    }
}

/// The Composio action contract, from the process-cached live toolkit catalog.
async fn lookup_composio_contract(config: &Config, action_slug: &str) -> Option<GatedContract> {
    let toolkit = toolkit_from_slug(action_slug)?;
    let contracts = fetch_live_toolkit_catalog(config, &toolkit).await?;
    let contract = contracts
        .into_iter()
        .find(|c| c.slug.eq_ignore_ascii_case(action_slug))?;
    Some(GatedContract {
        description: contract.description,
        required_args: contract.required_args,
        input_schema: contract.input_schema,
    })
}

/// The MCP tool contract, from the live connected-server map.
#[cfg(feature = "mcp")]
async fn lookup_mcp_contract(server: &str, tool: &str) -> Option<GatedContract> {
    let connected = crate::openhuman::mcp::registry::connections::all_connected_tools().await;
    resolve_mcp_contract(connected, server, tool)
}

/// Pick `(server, tool)` out of a connected-tools snapshot. The server
/// advertises a JSON Schema; its `required` array is the required-arg list.
/// Split from the live-map read so it is unit-testable against a synthetic
/// snapshot.
#[cfg(feature = "mcp")]
fn resolve_mcp_contract(
    connected: Vec<(
        String,
        String,
        crate::openhuman::mcp::registry::types::McpTool,
    )>,
    server: &str,
    tool: &str,
) -> Option<GatedContract> {
    let (_, _, advertised) = connected
        .into_iter()
        .find(|(server_id, _, t)| server_id == server && t.name == tool)?;
    Some(GatedContract {
        description: advertised.description,
        required_args: required_from_schema(&advertised.input_schema),
        input_schema: Some(advertised.input_schema),
    })
}

/// `mcp` off ⇒ no registry connections compiled in ⇒ nothing to surface.
#[cfg(not(feature = "mcp"))]
async fn lookup_mcp_contract(_server: &str, _tool: &str) -> Option<GatedContract> {
    None
}

/// The workflow contract, from its installed `[[inputs]]` block. Synthesised
/// into a JSON Schema so the shared validator and formatter treat a workflow
/// exactly like any other target.
///
/// The caller has already applied the active profile's skill allowlist before
/// consulting the gate, so this never renders a scoped-out workflow's contract.
#[cfg(feature = "skills")]
fn lookup_workflow_contract(config: &Config, workflow_id: &str) -> Option<GatedContract> {
    let def = crate::openhuman::skills::registry::get_workflow(&config.workspace_dir, workflow_id)?;
    let mut properties = serde_json::Map::new();
    let mut required_args = Vec::new();
    for input in &def.inputs {
        let mut prop = serde_json::Map::new();
        if let Some(kind) = input.kind.as_deref().filter(|k| !k.is_empty()) {
            prop.insert("type".to_string(), serde_json::Value::from(kind));
        }
        if !input.description.is_empty() {
            prop.insert(
                "description".to_string(),
                serde_json::Value::from(input.description.as_str()),
            );
        }
        properties.insert(input.name.clone(), serde_json::Value::Object(prop));
        if input.required {
            required_args.push(input.name.clone());
        }
    }
    Some(GatedContract {
        // A workflow's user-facing "what is this for" text is its agent
        // definition's `when_to_use` — there is no separate description field.
        description: Some(def.definition.when_to_use.trim().to_string()).filter(|d| !d.is_empty()),
        input_schema: Some(serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required_args,
        })),
        required_args,
    })
}

/// `skills` off ⇒ no workflow registry compiled in ⇒ nothing to surface.
#[cfg(not(feature = "skills"))]
fn lookup_workflow_contract(_config: &Config, _workflow_id: &str) -> Option<GatedContract> {
    None
}

/// The `required` array of a JSON Schema, as a list of argument names. Empty
/// when the schema publishes none.
#[cfg(feature = "mcp")]
fn required_from_schema(schema: &serde_json::Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Render the contract into a compact instruction for the model. Contains only
/// the target's own published description + JSON schema — no user data / PII.
fn format_contract(target: &GateTarget, contract: &GatedContract) -> String {
    let mut out = format!(
        "Before running {}, read its full contract below and then re-issue \
         the call with arguments that match it exactly.\n\n",
        target.label()
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
        None => out.push_str("Input JSON schema: not published for this target.\n"),
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
         subject:\"quarterly report\"). ",
    );
    out.push_str(target.retry_hint());
    out
}

// The gate's kind-agnostic logic (target identity, arg validation, contract
// rendering) compiles in every configuration, so the test module does too. The
// Composio block does as well, since the live-catalog cache it seeds now lives
// in the composio domain and is always compiled. The MCP and workflow blocks
// each carry their own domain feature gate.
#[cfg(test)]
#[path = "contract_gate_tests.rs"]
mod tests;
