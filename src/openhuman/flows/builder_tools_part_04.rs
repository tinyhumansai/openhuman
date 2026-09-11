
/// Search the FULL LIVE Composio catalog and return a [`CatalogSearchOutcome`].
///
/// Primary pass: case-insensitive AND — an action matches only if EVERY
/// whitespace-separated term substring-matches its slug, toolkit name, or
/// description (curated matches ranked first, stable sort preserves fetch
/// order). When that yields zero rows for a MULTI-WORD query, a per-keyword OR
/// fallback runs: each action is scored by how many query tokens match its
/// slug/toolkit/description, and the top [`MAX_FALLBACK_RESULTS`] (ranked by
/// hit-count desc, then curated first) are returned with an advisory `note`.
/// This is what keeps a natural-language query like "twitter tweet replies
/// lookup" from returning a bare `count: 0` even though `TWITTER_*` actions
/// exist — the agent gets the nearest keyword matches instead of falsely
/// concluding the action is missing.
pub(crate) async fn search_catalog(
    config: &Config,
    query: &str,
    toolkit_filter: Option<&str>,
    limit: usize,
) -> CatalogSearchOutcome {
    use crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog;
    // Contract crate — same item the `memory::sync::composio::providers` shim
    // re-exported; see `ListConnectableToolkitsTool::execute` for why (#5560).
    use tinymemory_api::composio::agent_ready_toolkits;

    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();

    let toolkits: Vec<String> = match toolkit_filter {
        Some(tk) if !tk.trim().is_empty() => vec![tk.trim().to_ascii_lowercase()],
        _ => agent_ready_toolkits()
            .into_iter()
            .map(str::to_string)
            .collect(),
    };

    // Fetch every candidate toolkit's live catalog concurrently — a bare
    // keyword query (no `toolkit` filter) fans out across every agent-ready
    // toolkit, and fetching them one at a time would pay for each one's
    // round trip back-to-back (the per-toolkit cache only helps repeats).
    let fetched: Vec<(
        String,
        Option<Vec<crate::openhuman::flows::tinyflows::caps::ToolContract>>,
    )> = futures::future::join_all(toolkits.into_iter().map(|toolkit| async move {
        let catalog = fetch_live_toolkit_catalog(config, &toolkit).await;
        (toolkit, catalog)
    }))
    .await;

    // Drop toolkits whose fetch failed (no backend session / network error) —
    // they contribute zero results rather than erroring the whole search.
    let fetched: Vec<(
        String,
        Vec<crate::openhuman::flows::tinyflows::caps::ToolContract>,
    )> = fetched
        .into_iter()
        .filter_map(|(tk, catalog)| catalog.map(|c| (tk, c)))
        .collect();

    // Does the scanned scope hold ANY actions at all? Distinguishes "keyword
    // miss" (has actions, none matched) from "nothing to search" (empty scope).
    let any_actions = fetched.iter().any(|(_, catalog)| !catalog.is_empty());

    // ── Primary pass: case-insensitive AND across every term ──
    let mut matches: Vec<(bool, Value)> = Vec::new();
    for (toolkit, catalog) in &fetched {
        // WS3 — a toolkit that ships a curated catalog is a hard curated-only
        // allowlist at RUNTIME, so any `featured: false` action of it is
        // rejected on every real run. Compute once per toolkit and flag those
        // rows so the blocker is visible at search time (transcript failure #2).
        let toolkit_curated = ops::toolkit_has_curated_catalog(toolkit);
        for tool in catalog {
            let slug_lc = tool.slug.to_ascii_lowercase();
            let desc_lc = tool
                .description
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_match = terms.iter().all(|term| {
                slug_lc.contains(term) || toolkit.contains(term) || desc_lc.contains(term)
            });
            if !is_match {
                continue;
            }
            matches.push((
                tool.is_curated,
                shape_catalog_row(tool, toolkit, toolkit_curated),
            ));
        }
    }

    // Curated (`featured`) results first; stable sort preserves fetch order
    // within each group.
    matches.sort_by_key(|(is_curated, _)| std::cmp::Reverse(*is_curated));
    matches.truncate(limit);
    let primary: Vec<Value> = matches.into_iter().map(|(_, v)| v).collect();

    if !primary.is_empty() {
        return CatalogSearchOutcome {
            results: primary,
            fallback: false,
            note: None,
        };
    }

    // ── Zero primary hits ──
    // Single-token queries keep today's behavior exactly; only attach a light
    // advisory note so a lone keyword miss still explains the search is
    // keyword-based (task WS5.4, optional).
    if terms.len() <= 1 {
        let note = if any_actions {
            Some(format!(
                "No actions matched '{query}'. This search is keyword-based (matches action \
                 slug/name/description) — try a different single keyword (e.g. 'gmail' or \
                 'tweets')."
            ))
        } else {
            None
        };
        return CatalogSearchOutcome {
            results: Vec::new(),
            fallback: false,
            note,
        };
    }

    // ── Fallback pass (multi-word, zero primary hits): per-token OR scoring ──
    // Score each action by how many DISTINCT query tokens match its
    // slug/toolkit/description; keep the primary path's curated boost as the
    // tiebreak. Rows go through the SAME `shape_catalog_row` path as primary.
    let mut scored: Vec<(usize, bool, Value)> = Vec::new();
    for (toolkit, catalog) in &fetched {
        let toolkit_curated = ops::toolkit_has_curated_catalog(toolkit);
        for tool in catalog {
            let slug_lc = tool.slug.to_ascii_lowercase();
            let desc_lc = tool
                .description
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let hits = terms
                .iter()
                .filter(|term| {
                    slug_lc.contains(*term) || toolkit.contains(*term) || desc_lc.contains(*term)
                })
                .count();
            if hits == 0 {
                continue;
            }
            scored.push((
                hits,
                tool.is_curated,
                shape_catalog_row(tool, toolkit, toolkit_curated),
            ));
        }
    }

    // Most keyword hits first, then curated first; stable sort preserves fetch
    // order within a (hits, curated) group.
    scored.sort_by_key(|(hits, is_curated, _)| std::cmp::Reverse((*hits, *is_curated)));
    scored.truncate(limit.min(MAX_FALLBACK_RESULTS));
    let results: Vec<Value> = scored.into_iter().map(|(_, _, v)| v).collect();

    tracing::debug!(
        target: "flows",
        query,
        fallback = true,
        hits = results.len(),
        "[flows] search_tool_catalog: primary AND-match empty for a multi-word query — ran per-keyword OR fallback"
    );

    if results.is_empty() {
        // Literally zero tokens matched anything: no rows, but a note so the
        // agent doesn't read `count: 0` as "action doesn't exist" (task WS5.3).
        return CatalogSearchOutcome {
            results,
            fallback: true,
            note: Some(format!(
                "No actions matched any keyword in '{query}'. This search is keyword-based \
                 (matches action slug/name/description) — retry with a single keyword (e.g. one \
                 word like 'gmail' or 'tweets') for a full listing."
            )),
        };
    }

    CatalogSearchOutcome {
        results,
        fallback: true,
        note: Some(format!(
            "No exact match for '{query}'. Showing the nearest per-keyword matches — retry with a \
             single keyword (e.g. one word like 'gmail' or 'tweets') for a full listing."
        )),
    }
}

