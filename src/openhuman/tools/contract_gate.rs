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
//! - **Auto-proceed safety net** (#5119): a per-turn counter breaks the
//!   re-delegation loop where each fresh sub-agent spawn builds a fresh gate.
//!
//! Both apply uniformly to every target kind, so a workflow or MCP call is
//! gated on exactly the terms a Composio action already is.
//!
//! ## Presence is read from the transcript, and survives any rewrite
//!
//! "Already surfaced" is only useful while the contract is still in front of the
//! model. Summarisation, microcompact tool-body blanking, hard trimming, and
//! result-size caps can all drop or rewrite a delivered contract mid-turn; a
//! gate that kept counting it as seen would let the model call the tool with a
//! schema it can no longer read — the exact failure this gate exists to prevent.
//!
//! So presence is **derived from the transcript**, not tracked as tool state.
//! Each delivered contract leads with a `[contract-gate:<slug list>]` marker and
//! its payload's hash is recorded in [`DELIVERED`]. Before every model call
//! [`refresh_present`] rescans the tool messages, re-hashes each marker's
//! payload, and credits the slugs **only on an exact match**. That is correct
//! across every context-management path by construction:
//!
//! - a contract still present byte-for-byte (including in a resumed sub-agent's
//!   history) hashes equal → present, no redundant re-delivery;
//! - one summarised, blanked, truncated, or whitespace-collapsed no longer
//!   hashes equal → absent → re-delivered once.
//!
//! Recognition is a **fixed-prefix compare at byte 0**, never a substring
//! search: the marker leads the message, so the rescan is one `strip_prefix` per
//! tool message rather than a scan of the whole transcript, and a model echoing
//! the marker mid-text cannot spoof presence. Only tool-role messages are
//! scanned. The marker itself carries slugs only — short and whitespace-free, so
//! it survives the reformatters that the payload hash is there to detect — and
//! packs several comma-separated slugs when one message describes several
//! targets, keeping the single-marker-at-the-start layout intact.
//!
//! ## Why this cannot become a re-delivery loop
//!
//! A presence check that answers "absent" costs a re-delivery, so the mechanism
//! is only safe if it is guaranteed to eventually answer "present". Two
//! properties give that, and both are load-bearing:
//!
//! 1. **A hash miss skips that message; it never rejects the target**
//!    ([`credit_marker`]). The scan accumulates over the whole transcript, so a
//!    lookalike — a tool echoing the syntax, a stale copy since rewritten —
//!    contributes nothing rather than vetoing a genuine copy elsewhere.
//! 2. **Delivery overwrites the recorded hash** ([`record_delivered`]). Every
//!    delivery records the hash of the exact bytes it emits, so the newest
//!    delivery is always creditable — the next rescan finds it intact and the
//!    retry runs.
//!
//! Together: whatever the transcript already holds, the delivery the gate just
//! made will be credited on the next model call, so a target can be re-delivered
//! at most once per compaction rather than on every call.
//!
//! Lives under `openhuman/tools/` rather than in any one domain because it is
//! consulted from `composio/`, `mcp_registry/`, and `agent/tools/` alike.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use crate::openhuman::config::Config;
// Each target kind's contract source lives behind that domain's Cargo feature
// (#4912), except Composio's: the live toolkit catalog moved into the composio
// domain itself and is always compiled. With a domain compiled out the gate has
// no contract to surface for its targets and proceeds, so each remaining lookup
// is gated in lockstep with its source.
use crate::openhuman::integrations::composio::catalog::fetch_live_toolkit_catalog;
use crate::openhuman::integrations::composio::providers::toolkit_from_slug;

