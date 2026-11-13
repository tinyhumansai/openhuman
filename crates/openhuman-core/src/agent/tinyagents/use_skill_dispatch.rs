//! Typed dispatch for `use_skill` (regression R3).
//!
//! `use_skill` is registered as a plain canonical tool, so a call reaching a
//! packed archetype delegation (`create_image`, `do_crypto`,
//! `make_presentation`, …) through it used to run through
//! `tinytools::Tool::execute_with_context`, which has no live parent
//! `RunContext`. `dispatch_subagent_with_live_parent` refuses to run without
//! one — "delegation requires a live harness run context." — so every packed
//! delegate was reachable only when a model happened to call it under its
//! bare name, and `PackedToolRouteMiddleware` actively rewrites bare packed
//! calls INTO `use_skill`, making the bug unconditional for any tool this
//! build packs.
//!
//! [`UseSkillDispatch`] closes the gap: it resolves the inner tool exactly as
//! [`crate::tools::toolpacks::tools::UseSkillTool::execute_with_context`]
//! does, then re-selects the same
//! [`super::tool_registration::typed_dispatch_for`] the harness would
//! have picked had the inner tool been natively advertised, and hands it the
//! REAL parent this dispatch itself received. The disclosure half (no `tool`
//! named), a missing `skill`, and a not-found tool are all delegated verbatim
//! to the wrapped `use_skill` adapter — those paths render `UseSkillTool`'s
//! existing schema listing / error text and need no live parent, so
//! duplicating that logic here would only create a second place for it to
//! drift.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyagents_harness::context::RunContext;
use tinyagents_harness::tool::{ToolDispatch, ToolExecutionContext};
use tinytools::{Tool, ToolCallOptions, ToolResult};

use super::host::OpenHumanRunContext;
use super::tool_registration::typed_dispatch_for;
use crate::tools::toolpacks::registry::CATALOG;
use tinyagents_harness::tool::packs::{named_tool, PackRegistryHandle};

/// Live-parent dispatch for the `use_skill` proxy tool.
///
/// `tool` is the `CanonicalSharedToolAdapter` the harness registered for
/// `use_skill` itself (used for the disclosure/error fallback paths and for
/// `tool()`); `handle` is the same pack-registry handle `use_skill`'s own
/// tool object carries, read off its erased host extension at registration
/// time.
pub(crate) struct UseSkillDispatch {
    tool: Arc<dyn Tool>,
    handle: PackRegistryHandle,
}

impl UseSkillDispatch {
    pub(crate) fn new(tool: Arc<dyn Tool>, handle: PackRegistryHandle) -> Self {
        Self { tool, handle }
    }
}

#[async_trait]
impl ToolDispatch<(), OpenHumanRunContext> for UseSkillDispatch {
    fn tool(&self) -> Arc<dyn Tool> {
        self.tool.clone()
    }

    fn bind_call(
        &self,
        arguments: &Value,
    ) -> anyhow::Result<Option<Arc<dyn ToolDispatch<(), OpenHumanRunContext>>>> {
        let requested = arguments
            .get("skill")
            .and_then(Value::as_str)
            .zip(named_tool(arguments));
        let Some((skill, name)) = requested else {
            // Disclosure and malformed calls stay on the outer adapter. They
            // cannot execute a packed target.
            return Ok(None);
        };

        let Some(tools) = self.handle.resolve_registry_for(&CATALOG, skill, name) else {
            let result = ToolResult::error(format!(
                "{} No tool `{name}` in skill `{skill}`. Call `use_skill {{ \"skill\": \"{skill}\" }}` to see what it contains.\n\nSkills:\n{}",
                CATALOG.not_found_marker(),
                CATALOG.pack_index_markdown()
            ));
            return Ok(Some(Arc::new(BoundUseSkillDispatch {
                outer: self.tool.clone(),
                inner: None,
                inner_arguments: arguments
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
                typed: None,
                answer: Some(result),
            })));
        };

        let inner_arguments = arguments
            .get("args")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let Some(inner_adapter) = super::tool_registration::shared_tool_adapter(vec![tools], name)
            .map(|adapter| Arc::new(adapter) as Arc<dyn Tool>)
        else {
            // Freeze the not-found result too: a target appearing after
            // authorization must not turn a rejected call into an execution.
            let marker = CATALOG.not_found_marker();
            let result =
                ToolResult::error(format!("{marker} No tool `{name}` in the requested skill."));
            return Ok(Some(Arc::new(BoundUseSkillDispatch {
                outer: self.tool.clone(),
                inner: None,
                inner_arguments,
                typed: None,
                answer: Some(result),
            })));
        };
        let typed = typed_dispatch_for(name, inner_adapter.clone());
        Ok(Some(Arc::new(BoundUseSkillDispatch {
            outer: self.tool.clone(),
            inner: Some(inner_adapter),
            inner_arguments,
            typed,
            answer: None,
        })))
    }

