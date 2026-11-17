use crate::config::Config;

/// Stage in-process attachments in the workspace exposed to this turn and
/// prepend their model-visible references to the user's message.
pub(super) async fn stage_host_attachments(
    attachments: &[crate::agent::HostAttachment],
    session_id: &str,
    config: &Config,
    cwd: Option<std::path::PathBuf>,
    message: &str,
) -> Result<String, String> {
    let staging_config = match cwd.as_ref() {
        Some(root) => {
            let mut scoped = config.clone();
            scoped.action_dir = root.clone();
            scoped
        }
        None => config.clone(),
    };
    let scope = crate::attachments::AttachmentAccessScope {
        external_channel: false,
        workspace: cwd,
        ..Default::default()
    };
    let references =
        crate::attachments::host::stage_host(attachments, session_id, &staging_config, &scope)
            .await
            .map_err(|error| error.to_string())?;
    log::debug!(
        "[inference] agent_chat staged {} host attachment(s)",
        attachments.len()
    );
    Ok(format!("{references}{message}"))
}
