//! The artifact tools and the operator-save entry point, split from
//! [`super`] (`tiny_store.rs`) to keep that file under the layout limit.
//! Re-exported there, so callers keep using `artifacts::tiny_store::*`.

use std::path::Path;
use std::sync::Arc;

use tiny_artifacts::{Author, Content};

use super::artifacts_service;
use super::artifacts_service_with_publisher;
use super::SavedVersion;
use crate::artifacts::files::FileRoots;
use crate::security::approval::ApprovalChatContext;
use crate::tools::per_call::{context as per_call_context, PerCallTool};

/// Build the artifact tools bound to this workspace's service: the crate's
/// default set (`create_artifact`, `read_artifact`, `update_artifact`,
/// `list_artifacts`), `delete_artifact` (default-off through its
/// `tools::user_filter` family), and — only when a `publisher` exists, i.e. the
/// user can use cloud file storage — `share_artifact`.
///
/// The crate's tools run without a run context, so each call builds its tool
/// around a service whose UI bridge carries that call's chat routing
/// ([`PerCallTool`]); the card then reaches the chat the agent is answering.
pub fn registered_tools(
    workspace_dir: &Path,
    files: FileRoots,
    action_dir: &Path,
    publisher: Option<crate::artifacts::ArtifactPublisherBinder>,
) -> Vec<Box<dyn tinytools::Tool>> {
    let can_publish = publisher.is_some();
    let service = {
        let workspace_dir = workspace_dir.to_path_buf();
        let action_dir = action_dir.to_path_buf();
        Arc::new(move |chat: Option<&ApprovalChatContext>| {
            Arc::new(artifacts_service_with_publisher(
                &workspace_dir,
                files.clone(),
                Some(&action_dir),
                chat,
                publisher.as_ref(),
            ))
        })
    };
    let author = Author::agent(AGENT_AUTHOR_ID);
    let mut tools: Vec<Box<dyn tinytools::Tool>> = Vec::new();
    {
        let (service, author) = (service.clone(), author.clone());
        tools.push(Box::new(PerCallTool::new(
            per_call_context,
            move |context| {
                let create = tiny_artifacts::CreateArtifactTool::new(
                    service(context.chat.as_ref()),
                    author.clone(),
                );
                let tenant = context.dispatch.tenant();
                crate::artifacts::box_source::BoxSourced::new(
                    create,
                    crate::artifacts::box_source::KindSource::Args,
                    Arc::new(crate::artifacts::box_source::SandboxBoxFiles::new(
                        context.dispatch,
                    )),
                    tenant,
                )
            },
        )));
    }
    tools.push(Box::new(tiny_artifacts::ReadArtifactTool::new(service(
        None,
    ))));
    {
        let (service, author) = (service.clone(), author.clone());
        tools.push(Box::new(PerCallTool::new(
            per_call_context,
            move |context| {
                let artifacts = service(context.chat.as_ref());
                let update =
                    tiny_artifacts::UpdateArtifactTool::new(Arc::clone(&artifacts), author.clone());
                let tenant = context.dispatch.tenant();
                crate::artifacts::box_source::BoxSourced::new(
                    update,
                    crate::artifacts::box_source::KindSource::Record(artifacts),
                    Arc::new(crate::artifacts::box_source::SandboxBoxFiles::new(
                        context.dispatch,
                    )),
                    tenant,
                )
            },
        )));
    }
    tools.push(Box::new(tiny_artifacts::ListArtifactsTool::new(service(
        None,
    ))));
    {
        let service = service.clone();
        tools.push(Box::new(PerCallTool::new(
            per_call_context,
            move |context| tiny_artifacts::DeleteArtifactTool::new(service(context.chat.as_ref())),
        )));
    }
    if can_publish {
        tools.push(Box::new(PerCallTool::new(
            per_call_context,
            move |context| tiny_artifacts::ShareArtifactTool::new(service(context.chat.as_ref())),
        )));
    }
    tools
}

/// Author id stamped on versions the user writes in the desktop editor.
pub const OPERATOR_AUTHOR_ID: &str = "operator";

/// Author id stamped on versions OpenHuman's agent writes — through the
/// artifact tools and through producer tools alike. A tool run carries no
/// agent identity, so every agent-written version is attributed to this one.
pub const AGENT_AUTHOR_ID: &str = "openhuman";

/// Append the user's edit of artifact `id` as a new operator-authored version
/// (the preview panel's autosave). The UI bridge writes it to the files folder
/// and updates the card. A body identical to the newest version appends
/// nothing.
pub async fn save_operator_version(
    workspace_dir: &Path,
    files: FileRoots,
    id: &str,
    body: &str,
    ctx: &crate::core::runtime::DispatchCtx,
) -> Result<SavedVersion, String> {
    let saved = artifacts_service(workspace_dir, files, None, None, ctx)
        .revise(
            id,
            Content::Text(body.to_string()),
            &Author::operator(OPERATOR_AUTHOR_ID),
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    if let Some(notice) = &saved.notice {
        tracing::warn!(target: "tiny-artifacts", artifact_id = %id, "[tiny-artifacts] {notice}");
    }
    Ok(SavedVersion {
        version: saved.version,
        saved: saved.appended,
    })
}
