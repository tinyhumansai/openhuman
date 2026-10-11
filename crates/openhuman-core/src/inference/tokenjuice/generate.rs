//! OpenHuman's turn-bound callbacks for the TinyJuice module's `MlHost.Generate`.
//!
//! TinyJuice writes the summary prompt and decides when one is worth writing;
//! the model call is ours, because only the host knows which model the turn
//! runs on, and the call must inherit that turn's cancellation, thread and
//! lineage. None of that crosses the bus, and task-local state does not cross
//! a module boundary either, so the turn is passed by reference instead:
//!
//! 1. before `CompactWith`, the caller [`register`]s a [`PreparedGenerate`] —
//!    a closure already bound to its turn — and sends the returned ticket's
//!    token as `context_token`;
//! 2. if the module asks for a summary, `MlHost.Generate` arrives with that
//!    token and [`serve`] runs the closure;
//! 3. the ticket removes the entry when dropped, whether or not it was used.
//!
//! An unknown token (expired, never registered, or already used) is a decline,
//! not an error: the module then falls back to its deterministic compressors.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{LazyLock, Mutex};

pub use tinyjuice_bus::wire::GenerateRequest;

/// One model call, bound to the turn that will pay for it.
pub type PreparedGenerate = Box<
    dyn FnOnce(GenerateRequest) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>
        + Send,
>;

static PENDING: LazyLock<Mutex<HashMap<String, PreparedGenerate>>> =
    LazyLock::new(Default::default);

/// Holds one registered call. Dropping it withdraws the call.
#[must_use = "dropping the ticket withdraws the registered call"]
pub struct GenerateTicket {
    token: String,
}

impl GenerateTicket {
    /// The value to send as `CompactRequest::context_token`.
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for GenerateTicket {
    fn drop(&mut self) {
        if let Ok(mut pending) = PENDING.lock() {
            pending.remove(&self.token);
        }
    }
}

/// Register a call and get the ticket that names it.
pub fn register(prepared: PreparedGenerate) -> GenerateTicket {
    let token = uuid::Uuid::new_v4().to_string();
    if let Ok(mut pending) = PENDING.lock() {
        pending.insert(token.clone(), prepared);
    }
    log::debug!("[tokenjuice::generate] registered context_token={token}");
    GenerateTicket { token }
}

/// Answer one `MlHost.Generate` call. `Ok(None)` declines.
pub async fn serve(request: GenerateRequest) -> Result<Option<String>, String> {
    let token = request.context_token.clone();
    let prepared = PENDING.lock().ok().and_then(|mut p| p.remove(&token));
    let Some(prepared) = prepared else {
        log::debug!("[tokenjuice::generate] no registered call, declining context_token={token}");
        return Ok(None);
    };
    let prompt_bytes = request.prompt.len();
    let purpose = request.purpose.clone();
    let started = std::time::Instant::now();
    match prepared(request).await {
        Ok(text) => {
            log::info!(
                "[tokenjuice::generate] served context_token={token} purpose={purpose} \
                 prompt_bytes={prompt_bytes} reply_bytes={} elapsed_ms={}",
                text.len(),
                started.elapsed().as_millis()
            );
            Ok(Some(text))
        }
        Err(error) => {
            log::warn!(
                "[tokenjuice::generate] model call failed context_token={token} purpose={purpose} \
                 error={error}"
            );
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
#[path = "generate_tests.rs"]
mod tests;
