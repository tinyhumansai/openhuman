//! Host calls to the attested tinycomputer module.

use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

use serde::Serialize;
use tinybus::Proxy;
use tinycomputer_bus::{names, DesktopResponse, PermissionsRequest};

use crate::config::Config;

#[cfg(test)]
#[path = "desktop_tests.rs"]
mod tests;

pub const MODULE_ID: &str = "tinycomputer";

pub use super::computer_config::{billing_route, module_config};

pub async fn jev_ready(config: &Config) -> bool {
    module_config(config).await["jev"].is_object()
}

fn last_config() -> &'static tokio::sync::Mutex<Option<u64>> {
    static LAST: OnceLock<tokio::sync::Mutex<Option<u64>>> = OnceLock::new();
    LAST.get_or_init(|| tokio::sync::Mutex::new(None))
}

fn fingerprint(value: &serde_json::Value) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

/// Load the module, refresh its private configuration when it changed, and
/// return a proxy for its one interface (desktop, browser and task members).
pub(crate) async fn proxy(config: &Config) -> Result<Proxy, String> {
    crate::modules::ops::ensure_loaded_within(
        config,
        MODULE_ID,
        Some(std::time::Duration::from_secs(8)),
    )
    .await
    .map_err(crate::modules::ops::load_error_message)?;
    let runtime = crate::modules::runtime::runtime()
        .await
        .map_err(|error| format!("TinyComputer module bus unavailable: {error}"))?;
    crate::modules::registry::find(MODULE_ID)
        .ok_or_else(|| "TinyComputer module is not in the compiled registry".to_owned())?;

    // Reinitialization is private and retains no plaintext secret in this host.
    // The lock makes credential rotation atomic across concurrent tool calls.
    let configuration = module_config(config).await;
    #[cfg(feature = "security-module")]
    let mut configuration = configuration;
    #[cfg(feature = "security-module")]
    {
        let callback_destination = crate::modules::ops::consequence_callback_destination(MODULE_ID)
            .await
            .ok_or_else(|| "TinyComputer host callback name unavailable".to_owned())?;
        let egress = match super::browser_egress_host::browser_dial_config(config).await {
            Ok(egress) => egress,
            Err(error) => {
                tracing::warn!(%error, "[computer] browser egress policy unavailable");
                return Err("TinySecurity browser egress policy unavailable".into());
            }
        };
        inject_security_browser_config(&mut configuration, egress, &callback_destination);
    }
    let current = fingerprint(&configuration);
    let mut previous = last_config().lock().await;
    if *previous != Some(current) {
        runtime
            .connection()
            .reinitialize_module(MODULE_ID, configuration)
            .await
            .map_err(|error| {
                let message = format!("TinyComputer configuration refresh failed: {error}");
                if error.wire_name().ends_with("ModuleUnavailable") {
                    // The module refused its configuration and faulted; it
                    // stays unusable in this process, so report it that way.
                    tracing::warn!(%message, "[computer] module faulted on reinitialization");
                    tinybus::module::resolution::global().mark_faulted(MODULE_ID, message.clone());
                }
                message
            })?;
    }
    *previous = Some(current);
    drop(previous);
    runtime
        .proxy(names::INTERFACE, names::OBJECT_PATH)
        .map_err(|error| format!("TinyComputer module proxy unavailable: {error}"))
}

fn inject_security_browser_config(
    configuration: &mut serde_json::Value,
    egress: serde_json::Value,
    callback_destination: &str,
) {
    configuration["browser"]["egress"] = egress;
    configuration["consequence_host"] = serde_json::json!({ "destination": callback_destination });
}

/// Call one contract member, preserving the structured error envelope.
pub async fn call<Request: Serialize + Send>(
    config: &Config,
    member: &str,
    request: Request,
) -> Result<DesktopResponse, String> {
    let proxy = proxy(config).await?;
    call_with_proxy(&proxy, member, request).await
}

async fn call_with_proxy<Request: Serialize + Send>(
    proxy: &Proxy,
    member: &str,
    request: Request,
) -> Result<DesktopResponse, String> {
    // RunGoal enforces its own shorter elapsed-time budget (at most five
    // minutes). Keep the bus deadline beyond that budget so the caller gets a
    // structured stop and partial execution evidence instead of a transport
    // timeout after TinyBus's default 30 seconds.
    let goal_proxy = (member == names::methods::RUN_GOAL).then(|| {
        proxy
            .clone()
            .with_timeout(std::time::Duration::from_secs(330))
    });
    let proxy = goal_proxy.as_ref().unwrap_or(proxy);
    // The contract's catalogue marks which members carry facts, credentials
    // or page data; those travel confidentially.
    let confidential =
        tinycomputer_bus::catalogue::member(member).is_some_and(|entry| entry.confidential);
    let response = if confidential {
        proxy
            .call_confidential::<DesktopResponse>(member, (request,))
            .await
    } else {
        proxy.call::<DesktopResponse>(member, (request,)).await
    };
    response.map_err(|error| format!("desktop {member} failed: {error}"))
}

pub async fn permissions(config: &Config) -> Result<DesktopResponse, String> {
    call(
        config,
        names::methods::PERMISSIONS,
        PermissionsRequest::default(),
    )
    .await
}

pub fn state(config: &Config) -> (String, Option<String>) {
    crate::modules::ops::list(config)
        .into_iter()
        .find(|item| item.id == MODULE_ID)
        .map_or_else(
            || {
                (
                    "failed".to_owned(),
                    Some("TinyComputer module is not registered".to_owned()),
                )
            },
            |item| (format!("{:?}", item.state).to_lowercase(), item.detail),
        )
}
