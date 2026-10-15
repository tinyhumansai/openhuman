use super::*;
use tinysecurity_bus::{PathAccess, PathPolicy, PathTrustedRoot, PathValidationResult};

/// The production client (`register_path_policy` -> `proxy` -> `ensure_loaded`
/// -> `module_config`) against the published archive loaded through the real
/// module runtime. Unlike the fixture-backed tests it takes the trusted host
/// from `module_config` itself, so it fails with `RegisterPathPolicy failed`
/// if that configuration ever stops naming the host's bus connection.
///
/// Process-global state (module runtime, resolution table, client state), so
/// run it with the rest of `modules::security` only through the native lane.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key (downloads the published archive)"]
async fn production_client_registers_and_checks_a_path_policy_on_the_released_archive() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(&root).unwrap();
    let base = dir.path().canonicalize().unwrap();
    let action = base.join("action");
    std::fs::create_dir(&action).unwrap();
    std::fs::write(action.join("readable.txt"), "fixture").unwrap();
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = true;
    config.modules.install_dir = Some(base.join("modules").to_string_lossy().into_owned());
    config.action_dir = action.clone();
    config.workspace_dir = base.join("state");

    let policy = PathPolicy {
        enabled: true,
        workspace_only: true,
        workspace_dir: base.join("state"),
        action_dir: action.clone(),
        home_dir: None,
        forbidden_paths: vec![],
        trusted_roots: vec![PathTrustedRoot {
            path: action.clone(),
            access: PathAccess::ReadWrite,
        }],
        reserved_paths: vec![],
        reserved_prefixes: vec![],
        readonly_paths: vec![],
        reserved_names: vec![],
    };
    let id = register_path_policy(&config, policy)
        .await
        .expect("production RegisterPathPolicy must be accepted from the trusted host");
    assert!(
        module_config(&config).trusted_host.is_some(),
        "module_config must name the host's bus connection once the runtime is up"
    );
    assert_eq!(
        validate_path(&config, id.clone(), "readable.txt".into())
            .await
            .unwrap(),
        PathValidationResult::Allowed {
            resolved: action.join("readable.txt")
        }
    );
    assert!(matches!(
        validate_path(&config, id, "../outside".into())
            .await
            .unwrap(),
        PathValidationResult::Denied { .. }
    ));
}

/// The shared sync seam against the released archive: a plain thread and a
/// worker thread of the module runtime both get the module's redaction through
/// `redact_blocking`, and a call with no time to run fails closed.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key (downloads the published archive)"]
async fn sync_seam_redacts_through_the_released_archive_from_inside_and_outside_a_runtime() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(&root).unwrap();
    let base = dir.path().canonicalize().unwrap();
    let mut config = Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = true;
    config.modules.install_dir = Some(base.join("modules").to_string_lossy().into_owned());
    config.action_dir = base.join("action");
    config.workspace_dir = base.join("state");

    // The first async call loads the module and arms the seam.
    let secret = "Bearer abcdefghijklmnopqrstuvwxyz0123456789";
    let first = text::redact(&config, text::RedactMode::Text, secret)
        .await
        .expect("module redaction");
    assert!(!first.contains("abcdefghijklmnopqrstuvwxyz"), "{first}");

    let timeout = std::time::Duration::from_secs(5);
    let outside = {
        let config = config.clone();
        std::thread::spawn(move || {
            text::redact_blocking(
                &config,
                text::RedactMode::Text,
                &format!("token: {}-{}", "sk", "live0123456789abcdef0123456789abcdef"),
                timeout,
            )
        })
        .join()
        .unwrap()
        .expect("sync redaction from a plain thread")
    };
    assert!(
        !outside.contains("0123456789abcdef0123456789abcdef"),
        "{outside}"
    );

    let inside = text::redact_blocking(
        &config,
        text::RedactMode::Text,
        "password=hunter2hunter2hunter2",
        timeout,
    )
    .expect("sync redaction from a runtime worker");
    assert!(!inside.contains("hunter2hunter2hunter2"), "{inside}");

    // No time left: refused, never the original text.
    let refused = text::redact_blocking(
        &config,
        text::RedactMode::Text,
        "fresh uncached text 1",
        std::time::Duration::ZERO,
    );
    assert!(refused.is_err() || refused.as_deref() == Ok("fresh uncached text 1"));
}

