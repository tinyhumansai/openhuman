use super::*;

pub(super) async fn attach(
    run_context: &OpenHumanRunContext,
    tool_result_artifact_index: Option<Arc<ToolResultArtifactIndexStore>>,
) -> Option<StoreRegistry> {
    // Assemble the run's store registry: the tool-result artifact index (when
    // present) and — behind the default-ON session dual-write flag — the
    // session KV store, so the harness carries a handle to the same
    // `{workspace}/tinyagents_store/kv` tree the live dual-write mirrors into
    // (issue #4249, 04.1). Both stores share one registry so neither clobbers
    // the other. Reads stay legacy until 04.2; this registration is additive
    // and best-effort (a workspace-resolve failure just skips it).
    let mut stores: Option<StoreRegistry> = None;
    if let Some(index) = tool_result_artifact_index.clone() {
        stores
            .get_or_insert_with(StoreRegistry::new)
            .register(TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE, index);
    }
    // `session_kv_store` self-gates on the dual-write flag (config default ON +
    // env kill switch), returning `None` when disabled or unresolvable.
    if let Some(session_kv) =
        crate::storage::import::live::session_kv_store(&run_context.dispatch_ctx()).await
    {
        stores.get_or_insert_with(StoreRegistry::new).register(
            crate::storage::import::live::TINYAGENTS_SESSION_KV_STORE,
            session_kv,
        );
        tracing::debug!(
            "[session-store] registered session kv store on RunContext.stores under '{}'",
            crate::storage::import::live::TINYAGENTS_SESSION_KV_STORE
        );
    }
    stores
}
