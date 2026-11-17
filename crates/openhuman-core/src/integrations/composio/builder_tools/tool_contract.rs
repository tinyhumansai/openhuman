//! `get_tool_contract` and the B12 `get_tool_output_sample` read-only probe.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::Config;
use tinytools::{PermissionLevel, Tool, ToolResult};

// ─────────────────────────────────────────────────────────────────────────────
// get_tool_contract — read-only: the FULL live contract for one action slug
// ─────────────────────────────────────────────────────────────────────────────

/// `get_tool_contract`: fetch the FULL live [`ToolContract`](crate::integrations::composio::catalog::ToolContract)
/// for one Composio action slug — the grounding step the builder MUST take
/// before wiring a `search_tool_catalog` match's args or a downstream
/// binding/`split_out.path` off it. Where `search_tool_catalog` is for
/// FINDING a real slug, this is for WIRING it correctly: exact
/// `required_args` (wire every one), the full `input_schema`/`output_schema`,
/// and `primary_array_path` (prefixed `json.` for a `split_out.path`).
pub struct GetToolContractTool {
    config: Arc<Config>,
}

impl GetToolContractTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for GetToolContractTool {
    fn name(&self) -> &str {
        "get_tool_contract"
    }

    fn description(&self) -> &str {
        "Fetch the FULL live contract for one Composio action slug (found via \
         search_tool_catalog) before wiring it into a tool_call node. Read-only. Returns { \
         slug, toolkit, description, required_args, input_schema, output_fields, \
         output_schema, primary_array_path, is_curated }. Use `required_args` for EVERY arg \
         you must wire in config.args; use `output_fields` for a downstream \
         `=nodes.<id>.item.json.data.<field>` binding — note the `data.` segment: a Composio \
         tool_call's real runtime output wraps its payload in `data` \
         (`ComposioExecuteResponse`), so `output_fields` names fields INSIDE that wrapper, not \
         top-level envelope keys — never guess a field name, and never drop the `data.` \
         segment (`.item.json.<field>` with no `data.` resolves null even when `<field>` is a \
         real output field). Use `primary_array_path` (prefixed with `json.`, e.g. \
         \"json.data.messages\" — the `data.` segment is already baked into the value) verbatim \
         as a downstream split_out.path when you need to fan out over this action's result \
         list. Call this for every real slug right before you wire its args — \
         search_tool_catalog's summary is enough to find the slug, this is what grounds the \
         wiring."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "The exact Composio action slug, e.g. 'GMAIL_SEND_EMAIL' (from search_tool_catalog)."
                }
            },
            "required": ["slug"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let slug = match args.get("slug").and_then(Value::as_str).map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return Ok(ToolResult::error("Missing 'slug' parameter".to_string())),
        };
        // Shape-check before any I/O, through the same guard the
        // `flows_get_tool_contract` RPC uses — `toolkit_from_slug` falls back to
        // the whole string when there is no `_`, so a bare action name would
        // otherwise resolve to a bogus toolkit and come back as the catalog
        // error below, which tells the model to retry a fetch that can never
        // succeed.
        match crate::integrations::composio::catalog::lookup_contract(&self.config, &slug).await {
            Ok((contract, runtime_gate)) => {
                // B12: a prior real-output probe (get_tool_output_sample) for
                // this exact slug is ACTUAL observed data and always wins
                // over the schema-derived hint — most relevant for an action
                // whose live listing publishes no output schema at all (e.g.
                // every GitHub action verified live as of this fix), where
                // `contract.primary_array_path` would otherwise be
                // permanently `None`.
                let contract = contract.clone();

                if let Some(runtime_gate) = runtime_gate {
                    tracing::debug!(
                        target: "flows",
                        %slug,
                        toolkit = %contract.toolkit,
                        "[flows] get_tool_contract: uncurated action of a curated toolkit — attaching runtime_gate warning"
                    );
                    #[derive(serde::Serialize)]
                    struct ContractWithRuntimeGate {
                        runtime_gate: &'static str,
                        #[serde(flatten)]
                        contract: crate::integrations::composio::catalog::ToolContract,
                    }
                    let payload = ContractWithRuntimeGate {
                        runtime_gate,
                        contract,
                    };
                    return Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?));
                }

                Ok(ToolResult::success(serde_json::to_string_pretty(
                    &contract,
                )?))
            }
            Err(crate::integrations::composio::catalog::ContractLookupError::InvalidSlug {
                slug,
            }) => Ok(ToolResult::error(format!(
                "Could not extract a toolkit from slug '{slug}' — it must look like \
                 '<TOOLKIT>_<ACTION>' (e.g. 'GMAIL_SEND_EMAIL')."
            ))),
            Err(
                crate::integrations::composio::catalog::ContractLookupError::CatalogUnavailable {
                    toolkit,
                },
            ) => Ok(ToolResult::error(format!(
                "Could not fetch the live Composio catalog for toolkit '{toolkit}' (no backend \
                     session, or a transient failure) — try again, or use search_tool_catalog to \
                     confirm the toolkit is reachable."
            ))),
            Err(crate::integrations::composio::catalog::ContractLookupError::ActionNotFound {
                slug,
                toolkit,
            }) => Ok(ToolResult::error(format!(
                "'{slug}' is not a real action in the '{toolkit}' toolkit's live catalog — use \
                     search_tool_catalog to find a real slug."
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_tool_output_sample — READ-ONLY real Composio call: the B12 output probe
// ─────────────────────────────────────────────────────────────────────────────

/// `get_tool_output_sample`: make ONE bounded, READ-ONLY, REAL Composio call
/// for `slug` and derive its `primary_array_path`/`output_fields` from the
/// ACTUAL response, overriding `get_tool_contract`'s schema-derived hint for
/// this slug from then on (see
/// the module's probe overlay).
///
/// **Exists because a schema-derived hint sometimes doesn't exist at all**:
/// Composio's live listing genuinely omits `output_parameters` for some
/// actions — verified live for every GitHub action, including the curated
/// `GITHUB_LIST_REPOSITORY_ISSUES` — leaving `get_tool_contract`'s
/// `primary_array_path` permanently `null`. Without ground truth the builder
/// has been observed guessing the whole-payload `"json.data"` as a
/// `split_out.path` (live flow "funny reminders v2": one item — the
/// `{issues:[...]}` container itself — instead of the real per-item list),
/// silently degrading a fan-out to a single item.
///
/// **This is a deliberate, narrow carve-out of the workflow-builder agent's
/// "propose/read only, no composio_execute" invariant** (see this module's
/// top doc): unlike `composio_execute`, this tool can ONLY ever perform a
/// `Read`-scope action (gated by
/// [`crate::integrations::composio::catalog::probe_tool_output_sample`]'s scope
/// check, which ignores the user's per-toolkit scope preference — a probe
/// must never perform a real mutation no matter what the user has toggled
/// on) against a toolkit the user has ALREADY connected. No message is sent,
/// no record created/updated/deleted, ever.
///
/// Pass the SAME `args` you intend to wire into the real `tool_call` node —
/// this samples THAT call, not a generic fixture. Omit `args` (or pass `{}`)
/// for a zero-required-arg action.
pub struct GetToolOutputSampleTool {
    config: Arc<Config>,
}

impl GetToolOutputSampleTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for GetToolOutputSampleTool {
    fn name(&self) -> &str {
        "get_tool_output_sample"
    }

    fn description(&self) -> &str {
        "Make ONE bounded, READ-ONLY, REAL call to a Composio action and derive its real \
         `primary_array_path`/`output_fields` from the ACTUAL response — use this when \
         get_tool_contract returns `output_schema: null` / `primary_array_path: null` for a \
         source tool you plan to `split_out` (e.g. every GitHub action, verified live), so a \
         downstream split_out.path never fans out over the whole-payload container by mistake. \
         Only ever performs a Read action (refuses Write/Admin actions unconditionally, \
         regardless of the user's scope preference) against an ALREADY-CONNECTED toolkit — never \
         sends, creates, updates, or deletes anything. Pass the SAME args you intend to wire into \
         the real tool_call node — this samples THAT exact call. Call get_tool_contract again \
         afterward (or trust this tool's own `primary_array_path`/`output_fields`) to see the \
         override applied. Real actions only, not `oh:` or `=`-derived slugs."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "slug": {
                    "type": "string",
                    "description": "The exact Composio action slug, e.g. 'GITHUB_LIST_REPOSITORY_ISSUES'."
                },
                "args": {
                    "type": "object",
                    "description": "Arguments for the real call — the SAME ones you intend to wire into the tool_call node (e.g. {\"owner\": \"acme\", \"repo\": \"widgets\"}). Omit for a zero-required-arg action."
                }
            },
            "required": ["slug"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    // T-m8: this DOES perform a real outbound Composio network call (see the
    // struct doc's B12 carve-out) despite declaring `external_effect() ==
    // false` — that is deliberate, not an oversight, and it never parks for
    // approval as a result. `external_effect` gates on WORLD-MUTATING
    // effects (a message sent, a record created/updated/deleted) that the
    // approval system exists to keep a human in the loop for; a probe here
    // is hard-restricted, independent of the approval gate, to Read-scope
    // actions only (`probe_tool_output_sample`'s own scope check, which
    // ignores the user's toggled write/admin scope preference) against a
    // toolkit the user has ALREADY connected — so there is nothing for a
    // human to approve: no side effect this call could possibly produce is
    // one the user hasn't already consented to by connecting the toolkit.
    // "Real network call" and "external_effect" are answering different
    // questions here on purpose.
    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let slug = match args.get("slug").and_then(Value::as_str).map(str::trim) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return Ok(ToolResult::error("Missing 'slug' parameter".to_string())),
        };
        let call_args = args.get("args").cloned().unwrap_or(json!({}));

        tracing::debug!(
            target: "flows",
            %slug,
            "[flows] get_tool_output_sample: tool invoked"
        );

        match crate::integrations::composio::catalog::probe_tool_output_sample(
            &self.config,
            &slug,
            call_args,
        )
        .await
        {
            Ok(sample) => {
                let primary_array_path_for_split_out = sample
                    .primary_array_path
                    .as_ref()
                    .map(|p| format!("json.{p}"));
                Ok(ToolResult::success(serde_json::to_string_pretty(&json!({
                    "slug": slug,
                    "primary_array_path": sample.primary_array_path,
                    "split_out_path": primary_array_path_for_split_out,
                    "output_fields": sample.output_fields,
                }))?))
            }
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}
