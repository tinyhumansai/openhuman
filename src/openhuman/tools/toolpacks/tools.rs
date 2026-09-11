//! The two always-on tools that stand in for every packed tool.

use std::sync::{Arc, RwLock, Weak};

use async_trait::async_trait;
use serde_json::{json, Value};

use super::registry;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCallOptions, ToolResult};
use tinytools::ToolRunContext;

pub const LOAD_SKILL: &str = "load_skill";
pub const USE_SKILL: &str = "use_skill";

/// An `Arc`-shared, owned view of the tool registry a pack tool lives in.
type ToolVec = Arc<Vec<Box<dyn Tool>>>;

/// A non-owning view of the tool registry, kept to break the binding cycle.
type ToolRegistryRef = Weak<Vec<Box<dyn Tool>>>;

/// A late-bound, non-owning view of the tool registry a pack tool lives in.
///
/// Late-bound because the pack tools are *inside* the registry they read: the
/// vector cannot be built until they exist, and they cannot see it until it is
/// built. Non-owning because an `Arc` back into that same vector would be a
/// cycle that never drops.
#[derive(Clone, Default)]
pub struct PackRegistryHandle {
    inner: Arc<RwLock<Option<ToolRegistryRef>>>,
}

impl PackRegistryHandle {
    /// Point this handle at the registry it lives in, replacing any previous
    /// binding.
    ///
    /// Rebinding has to actually take effect. This was a `OnceLock` whose
    /// second write was dropped, which silently contradicted
    /// [`super::bind_pack_registry`]'s own instruction to "re-bind after any
    /// later rebuild of this `Arc`": once an agent replaced its tool vector the
    /// handle still pointed at the old allocation, the `Weak` failed to
    /// upgrade, and every `load_skill` / `use_skill` call reported the registry
    /// as unavailable for the rest of the session. Last write wins.
    pub fn bind(&self, registry: ToolRegistryRef) {
        match self.inner.write() {
            Ok(mut slot) => *slot = Some(registry),
            // The lock is only ever held for a pointer read or write, so a
            // poisoned lock means a panic elsewhere. Recover rather than
            // propagate: a stale binding degrades to "skill unavailable",
            // which is the failure this rebinding exists to prevent.
            Err(poisoned) => *poisoned.into_inner() = Some(registry),
        }
    }

    fn tools(&self) -> Option<ToolVec> {
        let slot = match self.inner.read() {
            Ok(slot) => slot,
            Err(poisoned) => poisoned.into_inner(),
        };
        slot.as_ref()?.upgrade()
    }

    /// Resolve a packed tool by name, enforcing that it belongs to `skill`.
    ///
    /// The pack check is not decoration: without it `use_skill` would dispatch
    /// into any packed tool regardless of the skill named, and the model could
    /// reach a crypto write through a workflow skill.
    fn resolve(&self, skill: &str, tool: &str) -> Option<(ToolVec, usize)> {
        registry::pack(skill).filter(|p| p.owns(tool))?;
        let tools = self.tools()?;
        let idx = tools.iter().position(|t| t.name() == tool)?;
        Some((tools, idx))
    }
}

fn render_pack(skill: &str, handle: &PackRegistryHandle) -> Result<String, String> {
    let Some(pack) = registry::pack(skill) else {
        return Err(format!(
            "Unknown skill `{skill}`. Available:\n{}",
            registry::pack_index_markdown()
        ));
    };
    let Some(tools) = handle.tools() else {
        return Err(
            "The skill registry is not available in this session; the tools in this skill \
             cannot be loaded."
                .to_string(),
        );
    };

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
        let Some(tool) = tools.iter().find(|t| t.name() == *name) else {
            continue;
        };
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
        return Err(format!(
            "Skill `{}` has no tools available in this session.",
            pack.id
        ));
    }
    Ok(out)
}

fn skill_enum() -> Vec<&'static str> {
    registry::PACKS.iter().map(|p| p.id).collect()
}

/// Renders a pack's tool schemas into the conversation on demand.
pub struct LoadSkillTool {
    handle: PackRegistryHandle,
    description: String,
}

impl LoadSkillTool {
    pub fn new(handle: PackRegistryHandle) -> Self {
        let description = format!(
            "Load a skill's tools into this conversation. Their names, descriptions and argument \
             schemas are NOT in your context until you do. Call this before `use_skill` for any \
             skill you have not already loaded in this conversation.\n\nSkills:\n{}",
            registry::pack_index_markdown()
        );
        Self {
            handle,
            description,
        }
    }
}

#[async_trait]
impl Tool for LoadSkillTool {
    fn name(&self) -> &str {
        LOAD_SKILL
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string", "enum": skill_enum(), "description": "Skill to load." }
            },
            "required": ["skill"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let Some(skill) = args.get("skill").and_then(Value::as_str) else {
            return Ok(ToolResult::error("`skill` is required."));
        };
        Ok(match render_pack(skill, &self.handle) {
            Ok(text) => ToolResult::success(text),
            Err(message) => ToolResult::error(message),
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    /// The registry handle rides on the vocabulary's erased host extension:
    /// `PackRegistryHandle` is this host's concept, and `tinytools` has no
    /// business naming it. `traits::pack_registry_handle` reads it back.
    fn host_extension(&self) -> Option<&(dyn std::any::Any + Send + Sync)> {
        Some(&self.handle)
    }
}

/// Executes a tool belonging to a pack.
///
/// **Permission forwarding is load-bearing.** The harness gates a call on the
/// tool's `permission_level_with_args`, so a proxy reporting its own level would
/// launder every packed tool's risk down to this one's — a crypto write would
/// be admitted on a channel that refuses crypto writes. Both accessors resolve
/// the inner tool and defer to it; the arg-less one has nothing to resolve
/// from, so it reports the highest level any packed tool needs rather than
/// guessing low.
pub struct UseSkillTool {
    handle: PackRegistryHandle,
    description: String,
}

impl UseSkillTool {
    pub fn new(handle: PackRegistryHandle) -> Self {
        let description = format!(
            "Execute a tool belonging to a skill. Call `load_skill` first to see the skill's \
             tools and their arguments.\n\nSkills:\n{}",
            registry::pack_index_markdown()
        );
        Self {
            handle,
            description,
        }
    }

    fn resolve(&self, args: &Value) -> Option<(ToolVec, usize)> {
        let skill = args.get("skill").and_then(Value::as_str)?;
        let tool = args.get("tool").and_then(Value::as_str)?;
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
                "skill": { "type": "string", "enum": skill_enum(), "description": "Skill owning the tool." },
                "tool": { "type": "string", "description": "Tool name, as listed by `load_skill`." },
                "args": {
                    "type": "object",
                    "description": "The tool's own arguments, as documented by `load_skill`.",
                    "additionalProperties": true
                }
            },
            "required": ["skill", "tool"]
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
        let Some((tools, idx)) = self.resolve(&args) else {
            let skill = args.get("skill").and_then(Value::as_str).unwrap_or("");
            let name = args.get("tool").and_then(Value::as_str).unwrap_or("");
            return Ok(ToolResult::error(format!(
                "No tool `{name}` in skill `{skill}`. Call `load_skill {{ \"skill\": \"{skill}\" }}` \
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
        let Some(tools) = self.handle.tools() else {
            return PermissionLevel::Dangerous;
        };
        let packed = registry::all_packed_tool_names();
        tools
            .iter()
            .filter(|t| packed.contains(&t.name()))
            .map(|t| t.permission_level())
            .max()
            // Unbound or empty: report the ceiling, never a permissive default.
            .unwrap_or(PermissionLevel::Dangerous)
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
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
