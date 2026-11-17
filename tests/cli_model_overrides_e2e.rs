//! End-to-end coverage for transient model/provider selection on the CLI.

use std::process::Command;

use serde_json::Value;

#[test]
fn cli_model_and_provider_flags_override_the_loaded_session_without_persisting() {
    let workspace = tempfile::tempdir().expect("temporary OpenHuman workspace");
    let baseline = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args(["inference", "get_client_config"])
        .env("OPENHUMAN_WORKSPACE", workspace.path())
        .output()
        .expect("read the workspace's baseline inference config");
    assert!(
        baseline.status.success(),
        "baseline CLI failed: {}",
        String::from_utf8_lossy(&baseline.stderr)
    );
    let baseline: Value = serde_json::from_slice(&baseline.stdout).expect("baseline JSON response");
    let baseline_result = baseline.get("result").expect("baseline RPC result");

    let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args([
            "--provider",
            "ollama",
            "--model",
            "qwen3:8b",
            "inference",
            "get_client_config",
        ])
        .env("OPENHUMAN_WORKSPACE", workspace.path())
        .output()
        .expect("run OpenHuman CLI");

    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: Value = serde_json::from_slice(&output.stdout).expect("JSON CLI response");
    let result = response.get("result").expect("RPC result");
    for field in [
        "chat_provider",
        "reasoning_provider",
        "agentic_provider",
        "coding_provider",
    ] {
        assert_eq!(
            result.get(field).and_then(Value::as_str),
            Some("ollama:qwen3:8b"),
            "{field} did not receive the CLI override"
        );
    }
    assert_eq!(
        result.get("default_model"),
        baseline_result.get("default_model"),
        "a local route override must not replace the workspace's default model"
    );

    let persisted =
        std::fs::read_to_string(workspace.path().join("config.toml")).expect("persisted config");
    assert!(
        !persisted.contains("qwen3:8b"),
        "transient CLI model leaked into config.toml"
    );
}

#[test]
fn a_mutating_cli_command_does_not_persist_launch_overrides() {
    let workspace = tempfile::tempdir().expect("temporary OpenHuman workspace");
    let initialize = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args(["config", "get"])
        .env("OPENHUMAN_WORKSPACE", workspace.path())
        .output()
        .expect("initialize OpenHuman config");
    assert!(initialize.status.success());

    let config_path = workspace.path().join("config.toml");
    let before = std::fs::read_to_string(&config_path).expect("initial config");
    let mutate = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args([
            "--provider",
            "ollama",
            "--model",
            "qwen3:8b",
            "config",
            "set_onboarding_completed",
            "--value",
            "true",
        ])
        .env("OPENHUMAN_WORKSPACE", workspace.path())
        .output()
        .expect("run mutating OpenHuman command");
    assert!(
        mutate.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&mutate.stderr)
    );

    let after = std::fs::read_to_string(config_path).expect("mutated config");
    assert!(after.contains("onboarding_completed = true"));
    assert!(!after.contains("qwen3:8b"));
    for field in [
        "default_model",
        "chat_provider",
        "reasoning_provider",
        "agentic_provider",
        "coding_provider",
    ] {
        assert_eq!(
            toml_field(&after, field),
            toml_field(&before, field),
            "{field} was changed by a transient launch override"
        );
    }
}

fn toml_field<'a>(document: &'a str, field: &str) -> Option<&'a str> {
    document.lines().find(|line| {
        line.split_once('=')
            .is_some_and(|(key, _)| key.trim() == field)
    })
}

#[test]
fn cli_rejects_a_missing_model_value() {
    let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .arg("--model")
        .output()
        .expect("run OpenHuman CLI");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing value for --model"));
}

#[test]
fn core_boot_accepts_a_configured_master_key_from_either_source() {
    let key = "ab".repeat(32);
    let file_dir = tempfile::tempdir().expect("master-key directory");
    let key_path = file_dir.path().join("master.key");
    std::fs::write(&key_path, format!("{key}\n")).expect("write master key");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .expect("restrict master key");
    }

    for (name, value) in [
        ("OPENHUMAN_KEYRING_MASTER_KEY", key.as_str()),
        (
            "OPENHUMAN_KEYRING_MASTER_KEY_FILE",
            key_path.to_str().expect("key path is UTF-8"),
        ),
    ] {
        let workspace = tempfile::tempdir().expect("workspace");
        let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
            .args(["config", "get"])
            .env("OPENHUMAN_APP_ENV", "staging")
            .env("OPENHUMAN_WORKSPACE", workspace.path())
            .env("OPENHUMAN_KEYRING_MASTER_KEY", "")
            .env("OPENHUMAN_KEYRING_MASTER_KEY_FILE", "")
            .env(name, value)
            .output()
            .expect("run OpenHuman core");
        assert!(
            output.status.success(),
            "{name} boot failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn core_boot_rejects_a_malformed_configured_master_key() {
    let workspace = tempfile::tempdir().expect("workspace");
    let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args(["config", "get"])
        .env("OPENHUMAN_APP_ENV", "staging")
        .env("OPENHUMAN_WORKSPACE", workspace.path())
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "not-a-key")
        .output()
        .expect("run OpenHuman core");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("OPENHUMAN_KEYRING_MASTER_KEY"), "{stderr}");
    assert!(!stderr.contains("not-a-key"), "{stderr}");
}