/// Record of which target contracts this gate instance has already surfaced.
///
/// One [`ContractGate`] is held per gated tool instance — a
/// [`crate::openhuman::integrations::composio::action_tool::ComposioActionTool`], the
/// `composio_execute` dispatcher, `mcp_registry_tool_call`, or `run_workflow`.
/// Those tools are constructed fresh per agent spawn and live for that spawn's
/// tool loop.
///
/// This set does **not** decide presence — [`refresh_present`] does, from the
/// transcript. It only bounds this instance: having surfaced a contract once,
/// the gate does not surface it again even if the model never got it into
/// context, so a single instance can never loop. Entries are keyed by
/// [`GateTarget::key`], so one instance can track several targets (the
/// dispatchers see many) without cross-kind collisions. Interior-mutable so the
/// gate can record through the tool's `&self` `execute`.
///
/// ## Auto-proceed safety net (#5119)
///
/// When the main agent re-delegates to a fresh `integrations_agent` sub-agent,
/// each spawn creates a new tool with a fresh `ContractGate`. Without a
/// cross-instance consult counter, every fresh gate would surface the same
/// contract and the action would never execute — causing an infinite loop
/// ("same tool call 3× in a row" guard).
///
/// [`TurnState::first_consults`] tracks how many *unique gate instances* have
/// consulted each key for the "first time". After 3+ fresh instances have all
/// surfaced the same contract, the next instance auto-proceeds: the model has
/// clearly been given the schema and needs execution, not another schema dump.
///
/// The threshold is generous (3+ instances = at least 3 surfaced contracts in
/// different sub-agent iterations) so that the normal surface-once-then-execute
/// path within a single spawn is never disrupted. The counter is **per turn**,
/// not per process: a loop is an in-flight condition, so carrying its count into
/// later turns would permanently suppress the gate for that target — the model
/// would then guess arguments against a schema it never saw.
#[derive(Default)]
pub struct ContractGate {
    seen: Mutex<HashSet<String>>,
}

/// Process-global map from a marker's exact slug list (as built by
/// [`normalize_slug_list`]) to the XXH3-64 hash of the **payload** that followed
/// the marker at delivery (everything after the marker's `]`).
///
/// Presence is decided in [`refresh_present`] by re-hashing a transcript
/// marker's payload and matching it here — so the transcript marker itself
/// carries only slugs (short, whitespace-free), while a payload later
/// reformatted downstream no longer matches and the contract is re-delivered
/// (fail-safe). Global rather than per-run so a contract delivered in one run
/// stays creditable in a later run or a resume; it only ever credits a marker
/// whose payload is present *and* byte-identical in that run's own transcript.
static DELIVERED: LazyLock<Mutex<HashMap<String, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record `hash` as the creditable payload for `slug_list`, **overwriting** any
/// hash previously recorded for it.
///
/// The overwrite is deliberate, and it is the second half of the loop-breaking
/// argument (the first being that a hash miss never rejects a target — see
/// [`credit_marker`]). Every delivery records the hash of the exact bytes it is
/// about to emit, so **the most recent delivery is always creditable**: the next
/// [`refresh_present`] finds it intact in the transcript, credits the slug, and
/// the retry runs. Termination therefore does not depend on any earlier copy
/// surviving.
///
/// Keeping the first hash instead would be the bug: if the contract text ever
/// differed between deliveries — the provider republished the schema, or a
/// discovery listing recorded a different rendering first — the newly delivered
/// payload would hash to a value that was never recorded, so it could never be
/// credited, and the gate would re-deliver it on every single call forever.
///
/// The cost of overwriting is that an older copy still sitting in the transcript
/// stops matching. That is harmless: it only forfeits crediting from a stale
/// copy, and the fresh delivery right next to it credits instead.
fn record_delivered(slug_list: String, hash: u64) {
    if let Ok(mut delivered) = DELIVERED.lock() {
        delivered.insert(slug_list, hash);
    }
}

/// Per-turn gate state, scoped as a task-local by [`with_turn`] around the
/// top-level turn so every tool `execute()` on that task — including every
/// nested sub-agent's — reads the same instance.
#[derive(Default)]
struct TurnState {
    /// Target key → how many distinct [`ContractGate`] instances surfaced it
    /// this turn. Drives the auto-proceed safety net (#5119).
    first_consults: Mutex<HashMap<String, u32>>,
}

tokio::task_local! {
    /// The current turn's auto-proceed accounting. Absent outside a turn (a
    /// direct CLI/RPC tool invocation, a unit test), where the process-wide
    /// fallback stands in.
    static TURN_STATE: Arc<TurnState>;

    /// The current run's transcript-derived presence set, rebuilt by
    /// [`refresh_present`] before every model call. Scoped per RUN — a
    /// sub-agent has its own transcript, so it must not read its parent's.
    static RUN_PRESENCE: Arc<Mutex<HashSet<String>>>;
}

