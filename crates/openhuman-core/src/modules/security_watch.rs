//! Subscriber for the module's `PolicyChanged` signal.
//!
//! The host decision cache is already keyed by the module generation and
//! flushes whenever a response carries a new one. This subscriber closes the
//! remaining gap: a change committed by another caller is dropped from the
//! cache as soon as the module announces it, before the next response arrives.
//! The signal is a hint. Correctness never depends on it arriving.

use std::sync::atomic::{AtomicBool, Ordering};

use tinybus::module::ModuleRuntime;
use tinybus::{Message, MessageKind};
use tinysecurity_bus::{names, PolicyChanged};

static WATCHING: AtomicBool = AtomicBool::new(false);

/// Parse a `PolicyChanged` signal. Only a broadcast from `module_bus_name` on
/// the module's own interface and member is accepted; anything else is ignored.
pub(crate) fn decode_policy_changed(
    message: &Message,
    module_bus_name: &str,
) -> Option<PolicyChanged> {
    let header = &message.header;
    if header.kind != MessageKind::Signal
        || header.interface.as_ref().map(|i| i.as_str()) != Some(names::INTERFACE)
        || header.member.as_ref().map(|m| m.as_str()) != Some(names::POLICY_CHANGED)
    {
        return None;
    }
    let sender = header.sender.as_ref()?;
    if sender.as_str() != module_bus_name {
        log::debug!("[security][watch] ignored PolicyChanged from an unexpected sender");
        return None;
    }
    let (event,): (PolicyChanged,) = serde_json::from_value(message.body.clone()).ok()?;
    Some(event)
}

/// Start the subscriber once per process. A failure to subscribe is logged and
/// leaves generation-keyed invalidation as the only (still sound) mechanism.
pub(crate) fn ensure_watching(runtime: &'static ModuleRuntime, module_bus_name: &'static str) {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let connection = runtime.connection().clone();
    runtime.spawn(async move {
        let rule = tinybus::MatchRule::new()
            .signals()
            .interface(names::INTERFACE.try_into().expect("static interface"))
            .member(names::POLICY_CHANGED.try_into().expect("static member"));
        let mut receiver = match connection.add_match(rule).await {
            Ok(receiver) => receiver,
            Err(_) => {
                log::warn!("[security][watch] PolicyChanged subscription failed");
                WATCHING.store(false, Ordering::SeqCst);
                return;
            }
        };
        log::debug!("[security][watch] subscribed to PolicyChanged");
        loop {
            match receiver.recv().await {
                Ok(message) => {
                    if let Some(event) = decode_policy_changed(&message, module_bus_name) {
                        log::debug!(
                            "[security][watch] PolicyChanged generation={}",
                            event.generation
                        );
                        super::security::scoped_client::policy_changed(&event).await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Missed signals: flush conservatively via a zero-event.
                    super::security::scoped_client::flush_all().await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        WATCHING.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
#[path = "security_watch_tests.rs"]
mod tests;
