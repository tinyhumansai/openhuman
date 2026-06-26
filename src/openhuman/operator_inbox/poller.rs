//! Background IMAP polling scheduler for operator inbox.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tracing::{debug, info, warn};

use super::connection::fetch_new_emails;
use super::engine;
use super::imap_client::ImapConfig;
use super::types::MessageSource;

const LOG_PREFIX: &str = "[operator-inbox-poller]";
const DEFAULT_POLL_INTERVAL_SECS: u64 = 120;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Start the background IMAP polling loop. Returns false if already running.
pub fn start_polling(config: ImapConfig, interval_secs: Option<u64>) -> bool {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return false;
    }
    let interval = Duration::from_secs(interval_secs.unwrap_or(DEFAULT_POLL_INTERVAL_SECS));
    tokio::spawn(async move {
        info!("{LOG_PREFIX} started (interval={:?})", interval);
        loop {
            if !RUNNING.load(Ordering::SeqCst) {
                info!("{LOG_PREFIX} stopped");
                break;
            }
            match fetch_new_emails(&config).await {
                Ok(result) if result.new_count > 0 => {
                    info!("{LOG_PREFIX} fetched {} new emails", result.new_count);
                    for email in &result.emails {
                        engine::triage_message(
                            MessageSource::Email,
                            &email.from,
                            &email.subject,
                            &email.body_text,
                        );
                    }
                }
                Ok(_) => debug!("{LOG_PREFIX} no new emails"),
                Err(e) => warn!("{LOG_PREFIX} fetch failed: {e}"),
            }
            tokio::time::sleep(interval).await;
        }
    });
    true
}

/// Stop the background polling loop.
pub fn stop_polling() {
    RUNNING.store(false, Ordering::SeqCst);
}

/// Check if the poller is running.
pub fn is_polling() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_running_by_default() {
        assert!(!is_polling());
    }

    #[test]
    fn stop_is_idempotent() {
        stop_polling();
        assert!(!is_polling());
    }
}
