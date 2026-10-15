use super::*;
use tinybus::{broker::Broker, module::ModuleHost, transport::memory::MemoryBus, Connection};
use tinysecurity_bus::{PathAccess, PathPolicy, PathTrustedRoot, PathValidationResult};

/// Release-build per-call budget committed by the owner (tinysecurity#8,
/// `docs/performance.md`): p50 <= 1 ms and p99 <= 10 ms per Evaluate/Check.
/// Enforced only when `OPENHUMAN_SECURITY_PERF_BUDGET=1` on a release build.
const PERF_BUDGET_P50: Duration = Duration::from_millis(1);
const PERF_BUDGET_P99: Duration = Duration::from_millis(10);
const PERF_BUDGET_ENV: &str = "OPENHUMAN_SECURITY_PERF_BUDGET";
const PERF_OUT_ENV: &str = "OPENHUMAN_SECURITY_PERF_OUT";
const TURN_CHECKS: usize = 5;
const TURN_SAMPLES: usize = 200;

/// A released (or explicitly selected local) module loaded behind an in-memory
/// broker, with the host connection already named as its trusted peer.
pub(super) struct NativeFixture {
    pub(super) _fixtures: tempfile::TempDir,
    pub(super) acting: tempfile::TempDir,
    pub(super) host: ModuleHost,
    pub(super) _broker_task: tokio::task::JoinHandle<tinybus::Result<()>>,
    pub(super) proxy: tinybus::Proxy,
    pub(super) client: Connection,
    pub(super) record: super::super::ModuleRecord,
}

pub(super) async fn native_fixture() -> NativeFixture {
    native_fixture_with(None).await
}

