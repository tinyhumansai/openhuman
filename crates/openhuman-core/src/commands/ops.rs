//! Business logic for `commands.list`: the fixed built-in table, plus
//! best-effort fetches of `skills.list` / `flows.list` through their own
//! registered controllers (rather than reaching into their private types)
//! so this module has no compile-time dependency on either domain's wire
//! shape — only on the JSON both already return over RPC.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::RegisteredController;
use crate::core::{unwrap_rpc, Outcome};

use super::types::{CommandEntry, CommandKind, CommandsListResponse};

/// Fixed slash commands the core itself understands. `(command, description)`;
/// the leading `/` is part of the wire `label`/`insert` text, not the `id`.
const BUILTINS: &[(&str, &str)] = &[
    ("/new", "Start a new conversation"),
    ("/clear", "Clear the current conversation"),
    ("/plan", "Switch to plan mode (draft without side effects)"),
    (
        "/build",
        "Switch to build mode (resume normal tool execution)",
    ),
    ("/goal", "Set or view this thread's goal"),
    ("/todo", "View or manage the thread's todo list"),
    ("/stop", "Stop the current run"),
];

fn builtin_entries() -> Vec<CommandEntry> {
    BUILTINS
        .iter()
        .map(|(command, description)| CommandEntry {
            id: command.trim_start_matches('/').to_string(),
            label: (*command).to_string(),
            description: (*description).to_string(),
            kind: CommandKind::Builtin,
            insert: Some((*command).to_string()),
        })
        .collect()
}

/// Calls the registered `{namespace}.{function}` controller directly —
/// bypassing JSON-RPC dispatch entirely, since this runs in-process — and
/// returns its unwrapped JSON on success. `None` on any failure (missing
/// controller, handler error): a broken skills/flows catalog must never
/// take down the whole command palette.
async fn invoke(
    ctx: &DispatchCtx,
    controllers: &[RegisteredController],
    namespace: &str,
    function: &str,
    params: Map<String, Value>,
) -> Option<Value> {
    let controller = controllers
        .iter()
        .find(|c| c.schema.namespace == namespace && c.schema.function == function)?;
    match (controller.handler)(ctx.clone(), params).await {
        Ok(value) => Some(unwrap_rpc(&value).clone()),
        Err(error) => {
            log::debug!(
                "[commands] {namespace}.{function} lookup failed, omitting from palette: {error}"
            );
            None
        }
    }
}

fn entries_from_array(value: &Value, array_field: &str, kind: CommandKind) -> Vec<CommandEntry> {
    let Some(items) = value.get(array_field).and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?.to_string();
            let label = item
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(&id)
                .to_string();
            let description = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some(CommandEntry {
                id,
                label,
                description,
                kind,
                // Skills/workflows dispatch through their own run RPC, not
                // by inserting text into the composer.
                insert: None,
            })
        })
        .collect()
}

/// Builds the merged command list: built-ins first (stable order, cheapest),
/// then skills, then workflows.
pub async fn commands_list(ctx: &DispatchCtx) -> Result<Outcome<CommandsListResponse>, String> {
    let mut entries = builtin_entries();

    let skills_controllers = crate::skills::all_skills_registered_controllers();
    let mut skills_params = Map::new();
    // Include capability skills (`skills/` roots), not just `workflows/`-root
    // automations — the palette wants everything runnable, not just the
    // Automations tab's default view.
    skills_params.insert("include_skills".to_string(), Value::Bool(true));
    if let Some(value) = invoke(ctx, &skills_controllers, "skills", "list", skills_params).await {
        entries.extend(entries_from_array(&value, "skills", CommandKind::Skill));
    }

    // Gated like the module it calls. `crate::flows` is `#[cfg(feature =
    // "flows")]`, so naming it unconditionally builds only for the feature
    // sets that happen to enable it -- and a consumer with
    // `default-features = false`, which is how `tinyhivemind` depends on this
    // crate, cannot compile it at all.
    #[cfg(feature = "flows")]
    {
        let flows_controllers = crate::flows::all_flows_registered_controllers();
        if let Some(value) = invoke(ctx, &flows_controllers, "flows", "list", Map::new()).await {
            entries.extend(entries_from_array(&value, "flows", CommandKind::Workflow));
        }
    }

    log::debug!(
        "[commands] list: {} builtin + {} total entries",
        BUILTINS.len(),
        entries.len()
    );
    Ok(Outcome::new(
        CommandsListResponse { commands: entries },
        Vec::new(),
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
