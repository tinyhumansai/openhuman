//! Jev browser tasks, billed through the configured agentic route.
//!
//! The controller links only the browser bus contract. `BrowserClient` supplies
//! its browser port, while the same credential routing as other agentic work
//! chooses direct OpenRouter or the hosted TinyHumans System One proxy.

use std::time::Duration;

use tinybrowser_bus::SessionId;
use tinybrowser_control::{BrowserControl, ControlLimits, JevController, TaskRequest, TaskResult};
use tinyjevclient::{Client, ClientConfig};

use crate::api::config::effective_backend_api_url;
use crate::config::Config;
use crate::inference::provider::factory::{lookup_key_for_slug, provider_for_role};
use crate::security::credentials::session_support::direct_backend_credential;

#[cfg(test)]
#[path = "browser_task_tests.rs"]
mod tests;

/// The account charged for Jev decisions in a browser task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BillingRoute {
    /// The user's configured OpenRouter credential.
    DirectOpenRouter,
    /// OpenHuman's hosted System One proxy and managed credits.
    Hosted,
}

/// Resolve the browser decision route from the configured agentic provider.
#[must_use]
pub fn billing_route(config: &Config) -> BillingRoute {
    let agentic = provider_for_role("agentic", config);
    if agentic.starts_with("openrouter:") {
        BillingRoute::DirectOpenRouter
    } else {
        BillingRoute::Hosted
    }
}

fn jev_client(config: &Config) -> Result<Client, String> {
    let client_config = match billing_route(config) {
        BillingRoute::DirectOpenRouter => {
            let key = lookup_key_for_slug("openrouter", config)
                .map_err(|_| "OpenRouter agentic credential is unavailable".to_owned())?;
            if key.trim().is_empty() {
                return Err("OpenRouter agentic credential is unavailable".to_owned());
            }
            ClientConfig::openrouter(key)
        }
        BillingRoute::Hosted => {
            // The hosted route reaches the backend host directly (not through
            // the transport port): without a TinyHumans connection or a usable
            // credential there is nothing to call, so refuse before any request.
            let credential = direct_backend_credential(config, "browser task (hosted jev)")
                .ok_or_else(|| "TinyHumans credential is unavailable".to_owned())?;
            let mut client_config = ClientConfig::tinyhumans_openrouter(credential.into_secret());
            client_config.base_url = effective_backend_api_url(&config.api_url);
            client_config
        }
    };
    Client::new(client_config).map_err(|_| "Jev client configuration is invalid".to_owned())
}

/// Run a bounded task in an existing browser session.
///
/// A consequential action returns `NeedsConfirmation` with an exact pending
/// decision. The caller must obtain host confirmation before performing it.
///
/// # Errors
///
/// Returns a credential, provider, browser, or timeout error without exposing
/// credentials or page content in the message.
pub async fn run(
    browser: &impl BrowserControl,
    config: &Config,
    session: &SessionId,
    task: TaskRequest,
    max_steps: usize,
) -> Result<TaskResult, String> {
    let limits = ControlLimits {
        max_steps: max_steps.min(config.browser.max_task_steps),
        ..ControlLimits::default()
    };
    let controller = JevController::new(jev_client(config)?).with_limits(limits);
    let deadline = Duration::from_secs(config.browser.task_timeout_secs);
    tracing::debug!(route = ?billing_route(config), max_steps = limits.max_steps, "[browser-task] starting");
    tokio::time::timeout(deadline, controller.run(browser, session, &task))
        .await
        .map_err(|_| "browser task timed out".to_owned())?
        .map_err(|error| match error {
            tinybrowser_control::Error::InvalidTask { .. } => "invalid browser task".to_owned(),
            tinybrowser_control::Error::InvalidDecision { .. } => {
                "Jev returned an unusable browser decision".to_owned()
            }
            tinybrowser_control::Error::Provider { .. } => "Jev decision request failed".to_owned(),
            tinybrowser_control::Error::Browser { source } => {
                format!("browser operation failed: {}", source.wire_name())
            }
        })
}
