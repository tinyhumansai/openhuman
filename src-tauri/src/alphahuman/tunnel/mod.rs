mod cloudflare;
mod custom;
mod ngrok;
mod none;
mod tailscale;

pub use cloudflare::CloudflareTunnel;
pub use custom::CustomTunnel;
pub use ngrok::NgrokTunnel;
#[allow(unused_imports)]
pub use none::NoneTunnel;
pub use tailscale::TailscaleTunnel;

use crate::openhuman::config::{TailscaleTunnelConfig, TunnelConfig};
use anyhow::{bail, Result};
use std::sync::Arc;
use tokio::sync::Mutex;

// ── Tunnel trait ─────────────────────────────────────────────────

/// Agnostic tunnel abstraction — bring your own tunnel provider.
///
/// Implementations wrap an external tunnel binary (cloudflared, tailscale,
/// ngrok, etc.) or a custom command. The gateway calls `start()` after
/// binding its local port and `stop()` on shutdown.
#[async_trait::async_trait]
pub trait Tunnel: Send + Sync {
    /// Human-readable provider name (e.g. "cloudflare", "tailscale")
    fn name(&self) -> &str;

    /// Start the tunnel, exposing `local_host:local_port` externally.
    /// Returns the public URL on success.
    async fn start(&self, local_host: &str, local_port: u16) -> Result<String>;

    /// Stop the tunnel process gracefully.
    async fn stop(&self) -> Result<()>;

    /// Check if the tunnel is still alive.
    async fn health_check(&self) -> bool;

    /// Return the public URL if the tunnel is running.
    fn public_url(&self) -> Option<String>;
}

// ── Shared child-process handle ──────────────────────────────────

/// Wraps a spawned tunnel child process so implementations can share it.
pub(crate) struct TunnelProcess {
    pub child: tokio::process::Child,
    pub public_url: String,
}

pub(crate) type SharedProcess = Arc<Mutex<Option<TunnelProcess>>>;

pub(crate) fn new_shared_process() -> SharedProcess {
    Arc::new(Mutex::new(None))
}

/// Kill a shared tunnel process if running.
pub(crate) async fn kill_shared(proc: &SharedProcess) -> Result<()> {
    let mut guard = proc.lock().await;
    if let Some(ref mut tp) = *guard {
        tp.child.kill().await.ok();
        tp.child.wait().await.ok();
    }
    *guard = None;
    Ok(())
}

// ── Factory ──────────────────────────────────────────────────────

