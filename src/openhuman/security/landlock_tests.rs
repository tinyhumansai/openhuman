use super::*;

#[cfg(all(feature = "sandbox-landlock", target_os = "linux"))]
#[test]
fn landlock_sandbox_name() {
    if let Ok(sandbox) = LandlockSandbox::new() {
        assert_eq!(sandbox.name(), "landlock");
    }
}

#[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
#[test]
fn landlock_not_available_on_non_linux() {
    assert!(!LandlockSandbox.is_available());
    assert_eq!(LandlockSandbox.name(), "landlock");
}

#[test]
fn landlock_with_none_workspace() {
    // Should work even without a workspace directory
    let result = LandlockSandbox::with_workspace(None);
    // Result depends on platform and feature flag
    match result {
        Ok(sandbox) => assert!(sandbox.is_available()),
        Err(_) => {
            // Stub, feature off, or Linux kernel without Landlock support
        }
    }
}

// ── §1.1 Landlock stub tests ──────────────────────────────

#[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
#[test]
fn landlock_stub_wrap_command_returns_unsupported() {
    let sandbox = LandlockSandbox;
    let mut cmd = std::process::Command::new("echo");
    let result = sandbox.wrap_command(&mut cmd);
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Unsupported);
}
