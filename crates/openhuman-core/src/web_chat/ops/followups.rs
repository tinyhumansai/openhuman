//! Queued follow-up turns: once a turn finishes, the items its run queue
//! drained are started as fresh turns under the context the finished turn ran in.

use crate::web_chat::ops::start_chat;
use crate::web_chat::ChatRequestMetadata;

pub(super) fn dispatch_followups(
    ctx: &crate::core::runtime::DispatchCtx,
    followups: Vec<crate::agent::queued_turn::QueuedTurn>,
) {
    for fup in followups {
        let ctx = ctx.clone();
        tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
            async move {
                if let Err(err) = start_chat(
                    &ctx,
                    &fup.client_id,
                    &fup.thread_id,
                    &fup.text,
                    fup.model_override,
                    fup.temperature,
                    fup.locale,
                    Some("followup".to_string()),
                    ChatRequestMetadata::default(),
                )
                .await
                {
                    log::warn!(
                        "[web-channel] failed to dispatch followup thread_id={} err={}",
                        fup.thread_id,
                        err
                    );
                }
            },
        ));
    }
}
