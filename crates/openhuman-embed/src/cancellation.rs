//! Cooperative cancellation with acknowledgement of all attached calls stopping.
use std::sync::Arc;
use tokio::sync::watch;

struct State {
    requested: watch::Sender<bool>,
    active: watch::Sender<usize>,
}

/// Cancellation shared by attached completers. Cancellation is permanent;
/// create a fresh handle for a new operation or request.
#[derive(Clone)]
pub struct Cancellation(Arc<State>);
impl Default for Cancellation {
    fn default() -> Self {
        Self(Arc::new(State {
            requested: watch::Sender::new(false),
            active: watch::Sender::new(0),
        }))
    }
}
impl std::fmt::Debug for Cancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cancellation")
            .field("requested", &*self.0.requested.borrow())
            .finish()
    }
}
impl Cancellation {
    /// Request cancellation and wait for all currently attached call futures
    /// to be dropped, observers notified, and reservations settled. Keep polling
    /// those calls (usually in another Tokio task) while awaiting this method.
    pub async fn cancel(&self) {
        self.0.requested.send_replace(true);
        let mut active = self.0.active.subscribe();
        let _ = active.wait_for(|count| *count == 0).await;
    }
    pub(crate) async fn cancelled(&self) {
        let mut requested = self.0.requested.subscribe();
        let _ = requested.wait_for(|value| *value).await;
    }
    pub(crate) fn enter(&self) -> Guard {
        self.0.active.send_modify(|count| *count += 1);
        Guard(self.clone())
    }
}
pub(crate) struct Guard(Cancellation);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0 .0.active.send_modify(|count| *count -= 1);
    }
}
