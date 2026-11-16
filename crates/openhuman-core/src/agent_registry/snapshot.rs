//! The `agent.registry_snapshot` controller: a read-only projection of the
//! tinyagents `CapabilityRegistry` inventory, plus the local-model overlay it
//! feeds into the unified catalog.
//!
//! The wire name stays `agent.registry_snapshot`; `agent_runs::schemas` lists
//! it in the `agent` namespace's catalogue order and calls in here for the
//! schema and handler.

use crate::core::all::ControllerFuture;
use crate::core::runtime::DispatchCtx;
use crate::core::ControllerSchema;
use crate::util::params::json_output;
use serde_json::{Map, Value};

pub(crate) fn registry_snapshot_schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "agent",
        function: "registry_snapshot",
        description: "Project the tinyagents CapabilityRegistry inventory into a durable, \
                      read-only snapshot: every reachable model / tool / graph / agent \
                      component with its ComponentMetadata (id, kind, description, tags, \
                      aliases), plus a Graphviz DOT rendering. Assembled from the sources \
                      reachable outside an in-flight turn (static model catalog, baseline \
                      tool registry, graph topologies, built-in agents); live per-run \
                      tool/model handles are not included. Metadata only — never run state.",
        inputs: vec![],
        outputs: vec![json_output(
            "components",
            "Array of ComponentMetadata (id/kind/description/tags/aliases). Companion \
             fields: `counts` (per-kind totals), `dot` (Graphviz DOT), `deferred` (kinds \
             not fully projected outside a turn).",
        )],
    }
}

/// Project the user's model registry into runtime-discovered local models for
/// the unified catalog overlay.
///
/// A registry entry is treated as a local runtime model when its `provider`
/// parses as a [`LocalProviderKind`](tinyinference_local::profile::LocalProviderKind) (`ollama`, `lmstudio`, `mlx`, …). The
/// context window comes from the entry when set, else the runtime profile's
/// default; tool-calling/streaming flags come from the static provider profile
/// (the local runtimes do not advertise per-model capability here). Non-local
/// (cloud/BYOK) rows are skipped — those are owned by the priced catalog layers.
fn local_catalog_models_from_config(
    config: &crate::config::Config,
) -> Vec<crate::platform::cost::catalog::LocalCatalogModel> {
    use tinyinference_local::profile::{profile_for_kind, LocalProviderKind, ToolSupport};

    config
        .model_registry
        .iter()
        .filter_map(|entry| {
            let kind = LocalProviderKind::from_str_loose(&entry.provider)?;
            let profile = profile_for_kind(kind);
            let context_window = if entry.context_window > 0 {
                Some(u64::from(entry.context_window))
            } else {
                profile.default_context_window
            };
            Some(crate::platform::cost::catalog::LocalCatalogModel {
                provider: entry.provider.clone(),
                model_id: entry.id.clone(),
                context_window,
                tool_calling: matches!(profile.tool_support, ToolSupport::Native),
                streaming: profile.supports_streaming,
            })
        })
        .collect()
}