/// As [`native_fixture`], with the init-only confidential crypto surface.
pub(super) async fn native_fixture_with(
    crypto: Option<tinysecurity_bus::CryptoConfig2>,
) -> NativeFixture {
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    let fixture_root = std::env::var_os("OPENHUMAN_TEST_SECURITY_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| target.clone());
    assert!(
        fixture_root.is_absolute(),
        "native fixture root must be absolute"
    );
    std::fs::create_dir_all(&fixture_root).unwrap();
    let fixtures = tempfile::tempdir_in(&fixture_root).unwrap();
    restrict_native_fixture(fixtures.path());
    let acting = tempfile::tempdir_in(&target).unwrap();
    let transport = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(transport.clone());
    let host = ModuleHost::new(broker);
    // The module accepts privileged members only from the peer named at its
    // first initialization, so the host connection must exist before the load.
    let client = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let configuration = serde_json::to_value(ModuleConfig {
        trusted_host: client.unique_name().map(|name| name.to_string()),
        crypto,
        ..ModuleConfig::default()
    })
    .unwrap();
    let record = if let Ok(host_key) = std::env::var("OPENHUMAN_TEST_SECURITY_RELEASE_HOST") {
        let record = *super::super::registry::find(MODULE_ID).unwrap();
        let asset = record
            .asset_for(&host_key)
            .expect("published fixture platform must be pinned");
        host.load_github_release(
            record.release_url,
            asset.archive,
            Some(asset.sha256),
            configuration,
        )
        .expect("released module must pass manifest, digest and native admission");
        record
    } else {
        // Explicit local development fixture. It never supplies production pins.
        let library = std::path::PathBuf::from(
            std::env::var_os("OPENHUMAN_TEST_SECURITY_MODULE")
                .expect("configure a released host key or a local native fixture"),
        );
        let name = library.file_name().unwrap().to_str().unwrap();
        let copied = fixtures.path().join(name);
        std::fs::copy(&library, &copied).unwrap();
        let digest = tinybus::module::sha256_file(&copied).unwrap();
        std::fs::write(
            fixtures.path().join("modules.toml"),
            format!("\"{name}\" = \"{digest}\"\n"),
        )
        .unwrap();
        host.load_file_with_config(&copied, configuration).unwrap();
        super::super::ModuleRecord {
            id: MODULE_ID,
            description: "test-only hashed native fixture",
            bus_name: names::INTERFACE,
            object_path: names::OBJECT_PATH,
            version: "test-only",
            release_url: "https://example.invalid",
            assets: Box::leak(
                vec![super::super::PlatformAsset {
                    host_key: "test-only",
                    archive: "test-only-library",
                    sha256: Box::leak(digest.into_boxed_str()),
                }]
                .into_boxed_slice(),
            ),
            load: super::super::LoadPolicy::Eager,
            source: crate::modules::types::SourceSpec {
                submodule: "tinysecurity",
                manifest: "crates/tinysecurity-module/Cargo.toml",
                package: "tinysecurity-module",
                lib_name: "tinysecurity_module",
                features: &[],
            },
        }
    };
    let proxy = client
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .unwrap()
        .with_timeout(CALL_TIMEOUT);
    tokio::time::timeout(LOAD_TIMEOUT, async {
        while !proxy.is_available().await.unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let unpinned = super::super::ModuleRecord {
        assets: &[],
        ..record
    };
    assert_eq!(
        require_attestation(&proxy, &unpinned).await.unwrap_err(),
        "TinySecurity recipient does not match a pinned release"
    );
    require_attestation(&proxy, &record).await.unwrap();
    NativeFixture {
        _fixtures: fixtures,
        acting,
        host,
        _broker_task: broker_task,
        proxy,
        client,
        record,
    }
}

/// The native CI lane loads the real release through its compiled archive pin.
/// An explicitly selected local fixture remains available for module development.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn attested_native_path_policy_resolves_and_denies_through_host_client() {
    let NativeFixture {
        _fixtures,
        acting,
        host,
        _broker_task: broker_task,
        proxy,
        record: _,
        ..
    } = native_fixture().await;
    let root = acting.path().canonicalize().unwrap();
    let action = root.join("action");
    std::fs::create_dir(&action).unwrap();
    std::fs::write(action.join("readable.txt"), "fixture").unwrap();
    let policy = PathPolicy {
        enabled: true,
        workspace_only: true,
        workspace_dir: root.join("state"),
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
    let mut malformed = policy.clone();
    malformed.action_dir = "relative-root".into();
    assert!(call_register_path_policy(&proxy, malformed).await.is_err());
    let id = call_register_path_policy(&proxy, policy.clone())
        .await
        .unwrap();
    assert_eq!(
        id,
        call_register_path_policy(&proxy, policy.clone())
            .await
            .unwrap()
    );
    let valid = call_validate_path(
        &proxy,
        id.clone(),
        "readable.txt".into(),
        names::methods::VALIDATE_PATH,
    )
    .await
    .unwrap();
    assert_eq!(
        valid,
        PathValidationResult::Allowed {
            resolved: action.join("readable.txt")
        }
    );
    let parent = call_validate_path(
        &proxy,
        id.clone(),
        "new.txt".into(),
        names::methods::VALIDATE_PARENT,
    )
    .await
    .unwrap();
    assert_eq!(
        parent,
        PathValidationResult::Allowed {
            resolved: action.join("new.txt")
        }
    );
    assert!(matches!(
        call_validate_path(
            &proxy,
            id.clone(),
            "../outside".into(),
            names::methods::VALIDATE_PATH
        )
        .await
        .unwrap(),
        PathValidationResult::Denied { .. }
    ));
    host.reinitialize(
        "tinysecurity-module",
        serde_json::to_value(ModuleConfig::default()).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        valid,
        call_validate_path(
            &proxy,
            id.clone(),
            "readable.txt".into(),
            names::methods::VALIDATE_PATH
        )
        .await
        .unwrap()
    );
    verify_host_translation(&proxy, &root).await;
    // Production memoizes immutable registration and attested proxy; the
    // warmed validation path uses exactly one confidential bus call per check.
    let mut operation_state = ClientState::default();
    // Shared production registration/cache and native response-validation
    // helper, warmed once before timing. SDK admission precedes measurement.
    call_host_path(
        &mut operation_state,
        &proxy,
        policy.clone(),
        "readable.txt",
        false,
    )
    .await
    .unwrap();
    let mut samples = Vec::with_capacity(500);
    for _ in 0..500 {
        let started = std::time::Instant::now();
        assert!(matches!(
            call_host_path(
                &mut operation_state,
                &proxy,
                policy.clone(),
                "readable.txt",
                false
            )
            .await
            .unwrap(),
            PathValidationResult::Allowed { .. }
        ));
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    println!("native path client: 500 samples, 1 bus call/sample, shared production scope-cache/validation helper; excludes config-file load and first admission; p50={:?}, p99={:?}; admission budget p99<50ms", samples[250], samples[494]);
    assert!(
        samples[494] < Duration::from_millis(50),
        "native path p99 exceeds 50ms admission budget"
    );

    // Representative turn: one shell tool call that reads three files, creates
    // one and probes one path outside the root. Each check is exactly one bus
    // call once the registration is cached, so a turn costs TURN_CHECKS calls.
    // The per-call distribution is gated against the release-build budget the
    // owner commits in vendor/tinysecurity/docs/performance.md.
    const TURN: [(&str, bool); TURN_CHECKS] = [
        ("readable.txt", false),
        ("readable.txt", false),
        ("readable.txt", false),
        ("new.txt", true),
        ("../outside", false),
    ];
    let mut per_call = Vec::with_capacity(TURN_SAMPLES * TURN_CHECKS);
    let mut per_turn = Vec::with_capacity(TURN_SAMPLES);
    let mut calls = 0usize;
    for _ in 0..TURN_SAMPLES {
        let turn_started = std::time::Instant::now();
        for (path, parent) in TURN {
            let started = std::time::Instant::now();
            let outcome =
                call_host_path(&mut operation_state, &proxy, policy.clone(), path, parent)
                    .await
                    .unwrap();
            per_call.push(started.elapsed());
            calls += 1;
            assert_eq!(
                matches!(outcome, PathValidationResult::Denied { .. }),
                path == "../outside",
                "{path}"
            );
        }
        per_turn.push(turn_started.elapsed());
    }
    assert_eq!(calls, TURN_SAMPLES * TURN_CHECKS);
    per_call.sort_unstable();
    per_turn.sort_unstable();
    let pct = |v: &[Duration], q: f64| v[((v.len() as f64 * q) as usize).min(v.len() - 1)];
    let release = !cfg!(debug_assertions);
    let record = serde_json::json!({
        "profile": if release { "release" } else { "debug" },
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "workload": "shell turn: 3 reads, 1 parent-create, 1 denied probe; warmed registration",
        "calls_per_turn": TURN_CHECKS,
        "turns": TURN_SAMPLES,
        "call_p50_us": pct(&per_call, 0.50).as_micros(),
        "call_p99_us": pct(&per_call, 0.99).as_micros(),
        "turn_p50_us": pct(&per_turn, 0.50).as_micros(),
        "turn_p99_us": pct(&per_turn, 0.99).as_micros(),
        "budget_call_p50_us": PERF_BUDGET_P50.as_micros(),
        "budget_call_p99_us": PERF_BUDGET_P99.as_micros(),
    });
    println!("SECURITY-PERF {record}");
    if let Some(out) = std::env::var_os(PERF_OUT_ENV) {
        std::fs::write(out, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    }
    if std::env::var(PERF_BUDGET_ENV).as_deref() == Ok("1") {
        assert!(
            release,
            "{PERF_BUDGET_ENV}=1 gates the release-build budget; run with --release"
        );
        assert!(
            pct(&per_call, 0.50) <= PERF_BUDGET_P50,
            "native per-call p50 {:?} exceeds the {PERF_BUDGET_P50:?} budget",
            pct(&per_call, 0.50)
        );
        assert!(
            pct(&per_call, 0.99) <= PERF_BUDGET_P99,
            "native per-call p99 {:?} exceeds the {PERF_BUDGET_P99:?} budget",
            pct(&per_call, 0.99)
        );
    }
    host.shutdown(Duration::from_secs(1)).await;
    assert!(
        call_validate_path(
            &proxy,
            id,
            "readable.txt".into(),
            names::methods::VALIDATE_PATH
        )
        .await
        .is_err(),
        "stopped module must refuse validation without an in-process fallback"
    );
    broker_task.abort();
}

#[cfg(not(windows))]
fn restrict_native_fixture(_path: &std::path::Path) {}

#[cfg(windows)]
fn restrict_native_fixture(path: &std::path::Path) {
    // TinyBus correctly rejects a library writable by arbitrary Windows users.
    // Protect the fixture directory before copying, so files inherit this ACL.
    let script = r#"
$ErrorActionPreference = 'Stop'
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
$security = [System.Security.AccessControl.DirectorySecurity]::new()
$security.SetOwner($identity.User)
$security.SetAccessRuleProtection($true, $false)
foreach ($value in @($identity.User.Value, 'S-1-5-18', 'S-1-5-32-544')) {
    $sid = [System.Security.Principal.SecurityIdentifier]::new($value)
    $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($sid,
        [System.Security.AccessControl.FileSystemRights]::FullControl,
        [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit',
        [System.Security.AccessControl.PropagationFlags]::None,
        [System.Security.AccessControl.AccessControlType]::Allow)
    $security.AddAccessRule($rule)
}
Set-Acl -LiteralPath $env:OPENHUMAN_NATIVE_FIXTURE_DIR -AclObject $security
"#;
    let status = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("OPENHUMAN_NATIVE_FIXTURE_DIR", path)
        .status()
        .expect("PowerShell must protect native fixture ACL");
    assert!(status.success(), "native fixture ACL setup failed");
}

/// Exercise the product's actual host translation against the loaded native
/// engine, rather than hand-constructing a simplified module scope.
async fn verify_host_translation(proxy: &tinybus::Proxy, root: &std::path::Path) {
    use crate::security::{SecurityPolicy, TrustedAccess, TrustedRoot};
    let workspace = root.join("host-workspace");
    let action = root.join("host-action");
    let reference = root.join("host-reference");
    for path in [
        &action,
        &reference,
        &workspace.join("memory"),
        &workspace.join("artifacts/tool-results"),
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    for path in [
        action.join("document.txt"),
        reference.join("read.txt"),
        workspace.join("memory/private.txt"),
        workspace.join("artifacts/tool-results/output.txt"),
        root.join("config.toml"),
    ] {
        std::fs::write(path, "fixture").unwrap();
    }
    let host = SecurityPolicy {
        workspace_dir: workspace.clone(),
        action_dir: action.clone(),
        trusted_roots: vec![
            TrustedRoot {
                path: action.to_str().unwrap().into(),
                access: TrustedAccess::ReadWrite,
            },
            TrustedRoot {
                path: reference.to_str().unwrap().into(),
                access: TrustedAccess::Read,
            },
            // A broad workspace grant permits readable artifact outputs but
            // must never override the separately reserved internal state.
            TrustedRoot {
                path: workspace.to_str().unwrap().into(),
                access: TrustedAccess::ReadWrite,
            },
        ],
        ..SecurityPolicy::default()
    };
    let mut state = ClientState::default();
    let scope = host.native_path_policy();
    for (path, parent, allowed) in [
        ("document.txt".to_owned(), false, true),
        ("new-document.txt".to_owned(), true, true),
        (
            reference.join("read.txt").to_str().unwrap().into(),
            false,
            true,
        ),
        (
            reference.join("new.txt").to_str().unwrap().into(),
            true,
            false,
        ),
        (
            workspace
                .join("memory/private.txt")
                .to_str()
                .unwrap()
                .into(),
            false,
            false,
        ),
        (
            root.join("config.toml").to_str().unwrap().into(),
            false,
            false,
        ),
        (
            workspace.join("artifacts").to_str().unwrap().into(),
            false,
            true,
        ),
        (
            workspace
                .join("artifacts/tool-results/output.txt")
                .to_str()
                .unwrap()
                .into(),
            false,
            true,
        ),
        (
            workspace
                .join("artifacts/tool-results/new.txt")
                .to_str()
                .unwrap()
                .into(),
            true,
            false,
        ),
        (".env".into(), false, false),
        ("../escape".into(), true, false),
    ] {
        let result = call_host_path(&mut state, proxy, scope.clone(), &path, parent)
            .await
            .unwrap();
        assert_eq!(matches!(result, PathValidationResult::Allowed { .. }), allowed,
            "host-translated native authorization disagrees for {path}, parent={parent}: {result:?}");
    }
    std::fs::create_dir_all(action.join(".ssh")).unwrap();
    std::fs::write(action.join(".ssh/key"), "fixture").unwrap();
    let mut disabled = host;
    disabled.enabled = false;
    assert!(matches!(
        call_host_path(
            &mut state,
            proxy,
            disabled.native_path_policy(),
            workspace.join("memory/private.txt").to_str().unwrap(),
            false
        )
        .await
        .unwrap(),
        PathValidationResult::Allowed { .. }
    ));
    assert!(matches!(
        call_host_path(
            &mut state,
            proxy,
            disabled.native_path_policy(),
            ".ssh/key",
            false
        )
        .await
        .unwrap(),
        PathValidationResult::Denied { .. }
    ));
}

/// A tampered archive digest must be refused at admission, before any code runs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key (downloads the published archive)"]
async fn released_archive_with_tampered_digest_is_denied() {
    let host_key = std::env::var("OPENHUMAN_TEST_SECURITY_RELEASE_HOST")
        .expect("set OPENHUMAN_TEST_SECURITY_RELEASE_HOST");
    let record = *super::super::registry::find(MODULE_ID).unwrap();
    let asset = record.asset_for(&host_key).expect("pinned platform");
    let tampered = "0".repeat(asset.sha256.len());
    let transport = MemoryBus::new();
    let broker = Broker::new();
    let _task = broker.spawn(transport.clone());
    let host = ModuleHost::new(broker);
    let configuration = serde_json::to_value(ModuleConfig::default()).unwrap();
    let result = host.load_github_release(
        record.release_url,
        asset.archive,
        Some(&tampered),
        configuration,
    );
    assert!(result.is_err(), "tampered digest must not be admitted");
}