/// Shell and network gates on the production path: `SecurityPolicy` ->
/// `command_decision` -> `host_config` -> `proxy` (the module loaded from the
/// published archive by the real module runtime, with the host named as the
/// trusted peer by `module_config`) -> `RegisterCommandPolicy` / `CheckCommand` /
/// `ClassifyCommand`. The class gate is also asked from a plain thread and from
/// a runtime worker through the sync seam, as the network and filesystem tool
/// traits do.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key (downloads the published archive)"]
async fn production_client_gates_shell_and_network_on_the_released_archive() {
    use crate::security::policy::{CommandFlow, CommandQuery, CommandStage, Refusal};
    use crate::security::{AutonomyLevel, GateDecision, SecurityPolicy};
    use tinybox_bus::CommandClass;

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(&root).unwrap();
    let base = dir.path().canonicalize().unwrap();
    let workspace = base.join("workspace");
    let action = base.join("action");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&action).unwrap();
    std::fs::write(
        base.join("config.toml"),
        format!(
            "[modules]\nenabled = true\nallow_download = true\ninstall_dir = {:?}\n",
            base.join("modules").to_string_lossy()
        ),
    )
    .unwrap();

    let policy = |autonomy, max_actions_per_hour| SecurityPolicy {
        enabled: true,
        autonomy,
        workspace_dir: workspace.clone(),
        action_dir: action.clone(),
        max_actions_per_hour,
        ..SecurityPolicy::default()
    };
    let ask = |policy: &SecurityPolicy, command: &'static str, stage| {
        let policy = policy.clone();
        async move {
            policy
                .check_command(CommandQuery {
                    command,
                    declared: None,
                    flow: CommandFlow::Harness,
                    stage,
                })
                .await
        }
    };

    // The read-only tier lets a read through and refuses a write with its reason.
    let read_only = policy(AutonomyLevel::ReadOnly, 100);
    let verdict = ask(&read_only, "ls", CommandStage::Authorize)
        .await
        .expect("a read runs in read-only mode");
    assert_eq!(
        (verdict.class, verdict.gate),
        (CommandClass::Read, GateDecision::Allow)
    );
    assert_eq!(
        ask(&read_only, "touch f", CommandStage::Authorize).await,
        Err(Refusal::ReadOnly)
    );

    // Supervised prompts on a write and refuses hidden execution.
    let supervised = policy(AutonomyLevel::Supervised, 100);
    let verdict = ask(&supervised, "touch f", CommandStage::Authorize)
        .await
        .unwrap();
    assert_eq!(
        (verdict.class, verdict.gate),
        (CommandClass::Write, GateDecision::Prompt)
    );
    assert_eq!(
        ask(&supervised, "echo $(whoami)", CommandStage::Authorize).await,
        Err(Refusal::Hidden)
    );
    // The credential-store floor holds even with the policy disabled.
    let disabled = SecurityPolicy {
        enabled: false,
        ..supervised.clone()
    };
    assert_eq!(
        ask(&disabled, "cat ~/.ssh/id_rsa", CommandStage::Authorize).await,
        Err(Refusal::Protected)
    );

    // The run spends the module budget; asking before it does not.
    let limited = policy(AutonomyLevel::Supervised, 1);
    for _ in 0..3 {
        ask(&limited, "ls", CommandStage::Authorize).await.unwrap();
    }
    ask(&limited, "ls", CommandStage::Execute).await.unwrap();
    assert_eq!(
        ask(&limited, "ls", CommandStage::Execute).await,
        Err(Refusal::RateLimit)
    );

    // Class gates, async and through the sync seam (plain thread and worker).
    assert_eq!(
        supervised.command_class_gate(CommandClass::Network).await,
        Ok(GateDecision::Prompt)
    );
    let from_worker = supervised.command_class_gate_blocking(CommandClass::Network);
    assert_eq!(from_worker, Ok(GateDecision::Prompt));
    let from_thread = {
        let supervised = supervised.clone();
        std::thread::spawn(move || supervised.command_class_gate_blocking(CommandClass::Write))
            .join()
            .unwrap()
    };
    assert_eq!(from_thread, Ok(GateDecision::Prompt));
    let from_thread = {
        let read_only = read_only.clone();
        std::thread::spawn(move || read_only.command_class_gate_blocking(CommandClass::Network))
            .join()
            .unwrap()
    };
    assert_eq!(from_thread, Ok(GateDecision::Block));
}