/// Read-only projection of the tinyagents `CapabilityRegistry` inventory.
///
/// The per-turn registry (`tinyagents::assemble_turn_harness`) owns live model
/// and tool handles that only exist while a run is in flight and cannot be
/// reached from a standalone RPC. So here we re-project the **same durable
/// descriptor sources** that ARE reachable outside a turn into a
/// [`RegistrySnapshot`](tinyagents_registry::RegistrySnapshot), entirely from metadata (no live handles):
///
/// * **Model** — the static cost catalog projection
///   (`cost::catalog::tinyagents_catalog_snapshot`); carries model id, aliases,
///   provider/mode tags.
/// * **Tool** — the baseline tool registry (`tools::ops::default_tools`); names +
///   descriptions. NOTE: the *full* per-agent tool surface (`tools::ops::all_tools`)
///   needs config/memory/audit/action-dir wiring that only exists inside a turn,
///   so only the baseline set is projected here. Deferred — see `deferred` in
///   the response and the migration follow-up.
/// * **Graph** — the surviving host graph descriptors registered as
///   `ComponentKind::Graph` components.
/// * **Agent** — the built-in agent archetypes
///   (`agent_registry::agents::load_builtins()`); id + `when_to_use` blurb.
///
/// Other kinds (Router/Reducer/Store/Middleware/Checkpointer/TaskStore/Listener)
/// are wired per-run and are not enumerable outside a turn; they are reported in
/// `deferred`.
pub(crate) fn handle_registry_snapshot(
    ctx: DispatchCtx,
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        use tinyagents_registry::{ComponentKind, ComponentMetadata, RegistrySnapshot};

        let mut components: Vec<ComponentMetadata> = Vec::new();

        // ── Models: unified catalog (crate seed + OpenHuman overlay + local) ─
        // Derive runtime-discovered local models from the user's model registry:
        // any entry whose provider is a local runtime (ollama, lmstudio, mlx, …)
        // is overlaid as a free, runtime-profiled catalog entry so local models
        // appear in the one projection alongside priced vendor rows.
        let local_models = match ctx.load_config().await {
            Ok(config) => local_catalog_models_from_config(&config),
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    "[registry][rpc][agent] config unavailable; unified catalog omits local models"
                );
                Vec::new()
            }
        };
        let catalog = crate::platform::cost::catalog::unified_model_catalog(&local_models);
        let model_count = catalog.models.len();
        for entry in catalog.models {
            let mut meta = ComponentMetadata::new(entry.model_id.clone(), ComponentKind::Model)
                .with_description(format!("{} · {}", entry.provider, entry.mode))
                .with_tag(entry.provider.clone())
                .with_tag(entry.mode.clone());
            meta.aliases = entry.aliases;
            components.push(meta);
        }

        // ── Tools: baseline registry (full per-agent surface deferred) ──────
        let security = std::sync::Arc::new(crate::security::SecurityPolicy::default());
        let baseline_tools = crate::tools::ops::default_tools(security);
        let tool_count = baseline_tools.len();
        for tool in &baseline_tools {
            components.push(
                ComponentMetadata::new(tool.name().to_string(), ComponentKind::Tool)
                    .with_description(tool.description().to_string()),
            );
        }

        // ── Agents: built-in archetypes ─────────────────────────────────────
        let agent_defs = crate::agent_registry::agents::load_builtins()
            .map_err(|e| format!("loading built-in agents for registry snapshot: {e}"))?;
        let agent_count = agent_defs.len();
        for def in &agent_defs {
            let mut meta = ComponentMetadata::new(def.id.clone(), ComponentKind::Agent)
                .with_description(def.when_to_use.clone());
            if let Some(display) = &def.display_name {
                meta = meta.with_tag(display.clone());
            }
            components.push(meta);
        }

        // Sort by (kind, id) for stable, diff-friendly output — mirrors the
        // ordering `CapabilityRegistry::snapshot()` produces.
        components.sort_by(|a, b| (a.kind, a.id.0.as_str()).cmp(&(b.kind, b.id.0.as_str())));

        let snapshot = RegistrySnapshot {
            components,
            ..Default::default()
        };
        let dot = snapshot.to_dot();

        tracing::debug!(
            models = model_count,
            tools = tool_count,
            agents = agent_count,
            total = snapshot.len(),
            "[registry][rpc][agent] registry_snapshot export"
        );

        Ok(serde_json::json!({
            "components": snapshot.components,
            "counts": {
                "model": model_count,
                "tool": tool_count,
                "agent": agent_count,
                "total": snapshot.len(),
            },
            "dot": dot,
            // Kinds not fully enumerable outside an in-flight turn, so callers
            // know the snapshot is a reachable subset rather than the whole
            // per-run registry.
            "deferred": {
                "tool_full_surface": "only baseline tools projected; per-agent all_tools \
                    requires turn-scoped config/memory/audit state",
                "kinds": ["router", "reducer", "store", "middleware", "checkpointer",
                          "task_store", "listener"],
            },
        }))
    })
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
