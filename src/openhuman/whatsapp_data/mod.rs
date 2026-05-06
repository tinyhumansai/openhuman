//! Structured WhatsApp Web data — local-only SQLite persistence and agent API.
//!
//! This domain stores WhatsApp chats and messages scraped by the Tauri
//! `whatsapp_scanner` via CDP, making them queryable by the agent through
//! the JSON-RPC controller surface.
//!
//! **Data locality**: all data remains on-device in `whatsapp_data.db`; it is
//! never transmitted to any external service.
//!
//! ## RPC methods
//! - `openhuman.whatsapp_data_ingest` — scanner writes data here
//! - `openhuman.whatsapp_data_list_chats`
//! - `openhuman.whatsapp_data_list_messages`
//! - `openhuman.whatsapp_data_search_messages`

pub mod global;
pub mod ops;
pub mod rpc;
mod schemas;
pub mod store;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_whatsapp_data_controller_schemas,
    all_registered_controllers as all_whatsapp_data_registered_controllers,
};
