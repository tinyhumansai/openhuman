//! Bounded batch forwarding from module-owned continuous microphone capture.
use crate::config::Config;
use crate::voice::capture_lease::{CaptureLease, Kind};
pub(super) use tinyvoice_bus::capture::{CaptureFormat, RawChunk};
static STOP: std::sync::LazyLock<tokio::sync::watch::Sender<u64>> =
    std::sync::LazyLock::new(|| tokio::sync::watch::channel(0).0);
pub(super) fn stop_subscription() -> tokio::sync::watch::Receiver<u64> {
    STOP.subscribe()
}
pub(super) fn request_stop() {
    STOP.send_modify(|generation| *generation = generation.wrapping_add(1));
}
/// Owns the forwarding task; cancellation closes its opaque capture lease.
pub(super) struct CapturePump(tokio::task::JoinHandle<()>);
impl Drop for CapturePump {
    fn drop(&mut self) {
        self.0.abort();
    }
}
/// Start a module stream and forward ordered batches to the bounded processor queue.
pub(super) async fn spawn_capture_thread(
    config: &Config,
    tx: tokio::sync::mpsc::Sender<RawChunk>,
) -> Result<(CaptureFormat, CapturePump), String> {
    let lease = CaptureLease::reserve(config, Kind::Stream).await?;
    let stream = lease.start_stream().await?;
    let mut stopped = STOP.subscribe();
    let task = crate::core::runtime::spawn_scoped(async move {
        loop {
            if tx.is_closed() {
                break;
            }
            let reply = tokio::select! {
                biased;
                _ = stopped.changed() => break,
                reply = lease.poll() => reply,
            };
            let batch = match reply {
                Ok(batch) => batch,
                Err(error) => {
                    log::warn!("[voice::always_on] capture batch unavailable: {error}");
                    break;
                }
            };
            for chunk in batch.chunks {
                tokio::select! {
                    biased;
                    _ = stopped.changed() => return,
                    result = tx.send(chunk) => if result.is_err() { return; },
                }
            }
            if batch.closed {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        if let Err(error) = lease.close().await {
            log::warn!("[voice::always_on] capture cleanup pending: {error}");
        }
    });
    Ok((stream.format, CapturePump(task)))
}
