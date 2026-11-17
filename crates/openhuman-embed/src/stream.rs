//! Owned turn streaming over the harness's existing progress sink.
use crate::{AgentProgress, CoreError, Turn, TurnOutcome};
use futures_core::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
pub use tinyagents_harness::cancel::CancellationToken;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// One live progress notification or the final typed turn result.
#[derive(Debug)]
pub enum StreamEvent {
    /// Token deltas, tool events, and harness boundaries as they arrive.
    Progress(AgentProgress),
    /// Exactly one final result, after all preceding progress.
    Finished(Result<TurnOutcome, CoreError>),
}

/// A bounded stream that cooperatively cancels its turn when dropped.
pub struct TurnStream {
    inner: ReceiverStream<StreamEvent>,
    cancellation: CancellationToken,
}
impl TurnStream {
    pub(crate) fn start(turn: Turn) -> Self {
        let cancellation = turn.stream_cancellation();
        let (tx, rx) = mpsc::channel(64);
        let (progress_tx, mut progress_rx) = mpsc::channel(64);
        let token = cancellation.clone();
        tokio::spawn(async move {
            let mut result = Box::pin(
                turn.on_progress(progress_tx)
                    .cancellation(token.clone())
                    .send(),
            );
            let (outcome, pending) = loop {
                tokio::select! {
                    biased;
                    outcome = &mut result => break (outcome, None),
                    item = progress_rx.recv() => {
                        if let Some(item) = item {
                            // A blocked stream consumer must not stop polling
                            // turn cancellation, deadlines or cleanup.
                            let retained = item.clone();
                            tokio::select! {
                                outcome = &mut result => break (outcome, Some(retained)),
                                sent = tx.send(StreamEvent::Progress(item)) => { if sent.is_err() { token.cancel(); } },
                                _ = token.cancelled() => {},
                            }
                        } else {
                            break ((&mut result).await, None);
                        }
                    }
                }
            };
            drop(result);
            // The item whose send lost to completion precedes everything still
            // buffered. Keep progress ordered before the terminal notification.
            let mut pending = pending;
            while let Some(item) = pending.take().or_else(|| progress_rx.try_recv().ok()) {
                tokio::select! { _ = tx.send(StreamEvent::Progress(item)) => {}, _ = token.cancelled() => {} }
            }
            let _ = tx.send(StreamEvent::Finished(outcome)).await;
        });
        Self {
            inner: ReceiverStream::new(rx),
            cancellation,
        }
    }
    /// Receive the next notification without importing a stream extension trait.
    pub async fn recv(&mut self) -> Option<StreamEvent> {
        std::future::poll_fn(|cx| Pin::new(&mut *self).poll_next(cx)).await
    }
    /// The token shared with this turn and its recursive children.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}
impl Stream for TurnStream {
    type Item = StreamEvent;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}
impl Drop for TurnStream {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