/// Stand-in state for calls that reach a gated tool outside any turn — a direct
/// CLI/RPC tool invocation, or a unit test. Keeps the gate functional there
/// without pretending the process has turn boundaries it does not.
fn fallback_turn_state() -> &'static Arc<TurnState> {
    static FALLBACK: OnceLock<Arc<TurnState>> = OnceLock::new();
    FALLBACK.get_or_init(|| Arc::new(TurnState::default()))
}

/// Run `f` with fresh per-turn auto-proceed accounting.
///
/// Wrap the **top-level** turn only. Nested sub-agent runs must NOT re-scope: a
/// fresh count per spawn is exactly the condition the safety net exists to
/// detect, so re-scoping there would defeat it. Task-locals nest, so a child
/// that skips this call transparently reads its parent's state.
pub async fn with_turn<F: std::future::Future>(f: F) -> F::Output {
    TURN_STATE.scope(Arc::new(TurnState::default()), f).await
}

/// Run `f` with a fresh per-run presence set. Wrap **every** run, sub-agents
/// included: presence describes one transcript, and a sub-agent's is its own.
pub async fn with_run_presence<F: std::future::Future>(f: F) -> F::Output {
    RUN_PRESENCE
        .scope(Arc::new(Mutex::new(HashSet::new())), f)
        .await
}

/// Rebuild this run's presence set from the transcript's tool messages.
///
/// Call from `before_model`, passing the text of every message the **host**
/// wrote — tool rows, and the user rows a text-mode turn renders results into.
/// Role filtering is the caller's job, and the one role it must exclude is
/// assistant: a model echoing the marker in its own prose must not be able to
/// claim presence. It cannot reach the other two.
///
/// Each text is matched with a fixed-prefix compare at byte 0 (see the module
/// doc), and its payload re-hashed against [`DELIVERED`]; only an exact match
/// credits the marker's slugs.
///
/// **Every message is scanned and the credits accumulate — a mismatch is never
/// a verdict on the target.** A message that merely looks like a marker, or a
/// stale delivery whose payload was rewritten, contributes nothing and moves on;
/// a genuine copy later in the transcript still credits. Short-circuiting on the
/// first hash miss would let one lookalike make the contract permanently
/// un-creditable, and the gate would re-deliver it on every call forever.
///
/// Outside a run scope this is a no-op: there is no presence set to fill, and
/// the gate falls back to its per-instance bound.
pub fn refresh_present(tool_message_texts: impl IntoIterator<Item = String>) {
    let mut present = HashSet::new();
    if let Ok(delivered) = DELIVERED.lock() {
        for text in tool_message_texts {
            credit_marker(&text, &delivered, &mut present);
        }
    }
    let credited = present.len();
    let stored = RUN_PRESENCE
        .try_with(|set| {
            if let Ok(mut guard) = set.lock() {
                *guard = present;
            }
        })
        .is_ok();
    tracing::trace!(
        target: "contract_gate",
        credited,
        stored,
        "[contract-gate] presence rebuilt from the transcript"
    );
}

/// Whether `key`'s contract is verifiably in this run's transcript. `false`
/// outside a run scope — the gate then falls back to its per-instance bound.
fn is_present(key: &str) -> bool {
    RUN_PRESENCE
        .try_with(|set| set.lock().map(|guard| guard.contains(key)).unwrap_or(false))
        .unwrap_or(false)
}

