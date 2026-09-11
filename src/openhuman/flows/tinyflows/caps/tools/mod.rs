//! Tool dispatch for a `tool_call` node, split by slug namespace.
//!
//! A `tool_call` slug belongs to exactly one backend, chosen by prefix:
//! `oh:` names a native OpenHuman tool, anything else is a Composio action.
//! Before this module the two lived in one `impl ToolInvoker` with an
//! `if let Some(name) = slug.strip_prefix("oh:")` early return and the Composio
//! path as the implicit fallthrough, which meant "which namespaces exist" was
//! not answerable without reading the whole body.
//!
//! # Why backends are stateless
//!
//! Each backend is a unit struct, and the per-call state it needs
//! ([`Config`], [`SecurityPolicy`]) arrives as [`ToolCallCtx`]. That keeps the
//! registry a `&'static` slice with no construction cost, and — the reason it
//! matters here — it lets [`preflight`](ToolBackend::preflight) be called with
//! only a `Config`, by a caller that has no `SecurityPolicy` to give. The
//! dry-run preflight invoker is exactly that caller.
//!
//! # Ordering is load-bearing
//!
//! [`BACKENDS`] is ordered, and the Composio backend declares an **empty
//! prefix**, so it claims any slug no earlier backend did. It must therefore
//! stay last. A prefixed backend added after it would be unreachable.

mod composio;
mod native;

use async_trait::async_trait;
use serde_json::Value;
use tinyflows::error::{EngineError, Result};

use crate::openhuman::config::Config;
use crate::openhuman::security::SecurityPolicy;

pub(crate) use composio::ComposioToolBackend;
pub(crate) use native::NativeToolBackend;

/// Prefix marking a `tool_call` node's slug as a NATIVE OpenHuman tool (the
/// "Tool" node) rather than a Composio action (the "App action" node). e.g.
/// `oh:web_search`. Native tools run through the same agent tool registry the
/// assistant uses (`runtime_node::ops::execute_tool`), so a flow can call
/// search / media generation / file / shell / etc. — the full toolset.
pub(crate) const NATIVE_TOOL_PREFIX: &str = "oh:";

/// What a backend needs to service one call.
///
/// Borrowed rather than owned so dispatch clones nothing per call.
pub(crate) struct ToolCallCtx<'a> {
    /// Host config — credentials, entity id, composio mode.
    pub config: &'a Config,
    /// The autonomy tier / approval policy in force for this run.
    pub security: &'a SecurityPolicy,
}

/// One `tool_call` slug namespace.
///
/// Implementors own everything about their namespace: how a slug is validated,
/// which gates run and in what order, and how the call is dispatched. Nothing
/// about a namespace should be knowable from outside its backend — that is what
/// makes a namespace removable (a build without Composio drops its backend and
/// the dispatcher simply stops claiming those slugs).
#[async_trait]
pub(crate) trait ToolBackend: Send + Sync {
    /// Stable name, for the error a slug gets when nothing claims it.
    fn name(&self) -> &'static str;

    /// The slug prefix this backend claims. An **empty** prefix claims
    /// everything, so a backend returning `""` must be registered last.
    fn prefix(&self) -> &'static str;

    /// Whether this backend handles `slug`.
    fn claims(&self, slug: &str) -> bool {
        self.prefix().is_empty() || slug.starts_with(self.prefix())
    }

    /// Validate the call's arguments without performing it.
    ///
    /// Runs in the dry-run path, where the real invocation is replaced by an
    /// echo mock that would happily accept a `null` required arg. Defaults to
    /// accepting everything: native tools have no published argument schema,
    /// so they inherit this no-op instead of inventing a validation contract
    /// their real execution path does not enforce.
    ///
    /// Takes `config` rather than a full [`ToolCallCtx`] because a preflight
    /// makes no outbound call and so needs no security decision — which is what
    /// lets the dry-run invoker, holding only a `Config`, run it.
    async fn preflight(&self, config: &Config, slug: &str, args: &Value) -> Result<()> {
        let _ = (config, slug, args);
        Ok(())
    }

    /// Perform the call, applying this namespace's own gates first.
    async fn invoke(
        &self,
        ctx: &ToolCallCtx<'_>,
        slug: &str,
        args: Value,
        conn: Option<&str>,
    ) -> Result<Value>;
}

/// Every registered backend, in claim order. **Composio must stay last** — it
/// declares an empty prefix and so claims whatever is left.
pub(crate) const BACKENDS: &[&dyn ToolBackend] = &[&NativeToolBackend, &ComposioToolBackend];

/// The backend that claims `slug`, or `None` when no namespace matches.
pub(crate) fn backend_for(slug: &str) -> Option<&'static dyn ToolBackend> {
    BACKENDS.iter().copied().find(|b| b.claims(slug))
}

/// Error for a slug no backend claims, naming what *is* registered.
///
/// Only reachable if every backend declares a non-empty prefix — with the
/// Composio backend compiled in there is always a catch-all. It exists so a
/// build that drops the catch-all fails informatively instead of silently
/// having no tool surface.
pub(crate) fn unclaimed_slug_error(slug: &str) -> EngineError {
    let names: Vec<&str> = BACKENDS.iter().map(|b| b.name()).collect();
    EngineError::Capability(format!(
        "tool_call node: no tool backend handles slug `{slug}` (registered: {})",
        names.join(", ")
    ))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
