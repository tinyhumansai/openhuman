//! Shared gateway state for axum handlers.

use crate::alphahuman::channels::{LinqChannel, WhatsAppChannel};
use crate::alphahuman::config::Config;
use crate::alphahuman::memory::Memory;
use crate::alphahuman::providers::Provider;
use crate::alphahuman::security::pairing::PairingGuard;
use crate::alphahuman::gateway::rate_limit::{GatewayRateLimiter, IdempotencyStore};
use parking_lot::Mutex;
use std::sync::Arc;

/// Shared state for all axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Mutex<Config>>,
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub temperature: f64,
    pub mem: Arc<dyn Memory>,
    pub auto_save: bool,
    /// SHA-256 hash of `X-Webhook-Secret` (hex-encoded), never plaintext.
    pub webhook_secret_hash: Option<Arc<str>>,
    pub pairing: Arc<PairingGuard>,
    pub trust_forwarded_headers: bool,
    pub rate_limiter: Arc<GatewayRateLimiter>,
    pub idempotency_store: Arc<IdempotencyStore>,
    pub whatsapp: Option<Arc<WhatsAppChannel>>,
    /// `WhatsApp` app secret for webhook signature verification (`X-Hub-Signature-256`).
    pub whatsapp_app_secret: Option<Arc<str>>,
    pub linq: Option<Arc<LinqChannel>>,
    /// Linq webhook signing secret for signature verification.
    pub linq_signing_secret: Option<Arc<str>>,
    /// Observability backend for metrics scraping.
    pub observer: Arc<dyn crate::alphahuman::observability::Observer>,
}