#[async_trait]
impl Tool for SearchToolCatalogTool {
    fn name(&self) -> &str {
        "search_tool_catalog"
    }

    fn description(&self) -> &str {
        "Search the FULL LIVE Composio catalog for REAL action slugs to use on `tool_call` \
         nodes — every action for a named app, whether or not the user has connected it yet \
         and whether or not it's one of OpenHuman's hand-curated actions. Read-only. Query by \
         keyword (e.g. 'send email', 'slack message'); optionally scope to one `toolkit` (e.g. \
         'gmail', or any Composio app name) to search that app specifically. Returns matching \
         { slug, toolkit, description, required_args, output_fields, primary_array_path, \
         featured } entries, curated (`featured: true`) matches ranked first. ALWAYS ground a \
         tool_call node's `slug` in a real result here — never invent one. Before wiring a \
         match's args or a downstream binding, call get_tool_contract { slug } for the FULL \
         contract (exact required_args, full input/output JSON Schema) — this search result is \
         enough to FIND the right slug, get_tool_contract is what grounds the WIRING. If the \
         app isn't connected yet, you can still build the node and use composio_connect (or \
         tell the user) — the flow will prompt for the connection at run time."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords to match against tool slugs/descriptions (case-insensitive). All terms must match for an exact hit; a multi-word query with no exact match falls back to the nearest per-keyword matches. For the widest listing, prefer ONE keyword (e.g. 'gmail' or 'tweets')."
                },
                "toolkit": {
                    "type": "string",
                    "description": "Optional toolkit/app slug to scope the search (e.g. 'gmail', 'slack', or any named Composio app — connected or not)."
                }
            },
            "required": ["query"],
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
        let query = match args.get("query").and_then(Value::as_str).map(str::trim) {
            Some(q) if !q.is_empty() => q.to_string(),
            _ => return Ok(ToolResult::error("Missing 'query' parameter".to_string())),
        };
        let toolkit = args.get("toolkit").and_then(Value::as_str);
        tracing::debug!(
            target: "flows",
            %query,
            toolkit = toolkit.unwrap_or("(any)"),
            "[flows] search_tool_catalog: searching the FULL LIVE Composio catalog (read-only)"
        );
        let outcome = search_catalog(&self.config, &query, toolkit, MAX_CATALOG_RESULTS).await;
        // Build with `note` first so an agent reading top-down sees the
        // near-miss / keyword-based advisory before the (possibly zero) rows.
        // `count` is always the number of returned rows, never a stand-in for
        // "no such action" — a fallback carries a non-zero count.
        let mut obj = serde_json::Map::new();
        if let Some(note) = outcome.note {
            obj.insert("note".to_string(), Value::String(note));
        }
        obj.insert("query".to_string(), Value::String(query));
        obj.insert(
            "count".to_string(),
            Value::Number(outcome.results.len().into()),
        );
        obj.insert("results".to_string(), Value::Array(outcome.results));
        Ok(ToolResult::success(serde_json::to_string_pretty(
            &Value::Object(obj),
        )?))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_tool_contract — read-only: the FULL live contract for one action slug
