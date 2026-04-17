//! Auto-detection of available security features

use crate::openhuman::config::{SandboxBackend, SecurityConfig};
use crate::openhuman::security::traits::Sandbox;
use std::sync::Arc;

/// Create a sandbox based on auto-detection or explicit config
pub fn create_sandbox(config: &SecurityConfig) -> Arc<dyn Sandbox> {
    let backend = &config.sandbox.backend;

    // If explicitly disabled, return noop
    if matches!(backend, SandboxBackend::None) || config.sandbox.enabled == Some(false) {
        return Arc::new(super::traits::NoopSandbox);
    }

    // If specific backend requested, try that
    match backend {
        SandboxBackend::Landlock => {
            #[cfg(feature = "sandbox-landlock")]
            {
                if std::env::consts::OS == "linux" {
                    if let Ok(sandbox) = super::landlock::LandlockSandbox::new() {
                        return Arc::new(sandbox);
                    }
                }
            }
            log::warn!("Landlock requested but not available, falling back to application-layer");
            Arc::new(super::traits::NoopSandbox)
        }
        SandboxBackend::Firejail => {
            if std::env::consts::OS == "linux" {
                if let Ok(sandbox) = super::firejail::FirejailSandbox::new() {
                    return Arc::new(sandbox);
                }
            }
            log::warn!("Firejail requested but not available, falling back to application-layer");
            Arc::new(super::traits::NoopSandbox)
        }
        SandboxBackend::Bubblewrap => {
            #[cfg(feature = "sandbox-bubblewrap")]
            {
                if matches!(std::env::consts::OS, "linux" | "macos") {
                    if let Ok(sandbox) = super::bubblewrap::BubblewrapSandbox::new() {
                        return Arc::new(sandbox);
                    }
                }
            }
            log::warn!("Bubblewrap requested but not available, falling back to application-layer");
            Arc::new(super::traits::NoopSandbox)
        }
        SandboxBackend::Docker => {
            if let Ok(sandbox) = super::docker::DockerSandbox::new() {
                return Arc::new(sandbox);
            }
            log::warn!("Docker requested but not available, falling back to application-layer");
            Arc::new(super::traits::NoopSandbox)
        }
        SandboxBackend::Auto | SandboxBackend::None => {
            // Auto-detect best available
            detect_best_sandbox()
        }
    }
}

/// Auto-detect the best available sandbox
fn detect_best_sandbox() -> Arc<dyn Sandbox> {
    if std::env::consts::OS == "linux" {
        // Try Landlock first (native, no dependencies)
        #[cfg(feature = "sandbox-landlock")]
        {
            if let Ok(sandbox) = super::landlock::LandlockSandbox::probe() {
                log::info!("Landlock sandbox enabled (Linux kernel 5.13+)");
                return Arc::new(sandbox);
            }
        }

        // Try Firejail second (user-space tool)
        if let Ok(sandbox) = super::firejail::FirejailSandbox::probe() {
            log::info!("Firejail sandbox enabled");
            return Arc::new(sandbox);
        }
    }

    if std::env::consts::OS == "macos" {
        // Try Bubblewrap on macOS
        #[cfg(feature = "sandbox-bubblewrap")]
        {
            if let Ok(sandbox) = super::bubblewrap::BubblewrapSandbox::probe() {
                log::info!("Bubblewrap sandbox enabled");
                return Arc::new(sandbox);
            }
        }
    }

    // Docker is heavy but works everywhere if docker is installed
    if let Ok(sandbox) = super::docker::DockerSandbox::probe() {
        log::info!("Docker sandbox enabled");
        return Arc::new(sandbox);
    }

    // Fallback: application-layer security only
    log::info!("No sandbox backend available, using application-layer security");
    Arc::new(super::traits::NoopSandbox)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::{SandboxConfig, SecurityConfig};

    #[test]
    fn detect_best_sandbox_returns_something() {
        let sandbox = detect_best_sandbox();
        // Should always return at least NoopSandbox
        assert!(sandbox.is_available());
    }

    #[test]
    fn explicit_none_returns_noop() {
        let config = SecurityConfig {
            sandbox: SandboxConfig {
                enabled: Some(false),
                backend: SandboxBackend::None,
                firejail_args: Vec::new(),
            },
            ..SecurityConfig::default()
        };
        let sandbox = create_sandbox(&config);
        assert_eq!(sandbox.name(), "none");
    }

    #[test]
    fn auto_mode_detects_something() {
        let config = SecurityConfig {
            sandbox: SandboxConfig {
                enabled: None, // Auto-detect
                backend: SandboxBackend::Auto,
                firejail_args: Vec::new(),
            },
            ..SecurityConfig::default()
        };
        let sandbox = create_sandbox(&config);
        // Should return some sandbox (at least NoopSandbox)
        assert!(sandbox.is_available());
    }

    #[test]
    fn disabled_via_enabled_false_returns_noop() {
        let config = SecurityConfig {
            sandbox: SandboxConfig {
                enabled: Some(false),
                backend: SandboxBackend::Auto,
                firejail_args: Vec::new(),
            },
            ..SecurityConfig::default()
        };
        let sandbox = create_sandbox(&config);
        assert_eq!(sandbox.name(), "none");
    }

    #[test]
    fn landlock_backend_on_non_linux_falls_back() {
        // On macOS/Windows, Landlock isn't available — should fall back to Noop
        let config = SecurityConfig {
            sandbox: SandboxConfig {
                enabled: None,
                backend: SandboxBackend::Landlock,
                firejail_args: Vec::new(),
            },
            ..SecurityConfig::default()
        };
        let sandbox = create_sandbox(&config);
        if std::env::consts::OS != "linux" {
            assert_eq!(sandbox.name(), "none");
        }
    }

    #[test]
    fn firejail_backend_on_non_linux_falls_back() {
        let config = SecurityConfig {
            sandbox: SandboxConfig {
                enabled: None,
                backend: SandboxBackend::Firejail,
                firejail_args: Vec::new(),
            },
            ..SecurityConfig::default()
        };
        let sandbox = create_sandbox(&config);
        if std::env::consts::OS != "linux" {
            assert_eq!(sandbox.name(), "none");
        }
    }

    #[test]
    fn bubblewrap_backend_falls_back_when_unavailable() {
        let config = SecurityConfig {
            sandbox: SandboxConfig {
                enabled: None,
                backend: SandboxBackend::Bubblewrap,
                firejail_args: Vec::new(),
            },
            ..SecurityConfig::default()
        };
        let sandbox = create_sandbox(&config);
        // Bubblewrap probably isn't installed on CI/dev — expect fallback
        assert!(sandbox.is_available());
    }

    #[test]
    fn docker_backend_falls_back_when_unavailable() {
        let config = SecurityConfig {
            sandbox: SandboxConfig {
                enabled: None,
                backend: SandboxBackend::Docker,
                firejail_args: Vec::new(),
            },
            ..SecurityConfig::default()
        };
        let sandbox = create_sandbox(&config);
        // Docker may or may not be available
        assert!(sandbox.is_available());
    }
}
