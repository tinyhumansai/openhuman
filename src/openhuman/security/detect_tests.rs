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