// ─────────────────────────────────────────────────────────────────────────────

/// `get_tool_contract`: fetch the FULL live [`ToolContract`](crate::openhuman::flows::tinyflows::caps::ToolContract)
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
        let Some(toolkit) = crate::openhuman::flows::ops::toolkit_for_contract_slug(&slug) else {
            return Ok(ToolResult::error(format!(
                "Could not extract a toolkit from slug '{slug}' — it must look like \
                 '<TOOLKIT>_<ACTION>' (e.g. 'GMAIL_SEND_EMAIL')."
            )));
        };

        tracing::debug!(
            target: "flows",
            %slug,
            %toolkit,
            "[flows] get_tool_contract: fetching the live contract (read-only)"
        );

        let Some(catalog) = crate::openhuman::flows::tinyflows::caps::fetch_live_toolkit_catalog(
            &self.config,
            &toolkit,
        )
        .await
        else {
            return Ok(ToolResult::error(format!(
                "Could not fetch the live Composio catalog for toolkit '{toolkit}' (no backend \
                 session, or a transient failure) — try again, or use search_tool_catalog to \
                 confirm the toolkit is reachable."
            )));
        };

        match catalog.iter().find(|c| c.slug.eq_ignore_ascii_case(&slug)) {
            Some(contract) => {
                // B12: a prior real-output probe (get_tool_output_sample) for
                // this exact slug is ACTUAL observed data and always wins
                // over the schema-derived hint — most relevant for an action
                // whose live listing publishes no output schema at all (e.g.
                // every GitHub action verified live as of this fix), where
                // `contract.primary_array_path` would otherwise be
                // permanently `None`.
                let contract = crate::openhuman::flows::tinyflows::caps::apply_probe_override(
                    contract.clone(),
                );

                // WS3 — EARLY runtime-gate warning (transcript failure #2): a
                // real-but-uncurated action of a toolkit that ships a curated
                // catalog is a hard curated-only allowlist at RUNTIME, so it is
                // REJECTED on every real run. The late `validate_workflow` gate
                // catches it, but only ~15 tool calls after the agent has built
                // and wired the node. Surface the blocker HERE, at contract-fetch
                // time (and first in the payload), so the agent never wires it.
                if !contract.is_curated && ops::toolkit_has_curated_catalog(&toolkit) {
                    tracing::debug!(
                        target: "flows",
                        %slug,
                        %toolkit,
                        "[flows] get_tool_contract: uncurated action of a curated toolkit — attaching runtime_gate warning"
                    );
                    #[derive(serde::Serialize)]
                    struct ContractWithRuntimeGate {
                        runtime_gate: &'static str,
                        #[serde(flatten)]
                        contract: crate::openhuman::flows::tinyflows::caps::ToolContract,
                    }
                    let payload = ContractWithRuntimeGate {
                        runtime_gate: "This action will be REJECTED on every real run — the \
                                       runtime tool gate only allows curated actions for this \
                                       toolkit. Pick a `featured: true` result from \
                                       search_tool_catalog instead.",
                        contract,
                    };
                    return Ok(ToolResult::success(serde_json::to_string_pretty(&payload)?));
                }

                Ok(ToolResult::success(serde_json::to_string_pretty(
                    &contract,
                )?))
            }
            None => Ok(ToolResult::error(format!(
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
/// [`crate::openhuman::flows::tinyflows::caps::apply_probe_override`]).
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
/// [`crate::openhuman::flows::tinyflows::caps::probe_tool_output_sample`]'s scope
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

        match crate::openhuman::flows::tinyflows::caps::probe_tool_output_sample(
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

// ─────────────────────────────────────────────────────────────────────────────
// list_agent_profiles — read-only: selectable agent kinds for an `agent` node
// ─────────────────────────────────────────────────────────────────────────────

/// `list_agent_profiles`: read-only listing of the agent **kinds** an `agent`
/// node can select via `agent_ref` (researcher, code_executor, crypto_agent, …).
///
/// Grounds the builder's `agent_ref` choice in real registry ids — the agent
/// analogue of `search_tool_catalog` for `tool_call` slugs — so it never
/// hallucinates an agent kind. Returns `{ id, name, description, model, tools,
/// tags }` for every enabled registered agent.
pub struct ListAgentProfilesTool;

impl ListAgentProfilesTool {
    /// Builds the tool (no configuration — reads the process-global registry).
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListAgentProfilesTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListAgentProfilesTool {
    fn name(&self) -> &str {
        "list_agent_profiles"
    }

    fn description(&self) -> &str {
        "List the agent KINDS an `agent` node can run via its `agent_ref` config \
         field (e.g. researcher, code_executor, crypto_agent). Read-only. Returns \
         a JSON array of { id, name, description, model, tools, tags }. Use this to \
         pick a real agent_ref — a coding step should reference the coding agent, a \
         research step the researcher — instead of guessing an id. Note: setting \
         agent_ref runs the step as a REAL agent turn (its own `run_single`), with \
         the selected specialist's full persona, model, tool loop, and iteration \
         cap — not just a persona-flavored completion. A plain `agent` node with \
         no agent_ref only gets the default LLM plus its own inline `tools` list; \
         it cannot run code, search the web, or use any specialist's tools."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    fn external_effect(&self) -> bool {
        false
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        tracing::debug!(target: "flows", "[flows] list_agent_profiles: listing registered agent kinds (read-only)");
        match crate::openhuman::agent::registry::list_agents(false).await {
            Ok(agents) => {
                let profiles: Vec<Value> = agents
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id,
                            "name": a.name,
                            "description": a.description,
                            "model": a.model,
                            "tools": a.tool_allowlist,
                            "tags": a.tags,
                        })
                    })
                    .collect();
                Ok(ToolResult::success(serde_json::to_string_pretty(
                    &json!({ "agent_profiles": profiles }),
                )?))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to list agent profiles: {e}"
            ))),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_node_kinds / get_node_kind_contract — queryable DSL schema (F2)
// ─────────────────────────────────────────────────────────────────────────────

/// `list_node_kinds`: enumerate the 14 tinyflows node kinds with a one-line
/// summary each. The DSL counterpart of `search_tool_catalog` for Composio
/// actions — a cheap first call to orient before fetching a full contract.
pub struct ListNodeKindsTool;

impl ListNodeKindsTool {
    /// Builds the tool (no configuration — the contracts are static).
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}