    async fn execute(
        &self,
        _state: &(),
        call_id: tinyagents_harness::CallId,
        arguments: Value,
        options: ToolCallOptions,
        parent: &RunContext<OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        // Resolve `skill` + `tool` against the pack registry exactly as
        // `UseSkillTool::execute_with_context` does. Anything that does not
        // resolve here — no `skill`, the disclosure half (no `tool` named),
        // or a name the pack does not own — is a path that tool already
        // renders correctly and that touches no live parent, so fall through
        // to it verbatim rather than re-deriving the same schema listing or
        // not-found message.
        let resolved = arguments
            .get("skill")
            .and_then(Value::as_str)
            .zip(named_tool(&arguments))
            .and_then(|(skill, name)| {
                self.handle
                    .resolve_registry_for(&CATALOG, skill, name)
                    .map(|tools| (name.to_string(), tools))
            });

        let Some((name, tools)) = resolved else {
            return self
                .tool
                .execute_with_context(arguments, options, None)
                .await;
        };

        let inner_args = arguments
            .get("args")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        // Re-wrap the resolved tool in the same `CanonicalSharedToolAdapter`
        // seam the harness itself builds at registration: typed-dispatch
        // selection keys off the tool's name and schema
        // (`DelegationDispatch::for_tool`), not object identity, so this
        // adapter is equivalent to the one that would have been registered
        // had the model reached `name` directly instead of through
        // `use_skill`.
        let Some(inner_adapter) = super::tool_registration::shared_tool_adapter(vec![tools], &name)
            .map(|adapter| Arc::new(adapter) as Arc<dyn Tool>)
        else {
            return self
                .tool
                .execute_with_context(arguments, options, None)
                .await;
        };

        if let Some(dispatch) = typed_dispatch_for(&name, inner_adapter.clone()) {
            return dispatch
                .execute(&(), call_id, inner_args, options, parent)
                .await;
        }

        // Not a typed-dispatch tool: run it the way `use_skill` always has,
        // through `Tool::execute_with_context`, but still hand it a real
        // `ToolExecutionContext` built from the live parent rather than
        // `None` — a non-recursive packed tool that reads call id, thread id
        // or workspace off the erased host extension gets the same facts a
        // native registration would have given it.
        let tool_context = ToolExecutionContext::from_run_context(parent, call_id);
        inner_adapter
            .execute_with_context(inner_args, options, Some(&tool_context))
            .await
    }
}

struct BoundUseSkillDispatch {
    outer: Arc<dyn Tool>,
    inner: Option<Arc<dyn Tool>>,
    inner_arguments: Value,
    typed: Option<Arc<dyn ToolDispatch<(), OpenHumanRunContext>>>,
    answer: Option<ToolResult>,
}

struct BoundUseSkillTool {
    outer: Arc<dyn Tool>,
    inner: Option<Arc<dyn Tool>>,
    inner_arguments: Value,
    answer: Option<ToolResult>,
}

#[async_trait]
impl Tool for BoundUseSkillTool {
    fn name(&self) -> &str {
        self.outer.name()
    }

    fn description(&self) -> &str {
        self.outer.description()
    }

