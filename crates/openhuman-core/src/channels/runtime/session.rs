//! Host-owned logout generation and cancellation for channel orchestration.
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex};
use tokio_util::sync::CancellationToken;

struct SessionState {
    token: CancellationToken,
    paused: bool,
}

/// Controls one listener-service generation and its post-logout restart signal.
#[derive(Clone)]
pub(crate) struct ChannelSessionController(Arc<ControllerState>);

struct ControllerState {
    session: Mutex<SessionState>,
    resume: tokio::sync::Notify,
}

impl ChannelSessionController {
    fn new() -> Self {
        Self(Arc::new(ControllerState {
            session: Mutex::new(SessionState {
                token: CancellationToken::new(),
                paused: false,
            }),
            resume: tokio::sync::Notify::new(),
        }))
    }

    fn current(&self) -> CancellationToken {
        self.0
            .session
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .token
            .clone()
    }

    fn invalidate(&self) {
        let mut state = self
            .0
            .session
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.token.cancel();
        state.token = CancellationToken::new();
        state.paused = true;
    }

    fn resume(&self) {
        let resumed = {
            let mut state = self
                .0
                .session
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.paused {
                state.paused = false;
                true
            } else {
                false
            }
        };
        if resumed {
            self.0.resume.notify_one();
        }
    }

    async fn wait_for_resume(&self) -> CancellationToken {
        loop {
            let resumed = self.0.resume.notified();
            let token = {
                let state = self
                    .0
                    .session
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                (!state.paused).then(|| state.token.clone())
            };
            if let Some(token) = token {
                return token;
            }
            resumed.await;
        }
    }
}

static SESSION: LazyLock<ChannelSessionController> = LazyLock::new(ChannelSessionController::new);

pub(crate) fn channel_session() -> CancellationToken {
    SESSION.current()
}

pub(crate) fn invalidate_channel_session() {
    SESSION.invalidate();
}

/// Resume channel listeners after a credential has been successfully installed.
pub(crate) fn resume_channel_service() {
    SESSION.resume();
}

fn spawn_channel_service_with<F, Fut>(
    dispatch: &crate::core::runtime::DispatchCtx,
    controller: ChannelSessionController,
    mut run: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut(CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    dispatch.spawn(async move {
        // A logout can race the service task's first poll. Do not let a
        // service created during that window start with the replacement
        // token before a credential has been installed again.
        let mut session = controller.wait_for_resume().await;
        loop {
            if let Err(error) = run(session.clone()).await {
                tracing::error!(domain = "channels", operation = "service", error = %error, "channel service generation stopped");
                // In particular, a failed close must retain ownership and
                // stop here instead of admitting a second listener against
                // the still-live generation.
                return;
            }
            if !session.is_cancelled() {
                return;
            }
            session = controller.wait_for_resume().await;
        }
    })
}

pub(crate) fn spawn_current_channel_service_with<F, Fut>(
    dispatch: &crate::core::runtime::DispatchCtx,
    run: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut(CancellationToken) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    spawn_channel_service_with(dispatch, SESSION.clone(), run)
}

/// Stop the host dispatch generation on logout, even with independent model auth.
pub(super) async fn run_in_session(
    session: CancellationToken,
    runtime: impl Future<Output = anyhow::Result<()>>,
) -> anyhow::Result<()> {
    tokio::select! {
        biased;
        _ = session.cancelled() => Ok(()),
        result = runtime => result,
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
