//! The always-on tool that stands in for every packed tool.

use std::sync::{Arc, RwLock, Weak};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::registry;
use crate::openhuman::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolSpec,
};
use tinytools::ToolRunContext;

pub const USE_SKILL: &str = "use_skill";

/// An `Arc`-shared, owned view of the tool registry a pack tool lives in.
type ToolVec = Arc<Vec<Box<dyn Tool>>>;

/// A non-owning view of the tool registry, kept to break the binding cycle.
type ToolRegistryRef = Weak<Vec<Box<dyn Tool>>>;

/// A late-bound, non-owning view of the tool registries a pack tool reads.
///
/// Late-bound because the pack tool is *inside* the registry it reads: the
/// vector cannot be built until it exists, and it cannot see it until it is
/// built. Non-owning because an `Arc` back into that same vector would be a
/// cycle that never drops.
///
/// **Two registries, not one, and that is load-bearing.** An agent's tools live
/// in two `Arc`s: the durable registry, and the `synthesized_tools` set that
/// `collect_orchestrator_tools` rebuilds whenever the Composio connection set
/// changes (they were split in #6145 so a reconcile cannot block on a reader).
/// Every `delegate_*` tool is in the second one. Binding only the first is why
/// `use_skill` could not reach a single packed delegate — `do_crypto`,
/// `run_skill`, `build_workflow` and four more were withheld from the wire and
/// then unreachable through the route that was supposed to replace them, which
/// is strictly worse than not packing them at all.
#[derive(Clone, Default)]
pub struct PackRegistryHandle {
    inner: Arc<RwLock<Slots>>,
}

/// The two registries, each independently rebindable.
///
/// They are separate slots rather than one vector because they are replaced on
/// different schedules: the durable registry is rebuilt when the agent is, the
/// synthesised one on every delegation refresh.
#[derive(Default)]
struct Slots {
    durable: Option<ToolRegistryRef>,
    synthesized: Option<ToolRegistryRef>,
}

impl PackRegistryHandle {
    /// Point this handle at the durable registry it lives in, replacing any
    /// previous binding.
    ///
    /// Rebinding has to actually take effect. This was a `OnceLock` whose
    /// second write was dropped, which silently contradicted
    /// [`super::bind_pack_registry`]'s own instruction to "re-bind after any
    /// later rebuild of this `Arc`": once an agent replaced its tool vector the
    /// handle still pointed at the old allocation, the `Weak` failed to
    /// upgrade, and every `use_skill` call reported the registry as unavailable
    /// for the rest of the session. Last write wins.
    pub fn bind(&self, registry: ToolRegistryRef) {
        self.with_slots(|slots| slots.durable = Some(registry));
    }

    /// Point this handle at the synthesised delegate set.
    ///
    /// Call it again after **every** `refresh_delegation_tools`, which replaces
    /// that `Arc` wholesale — a stale `Weak` stops upgrading as soon as the last
    /// reader of the old allocation goes, and the packed delegates silently
    /// become unreachable.
    pub fn bind_synthesized(&self, registry: ToolRegistryRef) {
        self.with_slots(|slots| slots.synthesized = Some(registry));
    }

    fn with_slots(&self, edit: impl FnOnce(&mut Slots)) {
        match self.inner.write() {
            Ok(mut slots) => edit(&mut slots),
            // The lock is only ever held for a pointer read or write, so a
            // poisoned lock means a panic elsewhere. Recover rather than
            // propagate: a stale binding degrades to "skill unavailable",
            // which is the failure this rebinding exists to prevent.
            Err(poisoned) => edit(&mut poisoned.into_inner()),
        }
    }

