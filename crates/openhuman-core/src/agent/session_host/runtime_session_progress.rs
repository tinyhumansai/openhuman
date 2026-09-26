//! Post-commit progress delivery for a chat turn.

use crate::agent::progress::AgentProgress;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use tinyagents_runtime::CommitReceipt;

/// Preserve the response path if a progress receiver stays open but stops
/// consuming events. The web bridge has its own bounded drain wait afterward.
const COMMITTED_TURN_PROGRESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Route the commit fence to the turn that produced the receipt. A warm
/// session reuses its commit callback, so a sender captured when the session
/// was created would point at a prior turn's bridge.
pub(super) async fn send_receipt_progress(
    receipt: &CommitReceipt<OpenHumanRunContext>,
    input: &str,
    output: &str,
    iterations: u32,
) -> bool {
    match receipt.options.context.progress.as_ref() {
        Some(progress) => send_committed_turn_progress(progress, input, output, iterations).await,
        None => false,
    }
}

/// Send the terminal event as the progress bridge's drain fence. Content
/// capture stays best effort; the completion waits for channel capacity, up to
/// a deadline, so a healthy bridge can forward queued tool events first.
pub(super) async fn send_committed_turn_progress(
    progress: &tokio::sync::mpsc::Sender<AgentProgress>,
    input: &str,
    output: &str,
    iterations: u32,
) -> bool {
    let content = AgentProgress::TurnContent {
        input: Some(input.to_string()),
        output: Some(output.to_string()),
    };
    // With two free slots, preserve the usual content-then-completion order.
    // A full channel must reserve its next slot for the terminal fence. Send
    // content after that fence on a short-lived clone so trace IO is still
    // captured when the bridge catches up.
    let delayed_content = if progress.capacity() >= 2 {
        progress.try_send(content).err().map(|err| err.into_inner())
    } else {
        Some(content)
    };
    let completed = match tokio::time::timeout(
        COMMITTED_TURN_PROGRESS_TIMEOUT,
        progress.send(AgentProgress::TurnCompleted { iterations }),
    )
    .await
    {
        Ok(Ok(())) => true,
        Ok(Err(_)) => {
            log::warn!(
                "[agent_session] committed turn completion not delivered: progress receiver closed"
            );
            false
        }
        Err(_) => {
            log::warn!(
                "[agent_session] committed turn completion not delivered within {:?}: progress receiver stalled",
                COMMITTED_TURN_PROGRESS_TIMEOUT
            );
            false
        }
    };
    if completed {
        if let Some(content) = delayed_content {
            let progress = progress.clone();
            tokio::spawn(async move {
                if !matches!(
                    tokio::time::timeout(COMMITTED_TURN_PROGRESS_TIMEOUT, progress.send(content))
                        .await,
                    Ok(Ok(()))
                ) {
                    log::warn!(
                        "[agent_session] committed turn content not delivered after completion"
                    );
                }
            });
        }
    }
    completed
}