/// The state backing this task's turn, or the process-wide fallback.
fn turn_state() -> Arc<TurnState> {
    TURN_STATE
        .try_with(Arc::clone)
        .unwrap_or_else(|_| Arc::clone(fallback_turn_state()))
}

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
    /// 2. The turn's first-time consult counter for this slug is incremented.
    /// 3. If that counter exceeds [`AUTO_PROCEED_THRESHOLD`], the gate
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

        // 2. Increment this turn's first-time consult counter.
        let global_count = {
            let state = turn_state();
            let mut map = state
                .first_consults
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
/// ## Presence short-circuit
///
/// Before any of that, a target whose contract [`refresh_present`] verified is
/// still in this run's transcript proceeds immediately — including from a fresh
/// gate instance in a re-delegated sub-agent, which is why this check comes
/// first. Conversely a contract the transcript has since lost (summarised,
/// blanked, trimmed, or rewritten) stops being present and is re-delivered.
///
/// ## Auto-proceed safety net (#5119)
///
/// When the main agent re-delegates to a fresh `integrations_agent` sub-agent,
/// each new spawn builds fresh tools with fresh [`ContractGate`] instances.
/// Every fresh gate sees each target for the "first time" and surfaces the
/// full contract — so the call never executes, looping forever.
///
/// A per-turn consult counter tracks how many fresh gate instances have
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

    // The contract is verifiably in this run's transcript → the model can read
    // it → run the tool. Checked before the instance-local bookkeeping so a
    // fresh gate in a re-delegated sub-agent doesn't re-surface what the model
    // is already looking at.
    if is_present(&key) {
        tracing::debug!(
            target: "contract_gate",
            key = %key,
            "[contract-gate] contract present in the transcript; proceeding"
        );
        return GateDecision::Proceed;
    }

    // Consult the gate (instance-local seen-set + per-turn auto-proceed check).
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
        // Lead the delivery with the marker and record its payload hash, so the
        // next `refresh_present` credits this target only while the contract is
        // still in the transcript byte-for-byte.
        let slug_list = normalize_slug_list([key.clone()]);
        let (body, hash) = deliver_body(&slug_list, &format_contract(target, &contract));
        record_delivered(slug_list, hash);
        return GateDecision::Surface(body);
    }

    tracing::debug!(
        target: "contract_gate",
        key = %key,
        "[contract-gate] no contract available; proceeding without gating"
    );
    GateDecision::Proceed
}

// ── transcript marker ───────────────────────────────────────────────────────

/// Fixed opening of the transcript marker a contract-carrying tool message leads
/// with, placed at the START so a later model call recognises it with a
/// fixed-length prefix compare (no full-message scan). Closed by `]`, enclosing
/// only the marker's **slug list** — `<slug>[,<slug>…]`. There is no digest in
/// the marker itself; the payload hash lives in [`DELIVERED`], keyed by this
/// exact slug list. A gate delivery carries one slug; a full-schema discovery
/// tool packs several.
const MARKER_OPEN: &str = "[contract-gate:";

/// Separator between the slugs packed into one marker's slug list. A gate slug
/// never contains it — slugs are `composio:<SLUG>` / `mcp:<server>:<tool>` /
/// `workflow:<id>`, drawn from validated slugs, registry names, and directory
/// ids — and [`normalize_slug_list`] drops any slug that would (defence in
/// depth) so one pathological name can't corrupt the parse of the others.
const MARKER_SEP: char = ',';

/// Banner inserted right after the marker in every delivered contract. A weak
/// model reads the contract — the tool's *input schema* — as if it were the
/// tool's *output*, concludes "the call ran and returned nothing", and gives up
/// instead of retrying. State plainly that the tool did NOT run and a retry is
/// mandatory. The marker still leads the message, so the fixed-length prefix
/// scan is unaffected.
const RETRY_BANNER: &str = "This tool was NOT executed and returned NO result. You are seeing its \
     input contract because the tool must be read before it can run. This is not a failure, an \
     error, or an empty result — nothing has been searched, fetched, or run yet. To actually run \
     it, re-issue the SAME tool call now with arguments matching the schema below. Do NOT report \
     \"no results\" or stop: the call has not happened.";

/// XXH3-64 hash of a marker's **payload** (the bytes after the marker's `]`).
/// Recorded in [`DELIVERED`] at delivery and recomputed on rescan: a fast,
/// stable (cross-process reproducible) fingerprint that detects any downstream
/// reformat — summarizer, size cap, or the sub-agent handoff's
/// whitespace-collapse — of the delivered contract. Non-cryptographic: it guards
/// accidental mutation, not a forged collision.
fn payload_hash(payload: &str) -> u64 {
    xxhash_rust::xxh3::xxh3_64(payload.as_bytes())
}

/// Canonical slug-list string used **both** as a transcript marker's body and as
/// the [`DELIVERED`] key, so a marker and its recorded hash always agree. Slugs
/// are de-duplicated and **sorted** (order-independent), and any slug carrying
/// [`MARKER_SEP`] or `]` is dropped so one pathological name can't corrupt the
/// parse. Returns empty when nothing is creditable.
fn normalize_slug_list(slugs: impl IntoIterator<Item = String>) -> String {
    let mut v: Vec<String> = slugs
        .into_iter()
        .filter(|s| !s.is_empty() && !s.contains(MARKER_SEP) && !s.contains(']'))
        .collect();
    v.sort();
    v.dedup();
    v.join(",")
}

