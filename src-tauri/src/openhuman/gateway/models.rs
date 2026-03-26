//! Gateway request models.

use serde::Deserialize;

/// POST `/webhook` request body.
#[derive(Debug, Deserialize)]
pub struct WebhookBody {
    pub message: Option<String>,
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub memory: Option<bool>,
}

/// GET `/whatsapp` verification query parameters.
#[derive(Debug, Deserialize)]
pub struct WhatsAppVerifyQuery {
    #[serde(rename = "hub.mode")]
    pub mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    pub verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    pub challenge: Option<String>,
}
