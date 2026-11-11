//! Host-owned parent-epoch metadata for TinyAgents subagent sessions.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tinyagents_orchestration::subagent::SubagentError;

use super::storage_schema::EPOCHS;

#[derive(Deserialize, Serialize)]
struct ParentEpochRecord {
    parent_epoch: Option<u64>,
}

pub(super) fn import_parent_epoch(
    documents: Arc<dyn crate::storage::DocumentStore>,
    session_id: String,
    parent_epoch: Option<u64>,
) -> Result<(), SubagentError> {
    crate::storage::block_on(async move {
        documents
            .ensure_collection(&tinystoragedrivers::CollectionSpec::new(EPOCHS))
            .await?;
        let value = serde_json::to_value(ParentEpochRecord { parent_epoch })
            .map_err(|error| crate::storage::StorageError::backend(error.to_string()))?;
        crate::storage::documents::update_or_insert(&documents, EPOCHS, &session_id, move |_| {
            Some(value.clone())
        })
        .await?;
        Ok(())
    })
    .map_err(|error| SubagentError::Persistence(format!("import parent epoch: {error}")))
}

pub(crate) fn parent_epoch_from_sidecar(
    documents: Arc<dyn crate::storage::DocumentStore>,
    session_id: String,
) -> Result<Option<u64>, SubagentError> {
    crate::storage::block_on(async move {
        documents
            .ensure_collection(&tinystoragedrivers::CollectionSpec::new(EPOCHS))
            .await?;
        let Some(record) = documents.get(EPOCHS, &session_id).await? else {
            return Ok(None);
        };
        let record = serde_json::from_value::<ParentEpochRecord>(record.doc)
            .map_err(|error| crate::storage::StorageError::backend(error.to_string()))?;
        Ok(record.parent_epoch)
    })
    .map_err(|error| SubagentError::Persistence(format!("read parent epoch: {error}")))
}

pub(crate) fn parent_epoch_for_context(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    session_id: String,
) -> Result<Option<u64>, SubagentError> {
    let open_documents = || {
        let opened =
            crate::storage::local::open("subagent_sessions", workspace_dir).map_err(|error| {
                SubagentError::Persistence(format!("open session store: {error:#}"))
            })?;
        let scoped = opened.scoped().map_err(|error| {
            SubagentError::Persistence(format!("scope session store: {error:#}"))
        })?;
        Ok::<_, SubagentError>(Arc::clone(scoped.documents()))
    };
    let context = ctx.resolved().ok_or_else(|| {
        SubagentError::Persistence("subagent session store has no resolved owner scope".into())
    })?;
    parent_epoch_from_sidecar(
        crate::core::runtime::CoreContext::sync_scope(context, open_documents)?,
        session_id,
    )
}

pub(crate) fn set_parent_epoch_for_context(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace_dir: &std::path::Path,
    session_id: String,
    parent_epoch: Option<u64>,
) -> Result<(), SubagentError> {
    const DOMAIN: &str = "subagent_sessions";
    let open_documents = || {
        let opened = crate::storage::local::open(DOMAIN, workspace_dir).map_err(|error| {
            SubagentError::Persistence(format!("open session store: {error:#}"))
        })?;
        let scoped = opened.scoped().map_err(|error| {
            SubagentError::Persistence(format!("scope session store: {error:#}"))
        })?;
        Ok::<_, SubagentError>(Arc::clone(scoped.documents()))
    };
    let context = ctx.resolved().ok_or_else(|| {
        SubagentError::Persistence("subagent session store has no resolved owner scope".into())
    })?;
    let documents = crate::core::runtime::CoreContext::sync_scope(context, open_documents)?;
    import_parent_epoch(documents, session_id, parent_epoch)
}
