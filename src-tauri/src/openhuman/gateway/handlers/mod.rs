//! Gateway HTTP handlers grouped by concern.

mod health;
mod linq;
mod pair;
mod webhook;
mod whatsapp;

pub use health::{handle_health, handle_metrics, PROMETHEUS_CONTENT_TYPE};
pub use linq::handle_linq_webhook;
pub use pair::{handle_pair, persist_pairing_tokens};
pub use webhook::handle_webhook;
pub use whatsapp::{handle_whatsapp_message, handle_whatsapp_verify, verify_whatsapp_signature};
