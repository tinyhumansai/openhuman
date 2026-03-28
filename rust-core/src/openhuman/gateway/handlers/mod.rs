//! Gateway HTTP handlers grouped by concern.

mod health;
mod linq;
mod pair;
mod webhook;
mod whatsapp;

#[cfg(test)]
pub use health::PROMETHEUS_CONTENT_TYPE;
pub use health::{handle_health, handle_metrics};
pub use linq::handle_linq_webhook;
pub use pair::handle_pair;
#[cfg(test)]
pub use pair::persist_pairing_tokens;
pub use webhook::handle_webhook;
#[cfg(test)]
pub use whatsapp::verify_whatsapp_signature;
pub use whatsapp::{handle_whatsapp_message, handle_whatsapp_verify};