    /// Every live registry, durable first.
    ///
    /// Order matters on a name collision: `drop_synthesized_name_collisions`
    /// gives the durable tool the name, so resolving durable-first is what
    /// makes this agree with what the harness would actually execute.
    fn registries(&self) -> Vec<ToolVec> {
        let slots = match self.inner.read() {
            Ok(slots) => slots,
            Err(poisoned) => poisoned.into_inner(),
        };
        [slots.durable.as_ref(), slots.synthesized.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .collect()
    }

    /// Resolve a packed tool by name, enforcing that it belongs to `skill`.
    ///
    /// The pack check is not decoration: without it `use_skill` would dispatch
    /// into any packed tool regardless of the skill named, and the model could
    /// reach a crypto write through a workflow skill.
    fn resolve(&self, skill: &str, tool: &str) -> Option<(ToolVec, usize)> {
        registry::pack(skill).filter(|p| p.owns(tool))?;
        self.find(tool)
    }

    /// Locate `tool` in whichever registry holds it.
    fn find(&self, tool: &str) -> Option<(ToolVec, usize)> {
        for tools in self.registries() {
            if let Some(idx) = tools.iter().position(|t| t.name() == tool) {
                return Some((tools, idx));
            }
        }
        None
    }
}

/// Render a pack's listing, showing only the tools `is_callable` admits.
///
/// **The filter is the whole point.** `load_skill` used to render every tool in
/// the pack this build compiled, and `use_skill` then refused any of them the
/// session's allowlist denies (`tinyagents::middleware::channel_permission_block`).
/// A non-owner was handed a menu it could not order from: the orchestrator
/// loaded `workflows`, read `propose_workflow` off the listing, called it, and
/// was told it "is not allowed in the current session". The denial named no
/// alternative, so the model retried — one live chat turn died on the
/// repeated-failure breaker after six identical denials.
///
/// `registry.rs` used to claim non-owners "reach them through `use_skill`".
/// That was never true: the gate (`d5a09ea81`, 2026-08-21) predates the comment
/// asserting it (`a8f0a002b`, 2026-08-23). The listing is the side that was
/// wrong, so the listing is the side that changed.
///
/// `route` is the sentence to append when the session can call nothing in the
/// pack — see [`route_sentence`]. Empty means "say nothing extra".
pub fn render_pack_filtered(
    skill: &str,
    handle: &PackRegistryHandle,
    is_callable: &dyn Fn(&str) -> bool,
    route: &str,
) -> Result<String, String> {
    let Some(pack) = registry::pack(skill) else {
        // Scoped too: offering a hallucinating model a pack it cannot use is the
        // same wrong turn the advertised index used to take, one error later.
        return Err(format!(
            "Unknown skill `{skill}`. Available:\n{}",
            registry::pack_index_markdown_filtered(is_callable)
        ));
    };
    if handle.registries().is_empty() {
        return Err(
            "The skill registry is not available in this session; the tools in this skill \
             cannot be loaded."
                .to_string(),
        );
    }

    let mut out = format!("# Skill `{}`\n\n{}\n\n", pack.id, pack.summary);
    out.push_str(&format!(
        "Call these with `use_skill {{ \"skill\": \"{}\", \"tool\": \"<name>\", \"args\": {{ … }} }}`. \
         `args` is the tool's own argument object, exactly as documented below.\n\n",
        pack.id
    ));

    let mut found = 0usize;
    for name in pack.tools {
        // A pack may name a tool this build compiled out (feature gate) or that
        // this agent never had. Rendering the ones that exist beats failing the
        // whole load.
        let Some((tools, idx)) = handle.find(name) else {
            continue;
        };
        // Listing a tool the gate will refuse is worse than omitting it: a
        // model cannot tell a policy denial from a transient failure, so it
        // retries the same call instead of routing around it.
        if !is_callable(name) {
            continue;
        }
        let tool = &tools[idx];
        found += 1;
        out.push_str(&format!(
            "## `{}`\n\n{}\n\n",
            tool.name(),
            tool.description()
        ));
        // Minified, matching what the provider receives for a natively
        // advertised tool. Pretty-printing costs roughly a third more tokens
        // for indentation and newlines the model gains nothing from, and this
        // text is charged to the context window exactly like a native schema.
        out.push_str("```json\n");
        out.push_str(
            &serde_json::to_string(&tool.parameters_schema()).unwrap_or_else(|_| "{}".to_string()),
        );
        out.push_str("\n```\n\n");
    }

    if found == 0 {
        let mut message = format!(
            "Skill `{}` has no tools available in this session.",
            pack.id
        );
        if !route.is_empty() {
            message.push(' ');
            message.push_str(route);
        }
        return Err(message);
    }
    Ok(out)
}

/// The "go here instead" sentence shared by the `use_skill` listing and the
/// `use_skill` denial, so a model never sees two different stories.
///
/// `callable_delegates` are delegation tool names the caller has already
/// confirmed this session can invoke — naming the *tool* rather than the agent
/// is the difference between guidance and an instruction, and a model left to
/// guess the call retries. When none can be reached the owning agents are named
/// instead: strictly worse, but still better than a bare denial.
pub fn route_sentence(callable_delegates: &[String], owners: &[&str]) -> String {
    if !callable_delegates.is_empty() {
        let names = callable_delegates
            .iter()
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>()
            .join(" or ");
        return format!(
            "Call {names} instead — that agent owns these tools and runs them directly. \
             Do not retry this skill."
        );
    }
    if owners.is_empty() {
        return String::new();
    }
    format!(
        "These tools belong to {}; hand the task to one of them rather than calling directly.",
        owners
            .iter()
            .map(|o| format!("`{o}`"))
            .collect::<Vec<_>>()
            .join(" or ")
    )
}

fn render_pack(skill: &str, handle: &PackRegistryHandle) -> Result<String, String> {
    render_pack_filtered(skill, handle, &|_| true, "")
}

/// Rewrite `use_skill`'s advertised spec to match what this session can do.
///
/// The description is built once in [`UseSkillTool::new`], before any session
/// exists, so every agent was told all ten packs were loadable — including ones
/// it can call nothing in. Post-#(routing fix) that costs one wasted round trip
/// instead of a dead turn; it should cost zero.
///
/// Both halves are rewritten, and the schema is the stronger one: narrowing the
/// `skill` enum makes an unusable pack *unrepresentable* rather than merely
/// discouraged in prose, and a shorter enum is fewer tokens, not more.
///
/// Returns `false` when this session can call nothing in any pack — the caller
/// should then drop `use_skill` from the wire entirely, because an empty index
/// and an empty enum are not a tool.
pub fn scope_use_skill_spec(spec: &mut ToolSpec, is_callable: &dyn Fn(&str) -> bool) -> bool {
    let ids = registry::callable_pack_ids(is_callable);
    if ids.is_empty() {
        return false;
    }
    if let Some(index) = spec.description.find("\n\nSkills:\n") {
        spec.description.truncate(index);
        spec.description.push_str("\n\nSkills:\n");
        spec.description
            .push_str(&registry::pack_index_markdown_filtered(is_callable));
    }
    if let Some(enum_slot) = spec
        .parameters
        .pointer_mut("/properties/skill/enum")
        .filter(|v| v.is_array())
    {
        *enum_slot = Value::Array(
            ids.iter()
                .map(|id| Value::String((*id).to_string()))
                .collect(),
        );
    }
    true
}

fn skill_enum() -> Vec<&'static str> {
    registry::PACKS.iter().map(|p| p.id).collect()
}