    fn parameters_schema(&self) -> Value {
        self.outer.parameters_schema()
    }

    fn policy(&self) -> tinytools::ToolPolicy {
        self.inner
            .as_ref()
            .map_or_else(|| self.outer.policy(), |tool| tool.policy())
    }

    fn supports_markdown(&self) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|tool| tool.supports_markdown())
    }

    fn permission_level(&self) -> tinytools::PermissionLevel {
        self.inner.as_ref().map_or_else(
            || self.outer.permission_level(),
            |tool| tool.permission_level(),
        )
    }

    fn permission_level_with_args(&self, _arguments: &Value) -> tinytools::PermissionLevel {
        self.inner.as_ref().map_or_else(
            || self.outer.permission_level(),
            |tool| tool.permission_level_with_args(&self.inner_arguments),
        )
    }

    fn external_effect_with_args(&self, _arguments: &Value) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|tool| tool.external_effect_with_args(&self.inner_arguments))
    }

    fn timeout_policy(&self, _arguments: &Value) -> tinytools::ToolTimeout {
        self.inner
            .as_ref()
            .map_or(tinytools::ToolTimeout::Inherit, |tool| {
                tool.timeout_policy(&self.inner_arguments)
            })
    }

    async fn prepare_authorization(&self, _arguments: &Value) -> anyhow::Result<Option<Value>> {
        match &self.inner {
            Some(tool) => tool.prepare_authorization(&self.inner_arguments).await,
            None => Ok(None),
        }
    }

    async fn execute(&self, _arguments: Value) -> anyhow::Result<ToolResult> {
        if let Some(answer) = &self.answer {
            return Ok(answer.clone());
        }
        match &self.inner {
            Some(tool) => tool.execute(self.inner_arguments.clone()).await,
            None => self.outer.execute(Value::Null).await,
        }
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        options: ToolCallOptions,
        context: Option<&dyn tinytools::ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        if let Some(answer) = &self.answer {
            return Ok(answer.clone());
        }
        match &self.inner {
            Some(tool) => {
                tool.execute_with_context(self.inner_arguments.clone(), options, context)
                    .await
            }
            None => {
                self.outer
                    .execute_with_context(Value::Null, options, context)
                    .await
            }
        }
    }
}

#[async_trait]
impl ToolDispatch<(), OpenHumanRunContext> for BoundUseSkillDispatch {
    fn tool(&self) -> Arc<dyn Tool> {
        Arc::new(BoundUseSkillTool {
            outer: self.outer.clone(),
            inner: self.inner.clone(),
            inner_arguments: self.inner_arguments.clone(),
            answer: self.answer.clone(),
        })
    }

    fn output_origin(&self) -> tinyagents_harness::host::ContentOrigin {
        self.typed
            .as_ref()
            .map_or(tinyagents_harness::host::ContentOrigin::Tool, |dispatch| {
                dispatch.output_origin()
            })
    }

    fn admission_target(&self) -> Option<(&dyn Tool, &Value)> {
        Some((self.inner.as_deref()?, &self.inner_arguments))
    }

    async fn execute(
        &self,
        state: &(),
        call_id: tinyagents_harness::CallId,
        _arguments: Value,
        options: ToolCallOptions,
        parent: &RunContext<OpenHumanRunContext>,
    ) -> anyhow::Result<ToolResult> {
        if let Some(answer) = &self.answer {
            return Ok(answer.clone());
        }
        let Some(inner) = &self.inner else {
            return self
                .outer
                .execute_with_context(Value::Null, options, None)
                .await;
        };
        if let Some(dispatch) = &self.typed {
            return dispatch
                .execute(
                    state,
                    call_id,
                    self.inner_arguments.clone(),
                    options,
                    parent,
                )
                .await;
        }
        let context = ToolExecutionContext::from_run_context(parent, call_id);
        inner
            .execute_with_context(self.inner_arguments.clone(), options, Some(&context))
            .await
    }
}

#[cfg(test)]
#[path = "use_skill_dispatch_tests.rs"]
mod tests;