/// Create a tunnel from config. Returns `None` for provider "none".
pub fn create_tunnel(config: &TunnelConfig) -> Result<Option<Box<dyn Tunnel>>> {
    match config.provider.as_str() {
        "none" | "" => Ok(None),

        "cloudflare" => {
            let cf = config.cloudflare.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "tunnel.provider = \"cloudflare\" but [tunnel.cloudflare] section is missing"
                )
            })?;
            Ok(Some(Box::new(CloudflareTunnel::new(cf.token.clone()))))
        }

        "tailscale" => {
            let ts = config.tailscale.as_ref().unwrap_or(&TailscaleTunnelConfig {
                funnel: false,
                hostname: None,
            });
            Ok(Some(Box::new(TailscaleTunnel::new(
                ts.funnel,
                ts.hostname.clone(),
            ))))
        }

        "ngrok" => {
            let ng = config.ngrok.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "tunnel.provider = \"ngrok\" but [tunnel.ngrok] section is missing"
                )
            })?;
            Ok(Some(Box::new(NgrokTunnel::new(
                ng.auth_token.clone(),
                ng.domain.clone(),
            ))))
        }

        "custom" => {
            let cu = config.custom.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "tunnel.provider = \"custom\" but [tunnel.custom] section is missing"
                )
            })?;
            Ok(Some(Box::new(CustomTunnel::new(
                cu.start_command.clone(),
                cu.health_url.clone(),
                cu.url_pattern.clone(),
            ))))
        }

        other => bail!(
            "Unknown tunnel provider: \"{other}\". Valid: none, cloudflare, tailscale, ngrok, custom"
        ),
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::{
        CloudflareTunnelConfig, CustomTunnelConfig, NgrokTunnelConfig,
    };

    /// Helper: assert `create_tunnel` returns an error containing `needle`.
    fn assert_tunnel_err(cfg: &TunnelConfig, needle: &str) {
        match create_tunnel(cfg) {
            Err(e) => assert!(
                e.to_string().contains(needle),
                "Expected error containing \"{needle}\", got: {e}"
            ),
            Ok(_) => panic!("Expected error containing \"{needle}\", but got Ok"),
        }
    }

    #[test]
    fn factory_none_returns_none() {
        let cfg = TunnelConfig::default();
        let t = create_tunnel(&cfg).unwrap();
        assert!(t.is_none());
    }

    #[test]
    fn factory_empty_string_returns_none() {
        let cfg = TunnelConfig {
            provider: String::new(),
            ..TunnelConfig::default()
        };
        let t = create_tunnel(&cfg).unwrap();
        assert!(t.is_none());
    }

    #[test]
    fn factory_unknown_provider_errors() {
        let cfg = TunnelConfig {
            provider: "wireguard".into(),
            ..TunnelConfig::default()
        };
        assert_tunnel_err(&cfg, "Unknown tunnel provider");
    }

    #[test]
    fn factory_cloudflare_missing_config_errors() {
        let cfg = TunnelConfig {
            provider: "cloudflare".into(),
            ..TunnelConfig::default()
        };
        assert_tunnel_err(&cfg, "[tunnel.cloudflare]");
    }

    #[test]
    fn factory_cloudflare_with_config_ok() {
        let cfg = TunnelConfig {
            provider: "cloudflare".into(),
            cloudflare: Some(CloudflareTunnelConfig {
                token: "test-token".into(),
            }),
            ..TunnelConfig::default()
        };
        let t = create_tunnel(&cfg).unwrap();
        assert!(t.is_some());
        assert_eq!(t.unwrap().name(), "cloudflare");
    }

    #[test]
    fn factory_tailscale_defaults_ok() {
        let cfg = TunnelConfig {
            provider: "tailscale".into(),
            ..TunnelConfig::default()
        };
        let t = create_tunnel(&cfg).unwrap();
        assert!(t.is_some());
        assert_eq!(t.unwrap().name(), "tailscale");
    }

    #[test]
    fn factory_ngrok_missing_config_errors() {
        let cfg = TunnelConfig {
            provider: "ngrok".into(),
            ..TunnelConfig::default()
        };
        assert_tunnel_err(&cfg, "[tunnel.ngrok]");
    }

    #[test]
    fn factory_ngrok_with_config_ok() {
        let cfg = TunnelConfig {
            provider: "ngrok".into(),
            ngrok: Some(NgrokTunnelConfig {
                auth_token: "tok".into(),
                domain: None,
            }),
            ..TunnelConfig::default()
        };
        let t = create_tunnel(&cfg).unwrap();
        assert!(t.is_some());
        assert_eq!(t.unwrap().name(), "ngrok");
    }

    #[test]
    fn factory_custom_missing_config_errors() {
        let cfg = TunnelConfig {
            provider: "custom".into(),
            ..TunnelConfig::default()
        };
        assert_tunnel_err(&cfg, "[tunnel.custom]");
    }

    #[test]
    fn factory_custom_with_config_ok() {
        let cfg = TunnelConfig {
            provider: "custom".into(),
            custom: Some(CustomTunnelConfig {
                start_command: "echo tunnel".into(),
                health_url: None,
                url_pattern: None,
            }),
            ..TunnelConfig::default()
        };
        let t = create_tunnel(&cfg).unwrap();
        assert!(t.is_some());
        assert_eq!(t.unwrap().name(), "custom");
    }
}