/// The tool named in `args`, if the caller named one at all.
///
/// An absent (or empty) `tool` is not a malformed call: it is the disclosure
/// half of this tool, and the distinction decides both which branch
/// [`UseSkillTool::execute_with_context`] takes and what permission level the
/// call is gated at. Public because the policy middleware has to draw the same
/// line — it intercepts the disclosure half to scope the listing to the session
/// and lets the execution half through to its gate — and two spellings of "did
/// the caller name a tool" would be two chances to disagree.
pub fn named_tool(args: &Value) -> Option<&str> {
    args.get("tool")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
}

/// The one always-on tool that stands in for every packed tool.
///
/// It is both halves of the pack seam. Called with a `skill` alone it renders
/// that pack's tool schemas into the conversation; called with a `skill` and a
/// `tool` it executes that tool. These were two tools — `load_skill` and
/// `use_skill` — until the pack index that each carried in its own description
/// made the pair spend 3.3 kB of every single turn saying one list twice, and
/// made the first call of any packed tool a mandatory two-call round trip.
/// A model that learned the retired name gets the harness's "unknown tool"
/// recovery result (#4249) and retries against the schema it can actually see,
/// so no alias is carried for it.
///
/// **Permission forwarding is load-bearing.** The harness gates a call on the
/// tool's `permission_level_with_args`, so a proxy reporting its own level would
/// launder every packed tool's risk down to this one's — a crypto write would
/// be admitted on a channel that refuses crypto writes. Both accessors resolve
/// the inner tool and defer to it; the arg-less one has nothing to resolve
/// from, so it reports the highest level any packed tool needs rather than
/// guessing low. A call that names no `tool` reads a schema and nothing else,
/// so that one branch is genuinely `ReadOnly`.
pub struct UseSkillTool {
    handle: PackRegistryHandle,
    description: String,
}

