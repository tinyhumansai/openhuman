//! OpenHuman lifecycle observations for a neutral TinyAgents session.

use async_trait::async_trait;
use std::{future::Future, pin::Pin, sync::Arc};
use tinyagents_runtime::{
    CommitReceipt, ResumePreparation, RuntimeError, SessionHooks, SessionStateView,
    SessionTerminal, SessionTurnOutcome, SessionTurnRequest, TranscriptTurnOptions, TurnOptions,
    TurnPreparation,
};

use crate::agent::tinyagents::host::OpenHumanRunContext;

/// Product callbacks attached to one runtime session.
///
/// The runtime makes the durable commit before `after_commit` and guarantees a
/// single terminal callback.  OpenHuman adapters use these points for progress,
/// BUS projection, memory/experience work and usage finalization.
pub struct OpenHumanSessionHooks {
    before_resume: Arc<
        dyn for<'a> Fn(
                &'a mut SessionTurnRequest,
                &'a mut TurnOptions<OpenHumanRunContext>,
                SessionStateView<'a>,
            ) -> HookFuture<'a, ResumePreparation>
            + Send
            + Sync,
    >,
    before: Arc<
        dyn for<'a> Fn(
                &'a mut SessionTurnRequest,
                &'a mut TurnOptions<OpenHumanRunContext>,
                SessionStateView<'a>,
            ) -> HookFuture<'a, TurnPreparation>
            + Send
            + Sync,
    >,
    before_commit: Arc<
        dyn for<'a> Fn(
                &'a SessionTurnOutcome,
                &'a TranscriptTurnOptions<OpenHumanRunContext>,
            ) -> HookFuture<'a, ()>
            + Send
            + Sync,
    >,
    committed:
        Arc<dyn Fn(CommitReceipt<OpenHumanRunContext>) -> HookFuture<'static, ()> + Send + Sync>,
    terminal: Arc<dyn Fn(SessionTerminal) -> HookFuture<'static, ()> + Send + Sync>,
}

type HookFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, RuntimeError>> + Send + 'a>>;

impl OpenHumanSessionHooks {
    /// Installs product callbacks. Callbacks are intentionally synchronous;
    /// expensive memory and learning work should be spawned by `committed` so
    /// it cannot relabel an already durable turn.
    pub fn new(
        before_resume: impl for<'a> Fn(
            &'a mut SessionTurnRequest,
            &'a mut TurnOptions<OpenHumanRunContext>,
            SessionStateView<'a>,
        ) -> HookFuture<'a, ResumePreparation>
        + Send
        + Sync
        + 'static,
        before: impl for<'a> Fn(
            &'a mut SessionTurnRequest,
            &'a mut TurnOptions<OpenHumanRunContext>,
            SessionStateView<'a>,
        ) -> HookFuture<'a, TurnPreparation>
        + Send
        + Sync
        + 'static,
        before_commit: impl for<'a> Fn(
            &'a SessionTurnOutcome,
            &'a TranscriptTurnOptions<OpenHumanRunContext>,
        ) -> HookFuture<'a, ()>
        + Send
        + Sync
        + 'static,
        committed: impl Fn(CommitReceipt<OpenHumanRunContext>) -> HookFuture<'static, ()>
        + Send
        + Sync
        + 'static,
        terminal: impl Fn(SessionTerminal) -> HookFuture<'static, ()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            before_resume: Arc::new(before_resume),
            before: Arc::new(before),
            before_commit: Arc::new(before_commit),
            committed: Arc::new(committed),
            terminal: Arc::new(terminal),
        }
    }
}

#[async_trait]
impl SessionHooks<OpenHumanRunContext> for OpenHumanSessionHooks {
    async fn before_resume(
        &self,
        request: &mut SessionTurnRequest,
        options: &mut TurnOptions<OpenHumanRunContext>,
        state: SessionStateView<'_>,
    ) -> Result<ResumePreparation, RuntimeError> {
        (self.before_resume)(request, options, state).await
    }
    async fn before_turn(
        &self,
        request: &mut SessionTurnRequest,
        options: &mut TurnOptions<OpenHumanRunContext>,
        state: SessionStateView<'_>,
    ) -> Result<TurnPreparation, RuntimeError> {
        (self.before)(request, options, state).await
    }
    async fn before_commit(
        &self,
        outcome: &SessionTurnOutcome,
        options: &TranscriptTurnOptions<OpenHumanRunContext>,
    ) -> Result<(), RuntimeError> {
        (self.before_commit)(outcome, options).await
    }
    async fn after_commit(
        &self,
        receipt: CommitReceipt<OpenHumanRunContext>,
    ) -> Result<(), RuntimeError> {
        (self.committed)(receipt).await
    }
    async fn on_terminal(&self, terminal: SessionTerminal) -> Result<(), RuntimeError> {
        (self.terminal)(terminal).await
    }
}