/// The transcript marker for a [`normalize_slug_list`] string:
/// `[contract-gate:<slug list>]` — slugs only, no digest.
fn contract_marker(slug_list: &str) -> String {
    format!("{MARKER_OPEN}{slug_list}]")
}

/// The tool-error body a delivered contract carries, **plus the payload hash**
/// the caller records in [`DELIVERED`] under `slug_list`. Layout: the marker
/// FIRST (so the rescan recognises it with a fixed-length prefix compare), then
/// the payload — [`RETRY_BANNER`] then the contract text. The hash covers that
/// exact payload, so a later rescan credits the slug only while the payload
/// survives byte-for-byte.
fn deliver_body(slug_list: &str, contract: &str) -> (String, u64) {
    let payload = format!("\n\n{RETRY_BANNER}\n\n{contract}");
    let hash = payload_hash(&payload);
    (format!("{}{payload}", contract_marker(slug_list)), hash)
}

/// True when `content` is a contract-gate delivery **this process actually
/// made** — it leads with [`MARKER_OPEN`] and its payload still hashes to the
/// value recorded at delivery.
///
/// The hash check is what makes this safe to act on. A bare "starts with the
/// marker" test would also match a tool result that happens to echo the syntax,
/// and every caller below grants an exemption on the strength of this answer.
///
/// Two callers:
/// - the content-rewriting `after_tool` hooks, which must leave a delivery
///   byte-for-byte or its payload hash stops matching and the contract can never
///   be credited as present;
/// - the repeated-tool-failure breaker, for which a delivery is an error but not
///   a failure (it hands the model the contract and expects a retry). A
///   *NotFound* message carries no marker, so a model looping on a bogus slug
///   still trips the breaker.
pub(crate) fn is_contract_delivery(content: &str) -> bool {
    let mut present = HashSet::new();
    if let Ok(delivered) = DELIVERED.lock() {
        credit_marker(content, &delivered, &mut present);
    }
    !present.is_empty()
}

/// The tool result a [`GateDecision::Surface`] has to be returned as.
///
/// One constructor for every gated call site, because the two properties that
/// make a delivery work are invisible when a site forgets them:
///
/// - it is an **error** result, so the model reads "the call did not run" and
///   retries; a success result reads as output and it moves on;
/// - it is **`trusted_verbatim`**, so hosts put the contract at byte 0 of its
///   own message. That is what lets the model copy argument names character for
///   character, and what keeps the payload hashing to its recorded value so
///   [`refresh_present`] can credit the contract as present.
///
/// A site that hand-rolls `ToolResult::error(contract)` still compiles and still
/// delivers something the model can read, so the loss shows up only as a gate
/// that re-delivers forever and a model that mis-copies argument names.
pub(crate) fn surface_result(contract: String) -> crate::openhuman::tools::ToolResult {
    crate::openhuman::tools::ToolResult::error(contract).mark_trusted_verbatim()
}

/// If `text` leads with a `[contract-gate:<slug list>]` marker whose payload
/// (everything after the marker's `]`) still hashes to the value recorded in
/// `delivered` for that exact slug list, credit every slug in the list into
/// `present`.
///
/// Recognition is a fixed-length prefix compare (marker at the START), so a
/// model echoing the marker mid-text can't spoof presence, and there is one
/// marker per message.
///
/// The hash match is the integrity gate: a delivered contract summarised,
/// truncated by a result-size cap, or whitespace-collapsed by the sub-agent
/// handoff cleaner no longer hashes to the recorded value, so it is not credited
/// and the gate re-delivers rather than treating a mutated (possibly partial)
/// body as present.
///
/// **A hash miss skips this message; it never rejects the target.** A message
/// that only looks like a marker — a tool echoing the syntax, a stale delivery
/// whose payload was since rewritten — must not be able to veto a genuine copy
/// elsewhere in the transcript. [`refresh_present`] therefore accumulates across
/// every tool message and a miss contributes nothing, so one intact delivery
/// anywhere is enough to let the call through. Treating a miss as "absent" and
/// stopping would let a single lookalike wedge the gate into re-delivering
/// forever.
fn credit_marker(text: &str, delivered: &HashMap<String, u64>, present: &mut HashSet<String>) {
    let Some(rest) = text.strip_prefix(MARKER_OPEN) else {
        return;
    };
    let Some(close) = rest.find(']') else {
        return;
    };
    let slug_list = &rest[..close];
    let payload = &rest[close + 1..];
    if delivered.get(slug_list) != Some(&payload_hash(payload)) {
        return;
    }
    for slug in slug_list.split(MARKER_SEP) {
        let slug = slug.trim();
        if !slug.is_empty() {
            present.insert(slug.to_string());
        }
    }
}