impl UseSkillTool {
    pub fn new(handle: PackRegistryHandle) -> Self {
        let description = format!(
            "Reach a skill's tools. Their names, descriptions and argument schemas are NOT in \
             your context until you ask for them: call this with `skill` alone to see them, then \
             again with `skill` + `tool` + `args` to run one.\n\nSkills:\n{}",
            registry::pack_index_markdown()
        );
        Self {
            handle,
            description,
        }
    }

    fn resolve(&self, args: &Value) -> Option<(ToolVec, usize)> {
        let skill = args.get("skill").and_then(Value::as_str)?;
        let tool = named_tool(args)?;
        self.handle.resolve(skill, tool)
    }
}

#[async_trait]
impl Tool for UseSkillTool {
    fn name(&self) -> &str {
        USE_SKILL
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string", "enum": skill_enum(), "description": "Skill to read or run a tool from." },
                "tool": { "type": "string", "description": "Tool to run. Omit to list the skill's tools and their arguments instead." },
                "args": {
                    "type": "object",
                    "description": "The tool's own arguments, as documented in the listing.",
                    "additionalProperties": true
                }
            },
            "required": ["skill"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, options, None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let Some(skill) = args.get("skill").and_then(Value::as_str) else {
            return Ok(ToolResult::error(format!(
                "`skill` is required.\n\nSkills:\n{}",
                registry::pack_index_markdown()
            )));
        };

        // Disclosure half: no tool named, so render the pack's schemas.
        let Some(name) = named_tool(&args) else {
            return Ok(match render_pack(skill, &self.handle) {
                Ok(text) => ToolResult::success(text),
                Err(message) => ToolResult::error(message),
            });
        };

        let Some((tools, idx)) = self.handle.resolve(skill, name) else {
            return Ok(ToolResult::error(format!(
                "No tool `{name}` in skill `{skill}`. Call `use_skill {{ \"skill\": \"{skill}\" }}` \
                 to see what it contains.\n\nSkills:\n{}",
                registry::pack_index_markdown()
            )));
        };
        let inner_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
        tracing::debug!(tool = tools[idx].name(), "[toolpacks] use_skill dispatch");
        tools[idx]
            .execute_with_context(inner_args, options, context)
            .await
    }

    fn supports_markdown(&self) -> bool {
        // The inner result is forwarded verbatim, markdown rendering included,
        // so advertise the capability rather than suppressing a real saving.
        true
    }

    fn external_effect_with_args(&self, args: &Value) -> bool {
        match self.resolve(args) {
            Some((tools, idx)) => {
                let inner_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
                tools[idx].external_effect_with_args(&inner_args)
            }
            None => false,
        }
    }

    fn timeout_policy(&self, args: &Value) -> crate::openhuman::tools::traits::ToolTimeout {
        match self.resolve(args) {
            Some((tools, idx)) => {
                let inner_args = args.get("args").cloned().unwrap_or_else(|| json!({}));
                tools[idx].timeout_policy(&inner_args)
            }
            None => crate::openhuman::tools::traits::ToolTimeout::Inherit,
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        let packed = registry::all_packed_tool_names();
        self.handle
            .registries()
            .iter()
            .flat_map(|tools| tools.iter())
            .filter(|t| packed.contains(&t.name()))
            .map(|t| t.permission_level())
            .max()
            // Unbound or empty: report the ceiling, never a permissive default.
            .unwrap_or(PermissionLevel::Dangerous)
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        // Naming no tool renders a schema and does nothing else. Reporting the
        // packed ceiling here would put an approval prompt in front of reading
        // a tool list, which is the round trip this tool exists to remove.
        if named_tool(args).is_none() {
            return PermissionLevel::ReadOnly;
        }
        match self.resolve(args) {
            Some((tools, idx)) => {
                let inner = args.get("args").cloned().unwrap_or_else(|| json!({}));
                tools[idx].permission_level_with_args(&inner)
            }
            // Unresolvable: the call will fail anyway, but report the ceiling so
            // a malformed call can never be admitted on a channel that would
            // have refused the real tool.
            None => self.permission_level(),
        }
    }

    /// The registry handle rides on the vocabulary's erased host extension:
    /// `PackRegistryHandle` is this host's concept, and `tinytools` has no
    /// business naming it. `traits::pack_registry_handle` reads it back.
    fn host_extension(&self) -> Option<&(dyn std::any::Any + Send + Sync)> {
        Some(&self.handle)
    }
}
