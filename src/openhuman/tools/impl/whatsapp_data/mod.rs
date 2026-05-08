//! LLM-callable wrappers for the local WhatsApp data store (issue #1341).
//!
//! Each tool is a thin shim over one of the read-only RPC handlers in
//! [`crate::openhuman::whatsapp_data::rpc`], unwrapping the `RpcOutcome`
//! envelope and emitting a compact JSON object that includes a
//! `"provider": "whatsapp"` provenance tag. The agent can then cite
//! WhatsApp as the source without depending on field-level guessing.
//!
//! The write-path controller `whatsapp_data_ingest` is intentionally
//! NOT wrapped here — it is registered as an internal-only controller
//! in `src/core/all.rs` (the scanner is the only legitimate caller).
//! Adding a Tool impl for it would reopen the read-only boundary that
//! this module exists to preserve, so the omission is load-bearing.

mod list_chats;
mod list_messages;
mod search_messages;

pub use list_chats::WhatsAppDataListChatsTool;
pub use list_messages::WhatsAppDataListMessagesTool;
pub use search_messages::WhatsAppDataSearchMessagesTool;
