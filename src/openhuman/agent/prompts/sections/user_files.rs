use crate::openhuman::agent::prompts::types::{PromptContext, PromptSection, USER_FILE_MAX_CHARS};
use crate::openhuman::agent::prompts::workspace_files::{
    inject_snapshot_content, inject_workspace_file_capped,
};
use anyhow::Result;

/// Injects the user-specific, session-frozen workspace files
/// (`PROFILE.md` + `MEMORY.md`), each capped at [`USER_FILE_MAX_CHARS`].
///
/// Separate from [`IdentitySection`] so agents that strip the project-
/// context preamble (`omit_identity = true` — welcome, orchestrator,
/// the trigger pair) still get their user-file injection at runtime via
/// [`SystemPromptBuilder::for_subagent`], which skips `IdentitySection`
/// entirely when `omit_identity` is on.
///
/// Cache-stability: static per session — the whole point of the
/// 2000-char cap and the load-once rule documented on
/// [`AgentDefinition::omit_profile`] / `omit_memory_md`.
pub struct UserFilesSection;

impl PromptSection for UserFilesSection {
    fn name(&self) -> &str {
        "user_files"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        // Gate on the per-agent flags derived from
        // `AgentDefinition::omit_profile` / `omit_memory_md`. Both files
        // are user-specific, potentially growing, and capped at
        // [`USER_FILE_MAX_CHARS`] (~1000 tokens) so they can't bloat the
        // cached prefix.
        //
        // KV-cache contract: once injected into a session's rendered
        // prompt, the bytes are frozen for the remainder of that
        // session — any mid-session archivist write or enrichment
        // refresh lands on the NEXT session, never the in-flight one.
        let mut out = String::new();
        if ctx.include_profile {
            inject_workspace_file_capped(
                &mut out,
                ctx.workspace_dir,
                "PROFILE.md",
                USER_FILE_MAX_CHARS,
            );
        }
        if ctx.include_memory_md {
            // Prefer the session-frozen curated-memory snapshot when the
            // session has taken one — that's the runtime-writable store
            // behind `curated_memory.add/replace/remove`. Fall back to
            // the workspace file only when no snapshot is attached (pure
            // prompt-unit tests and older call sites).
            if let Some(snap) = ctx.curated_snapshot {
                inject_snapshot_content(&mut out, "MEMORY.md", &snap.memory, USER_FILE_MAX_CHARS);
                inject_snapshot_content(&mut out, "USER.md", &snap.user, USER_FILE_MAX_CHARS);
            } else {
                inject_workspace_file_capped(
                    &mut out,
                    ctx.workspace_dir,
                    "MEMORY.md",
                    USER_FILE_MAX_CHARS,
                );
            }
        }
        Ok(out)
    }
}

/// Render the `PROFILE.md` + `MEMORY.md` user-file injection.
/// Empty when neither `ctx.include_profile` nor `ctx.include_memory_md`
/// is set.
pub fn render_user_files(ctx: &PromptContext<'_>) -> Result<String> {
    UserFilesSection.build(ctx)
}
