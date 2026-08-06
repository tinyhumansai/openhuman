//! LLM-callable wrappers for the WhatsApp data store (issue #1341).
//!
//! The store itself lives in the Tauri shell. Each tool dispatches its query
//! over the in-process native request bus
//! ([`crate::core::event_bus::request_native_global`]) to the shell-registered
//! handler, unwraps the typed response, and emits a compact JSON object that
//! includes a `"provider": "whatsapp"` provenance tag so replies can cite
//! WhatsApp as the source.
//!
//! **Graceful degradation.** In a headless / CLI / docker build there is no
//! desktop shell, so no handler is registered. In that case the native
//! dispatch returns [`NativeRequestError::NotInitialized`] or
//! [`NativeRequestError::UnregisteredHandler`]; the tools treat that as
//! "WhatsApp data unavailable (desktop only)" and return an empty, well-formed
//! result rather than surfacing an error to the agent. A genuine handler-side
//! failure ([`NativeRequestError::HandlerFailed`] / [`NativeRequestError::TypeMismatch`])
//! still propagates as a tool error.
//!
//! The write-path `whatsapp_data.ingest` is intentionally NOT wrapped here —
//! it is a scanner-only write path, dispatched by the Tauri shell scanner
//! directly. Exposing it as an agent tool would reopen the read-only boundary
//! this module exists to preserve.

mod list_chats;
mod list_messages;
mod search_messages;

pub use list_chats::WhatsAppDataListChatsTool;
pub use list_messages::WhatsAppDataListMessagesTool;
pub use search_messages::WhatsAppDataSearchMessagesTool;

use crate::core::event_bus::NativeRequestError;

/// Note surfaced when the WhatsApp data store is unavailable because no desktop
/// shell handler is registered (headless / CLI / docker builds).
pub(crate) const UNAVAILABLE_NOTE: &str = "WhatsApp data unavailable (desktop only)";

/// True when `err` means "no shell handler is wired" — i.e. the native registry
/// is uninitialized or has no handler for this method. Both map to graceful
/// degradation (empty result) rather than a tool error.
pub(crate) fn is_handler_absent(err: &NativeRequestError) -> bool {
    matches!(
        err,
        NativeRequestError::NotInitialized | NativeRequestError::UnregisteredHandler { .. }
    )
}