/// Prepend a **full-schema** discovery tool's presence marker to its output
/// `body`, returning the marker-led message the gate later credits — or `body`
/// unchanged when nothing is creditable — and record the payload hash in
/// [`DELIVERED`] under the marker's slug list.
///
/// So a later [`refresh_present`] credits every slug once the model has read the
/// full listing: a `describe_workflow` / `mcp_registry_list_tools` / full
/// `composio_list_tools` before the real call pays no redundant re-delivery.
/// Several slugs pack into the one marker, keeping the
/// single-marker-at-the-start layout the prefix scan depends on.
///
/// This function OWNS the marker↔body concatenation so the recorded hash covers
/// the exact bytes that follow the marker: the payload is `"\n\n" + body`, and
/// the returned string is `marker + payload`.
///
/// Only a rendering that puts the **full** contract in the model's context may
/// call this; a thin listing must not, or the gate would mark a contract the
/// model has never seen as present and stop gating it. The slugs come from
/// [`composio_key`] / [`mcp_key`] / [`workflow_key`] so they match exactly what
/// the gate later checks presence against.
pub(crate) fn prefix_with_present_marker(
    slugs: impl IntoIterator<Item = String>,
    body: &str,
) -> String {
    let slug_list = normalize_slug_list(slugs);
    if slug_list.is_empty() {
        return body.to_string();
    }
    let payload = format!("\n\n{body}");
    record_delivered(slug_list.clone(), payload_hash(&payload));
    format!("{}{payload}", contract_marker(&slug_list))
}

/// Which slugs `tool_message_texts` credit — the pure core of
/// [`refresh_present`], without the task-local write, so the scan's rules
/// (fixed-prefix recognition, hash integrity, accumulate-never-reject) are
/// testable directly.
#[cfg(test)]
fn credited_slugs(tool_message_texts: impl IntoIterator<Item = String>) -> HashSet<String> {
    let mut present = HashSet::new();
    if let Ok(delivered) = DELIVERED.lock() {
        for text in tool_message_texts {
            credit_marker(&text, &delivered, &mut present);
        }
    }
    present
}

/// Deliver `contract` for a single `key` exactly as [`consult`] would — marker,
/// banner, recorded payload hash — and return `(message, slug_list)`.
#[cfg(test)]
fn seed_delivery(key: &str, contract: &str) -> (String, String) {
    let slug_list = normalize_slug_list([key.to_string()]);
    let (body, hash) = deliver_body(&slug_list, contract);
    record_delivered(slug_list.clone(), hash);
    (body, slug_list)
}

/// Presence key for a Composio action slug — matches [`GateTarget::Composio`]'s
/// key so a `composio_list_tools` carrying the full schema credits the slug the
/// later `composio_execute` (or per-action tool) gates on.
pub(crate) fn composio_key(slug: &str) -> String {
    GateTarget::Composio(slug.to_string()).key()
}

/// Presence key for an MCP-registry `(server, tool)` — matches
/// [`GateTarget::McpRegistry`]'s key so `mcp_registry_list_tools` credits the
/// tool the later `mcp_registry_tool_call` gates on.
pub(crate) fn mcp_key(server: &str, tool: &str) -> String {
    GateTarget::McpRegistry {
        server: server.to_string(),
        tool: tool.to_string(),
    }
    .key()
}

/// Presence key for a workflow id — matches [`GateTarget::Workflow`]'s key so
/// `describe_workflow` credits the id the later `run_workflow` gates on.
pub(crate) fn workflow_key(id: &str) -> String {
    GateTarget::Workflow(id.to_string()).key()
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
