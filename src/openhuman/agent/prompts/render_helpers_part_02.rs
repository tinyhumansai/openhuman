
/// Shared `## Project instructions (AGENTS.md)` block writer.
///
/// Used by both [`super::sections::AgentsInstructionsSection`] (the default /
/// sub-agent builder chains) and the narrow sub-agent renderer
/// ([`render_subagent_system_prompt_with_format`]) so the two paths never
/// drift. The heading is emitted only when at least one layer carries content;
/// the global layer renders first, then the local/project layer. Each layer is
/// injected via [`inject_inline_content`] under its own `###` sub-heading and
/// capped at [`BOOTSTRAP_MAX_CHARS`] with a `[... truncated]` marker.
///
/// Both inputs are already-loaded, pre-trimmed strings (see
/// [`super::agents_md::load_agents_md`]) — this writer does no file I/O, keeping
/// the rendered bytes a pure function of its inputs for KV-cache stability.
pub(crate) fn write_agents_md_blocks(out: &mut String, global: Option<&str>, local: Option<&str>) {
    let mut body = String::new();
    if let Some(g) = global {
        inject_inline_content(&mut body, "AGENTS.md (workspace)", g, BOOTSTRAP_MAX_CHARS);
    }
    if let Some(l) = local {
        inject_inline_content(&mut body, "AGENTS.md (project)", l, BOOTSTRAP_MAX_CHARS);
    }
    if body.trim().is_empty() {
        log::debug!("[agents_md] no AGENTS.md content to inject; skipping section");
        return;
    }
    log::debug!(
        "[agents_md] injecting AGENTS.md section (global={}, local={})",
        global.is_some(),
        local.is_some()
    );
    out.push_str("## Project instructions (AGENTS.md)\n\n");
    out.push_str(
        "Configurable standing instructions loaded from AGENTS.md files. Treat these as \
         durable guidance for how to operate here. The workspace layer applies globally; \
         the project layer applies to the current working directory and takes precedence \
         where the two conflict. They are background guidance, not messages in this \
         conversation.\n\n",
    );
    out.push_str(&body);
}

/// for the output header and truncation semantics.
///
/// Empty/whitespace content is silently skipped, mirroring the file
/// loader's "no noisy placeholder" behaviour.
pub fn inject_snapshot_content(prompt: &mut String, label: &str, content: &str, max_chars: usize) {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return;
    }
    let _ = writeln!(prompt, "### {label}\n");
    let truncated = if trimmed.chars().count() > max_chars {
        trimmed
            .char_indices()
            .nth(max_chars)
            .map(|(idx, _)| &trimmed[..idx])
            .unwrap_or(trimmed)
    } else {
        trimmed
    };
    prompt.push_str(truncated);
    if truncated.len() < trimmed.len() {
        let _ = writeln!(
            prompt,
            "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
        );
    } else {
        prompt.push_str("\n\n");
    }
}

/// Inject `filename` into `prompt` with an explicit character budget.
///
/// Used directly by callers that want a tighter cap than
/// [`BOOTSTRAP_MAX_CHARS`] — notably `PROFILE.md` and `MEMORY.md` which
/// are user-specific, potentially growing, and do not warrant a full
/// 20K-char budget (see [`USER_FILE_MAX_CHARS`]).
///
/// Missing / empty files are silently skipped so callers can inject
/// optional files unconditionally without emitting a noisy placeholder.
///
/// **KV-cache contract:** the output is a pure function of `filename`,
/// file bytes at call time, and `max_chars`. Callers must invoke this
/// once per session — re-reading mid-session breaks the inference
/// backend's automatic prefix cache. See the byte-stability note on
/// [`render_subagent_system_prompt`].
pub fn inject_workspace_file_capped(
    prompt: &mut String,
    workspace_dir: &Path,
    filename: &str,
    max_chars: usize,
) {
    let path = workspace_dir.join(filename);

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let _ = writeln!(prompt, "### {filename}\n");
            let truncated = if trimmed.chars().count() > max_chars {
                trimmed
                    .char_indices()
                    .nth(max_chars)
                    .map(|(idx, _)| &trimmed[..idx])
                    .unwrap_or(trimmed)
            } else {
                trimmed
            };
            prompt.push_str(truncated);
            if truncated.len() < trimmed.len() {
                let _ = writeln!(
                    prompt,
                    "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
                );
            } else {
                prompt.push_str("\n\n");
            }
        }
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound => {
                // Keep prompt focused: missing optional identity/bootstrap files should not
                // add noisy placeholders that dilute tool-calling instructions.
            }
            _ => {
                log::debug!("[prompt] failed to read {}: {e}", path.display());
            }
        },
    }
}

pub fn default_workspace_file_content(filename: &str) -> &'static str {
    // The bundled identity files live at `src/openhuman/agent/prompts/`
    // (owned by the `agent/` tree because they describe agent identity).
    // This module is under `src/openhuman/agent/context/`, so the relative path
    // walks up one level and back into `agent/prompts/`.
    match filename {
        "SOUL.md" => include_str!("SOUL.md"),
        "IDENTITY.md" => include_str!("IDENTITY.md"),
        // The user-facing agent's role brief and writing style, moved out of
        // the compiled `orchestrator/prompt.md` so both are tunable on disk
        // without a rebuild (#5701). Same sync-and-inject contract as
        // SOUL.md / IDENTITY.md: the bundled copy seeds the workspace, and a
        // user edit wins from the next session on.
        "ROLE.md" => include_str!("ROLE.md"),
        "STYLE.md" => include_str!("STYLE.md"),

        _ => "",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Build a throwaway `PromptContext` for sections whose `build` only
/// uses static/immutable inputs (currently just `SafetySection`). Keeps
/// the `render_safety()` free function from forcing callers to
/// manufacture a full context when they only need the static text.
fn empty_prompt_context_for_static_sections() -> PromptContext<'static> {
    static EMPTY_TOOLS: &[PromptTool<'static>] = &[];
    static EMPTY_WORKFLOWS: &[crate::openhuman::skills::Workflow] = &[];
    static EMPTY_INTEGRATIONS: &[ConnectedIntegration] = &[];
    // SAFETY: the &HashSet reference must outlive the returned context;
    // a leaked OnceLock-style allocation gives us a permanent 'static
    // anchor without adding runtime cost on the hot path.
    static EMPTY_VISIBLE: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    let visible = EMPTY_VISIBLE.get_or_init(std::collections::HashSet::new);
    PromptContext {
        workspace_dir: std::path::Path::new(""),
        model_name: "",
        agent_id: "",
        tools: EMPTY_TOOLS,
        workflows: EMPTY_WORKFLOWS,
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: EMPTY_INTEGRATIONS,
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    }
}

/// Build a P-Format signature line (`name[a|b|c]`) from a `&dyn Tool`.
/// Used by `render_subagent_system_prompt` which operates on `Box<dyn Tool>`
/// directly (no intermediate `PromptTool`). Mirrors the `PromptTool` variant
/// below — both BTreeMap-iterate the schema's `properties` in the same order.
fn render_pformat_signature_for_box_tool(tool: &dyn crate::openhuman::tools::Tool) -> String {
    let schema = tool.parameters_schema();
    let names: Vec<String> = schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    if names.is_empty() {
        format!("{}[]", tool.name())
    } else {
        format!("{}[{}]", tool.name(), names.join("|"))
    }
}
