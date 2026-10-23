//! Sandbox planning against the real TinySecurity module.
//!
//! These tests load the actual native library behind an in-memory broker, configured
//! the way the host configures it (approval, audit and an empty sandbox
//! initialization) with the host's own audit callbacks served. They need a released
//! archive (`OPENHUMAN_TEST_SECURITY_RELEASE_HOST`) or an explicitly selected local
//! fixture (`OPENHUMAN_TEST_SECURITY_MODULE`), so they are ignored by default.

use std::path::PathBuf;

use tinysecurity_bus::{
    ConstraintEnforcement2, ConstraintSupport2, IsolationLevel2, SandboxBackend,
    SandboxConstraint2, SandboxExecutor2,
};

use super::*;
use crate::modules::security::native_tests::{native_fixture_configured, NativeFixture};
use crate::modules::tinybox::contract::{JailEnforcement, NetworkPolicy};
use crate::sandbox::plan::PlannedExecutor;
use crate::sandbox::types::DockerOverrides;

const IGNORE: &str = "requires a released host key or an explicitly selected local native fixture";

struct World {
    _fixture: NativeFixture,
    _dir: tempfile::TempDir,
    module: FixedModule,
    config: Config,
    action: PathBuf,
}

fn executor(isolation: IsolationLevel2, enforcement: ConstraintEnforcement2) -> SandboxExecutor2 {
    SandboxExecutor2 {
        executor_id: executors::JAIL_EXECUTOR.into(),
        capability_id: format!("test-{isolation:?}-{enforcement:?}"),
        backend: SandboxBackend::Jail,
        available: true,
        isolation,
        constraints: [
            SandboxConstraint2::Filesystem,
            SandboxConstraint2::Network,
            SandboxConstraint2::Subprocesses,
        ]
        .into_iter()
        .map(|constraint| ConstraintSupport2 {
            constraint,
            enforcement,
        })
        .collect(),
    }
}

async fn world(executor: SandboxExecutor2) -> World {
    world_with(vec![executor]).await
}

async fn world_with(executors: Vec<SandboxExecutor2>) -> World {
    let fixture = native_fixture_configured(true, |config| {
        config.sandbox = Some(crate::modules::security::sandbox_init_config());
    })
    .await;
    let dir = tempfile::tempdir_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp"),
    )
    .unwrap_or_else(|_| tempfile::tempdir().unwrap());
    let base = dir.path().canonicalize().unwrap();
    let action = base.join("action");
    std::fs::create_dir_all(&action).unwrap();
    let config = Config {
        workspace_dir: base.join("workspace"),
        action_dir: action.clone(),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    World {
        module: FixedModule {
            proxy: fixture.proxy.clone(),
            executors,
        },
        _fixture: fixture,
        _dir: dir,
        config,
        action,
    }
}

fn policy(world: &World, network: bool) -> SandboxPolicy {
    SandboxPolicy {
        backend: SandboxBackendKind::Local,
        workspace_root: world.action.clone(),
        state_dir: world.config.workspace_dir.clone(),
        read_only_mounts: vec![],
        read_write_mounts: vec![],
        allow_network: network,
        env_passthrough: vec![],
        docker_overrides: None,
        mount_workspace: true,
    }
}

fn operation<'a>(world: &'a World, command: &'a str) -> SpawnOperation<'a> {
    SpawnOperation {
        command,
        cwd: &world.action,
        env: vec![("LANG".into(), "C".into())],
        stdin: None,
        preserve_pipeline_status: false,
    }
}

fn request<'a>(
    world: &'a World,
    policy: &'a SandboxPolicy,
    operation: &'a SpawnOperation<'a>,
    source: SpawnSource,
) -> SpawnRequest<'a> {
    SpawnRequest {
        config: &world.config,
        policy,
        source,
        operation,
        operator_off: false,
        host_run: false,
        module: ModuleSource::Fixed(&world.module),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_kernel_jail_spawn_is_planned_reserved_once_and_audited() {
    let _ = IGNORE;
    let world = world(executor(
        IsolationLevel2::Kernel,
        ConstraintEnforcement2::Enforced,
    ))
    .await;
    let policy = policy(&world, false);
    let op = operation(&world, "echo hello");
    let planned = plan(request(&world, &policy, &op, SpawnSource::Agent))
        .await
        .expect("the module plans a kernel jail spawn");
    assert_eq!(planned.plan.executor, PlannedExecutor::NativeJail);
    assert!(!planned.plan.jail.allow_net);
    assert!(planned.plan.jail.require_process_isolation);
    assert_eq!(
        planned.plan.jail.read_write,
        [world.action.to_str().unwrap().to_owned()]
    );
    consume(&planned)
        .await
        .expect("an issued plan is admitted once");
    assert_eq!(
        consume(&planned).await.unwrap_err(),
        PlanError::Refused(SandboxDenial2::InvalidContext),
        "a plan is never admitted twice"
    );
    // Two events (plan, consume) reached the subject's ledger, plus the replay's
    // refusal left no extra permission behind.
    let tail = planned
        .stream
        .correlation(1)
        .await
        .expect("correlation")
        .expected_tail;
    assert!(
        tail >= 2,
        "plan and consume were both audited (tail {tail})"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_changed_operation_never_reuses_a_plan() {
    let world = world(executor(
        IsolationLevel2::Kernel,
        ConstraintEnforcement2::Enforced,
    ))
    .await;
    let policy = policy(&world, false);
    let op = operation(&world, "echo hello");
    let mut planned = plan(request(&world, &policy, &op, SpawnSource::Agent))
        .await
        .expect("plan");
    planned.request.operation_json = operation(&world, "echo goodbye").canonical().unwrap();
    assert!(consume(&planned).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_policy_that_allows_the_network_gets_it_without_a_per_command_approval() {
    let world = world(executor(
        IsolationLevel2::Kernel,
        ConstraintEnforcement2::Enforced,
    ))
    .await;
    let asked = policy(&world, true);
    let op = operation(&world, "curl example.com");
    let planned = plan(request(&world, &asked, &op, SpawnSource::Agent))
        .await
        .expect("planned with the network");
    assert!(
        planned.plan.jail.allow_net,
        "the allowance reaches the plan"
    );
    assert_eq!(planned.plan.network, NetworkPolicy::Open);
    assert!(planned.apply(&asked).allow_network);

    // A policy without network still runs offline.
    let offline = policy(&world, false);
    let planned = plan(request(&world, &offline, &op, SpawnSource::Agent))
        .await
        .expect("planned offline");
    assert!(!planned.plan.jail.allow_net);
    assert!(!planned.apply(&offline).allow_network);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_best_effort_jail_runs_the_agents_own_commands_but_never_untrusted_text() {
    let world = world(executor(
        IsolationLevel2::Process,
        ConstraintEnforcement2::BestEffort,
    ))
    .await;
    let policy = policy(&world, false);
    let op = operation(&world, "echo hello");
    let planned = plan(request(&world, &policy, &op, SpawnSource::Agent))
        .await
        .expect("the agent's own command may run folder-confined (macOS Seatbelt)");
    assert!(!planned.plan.jail.require_process_isolation);
    assert_eq!(planned.plan.jail.enforcement, JailEnforcement::Declared);
    consume(&planned).await.expect("admitted once");
    assert_eq!(
        plan(request(&world, &policy, &op, SpawnSource::Untrusted))
            .await
            .unwrap_err(),
        PlanError::Refused(SandboxDenial2::Unsupported),
        "hooks, scheduled jobs and registry servers stay refused"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn text_the_host_did_not_write_needs_kernel_isolation_whatever_the_switches_say() {
    let world = world(executor(
        IsolationLevel2::Process,
        ConstraintEnforcement2::Enforced,
    ))
    .await;
    let policy = policy(&world, false);
    let op = operation(&world, "echo hello");
    // A scheduled job or hook never gets a process-level jail, however exactly it
    // enforces its constraints.
    assert_eq!(
        plan(request(&world, &policy, &op, SpawnSource::Untrusted))
            .await
            .unwrap_err(),
        PlanError::Refused(SandboxDenial2::Unsupported)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_grant_outside_the_host_chosen_roots_or_inside_a_credential_store_is_refused() {
    let world = world(executor(
        IsolationLevel2::Kernel,
        ConstraintEnforcement2::Enforced,
    ))
    .await;
    let mut policy = policy(&world, false);
    // The host's own mount list names a credential store: the module's floor refuses it.
    policy.read_only_mounts = vec![PathBuf::from("/home/someone/.ssh")];
    let op = operation(&world, "echo hello");
    assert!(plan(request(&world, &policy, &op, SpawnSource::Agent))
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn container_bounds_come_from_the_plan() {
    let world = world(executor(
        IsolationLevel2::Kernel,
        ConstraintEnforcement2::Enforced,
    ))
    .await;
    let mut policy = policy(&world, false);
    policy.backend = SandboxBackendKind::Docker;
    policy.docker_overrides = Some(DockerOverrides {
        image: Some("alpine".into()),
        network: Some("bridge".into()),
        memory_limit_mb: Some(256),
        cpu_limit: Some(0.5),
        read_only_rootfs: Some(true),
        extra_caps_drop: vec![],
    });
    let docker = FixedModule {
        proxy: world.module.proxy.clone(),
        executors: vec![executors::docker(true)],
    };
    let op = operation(&world, "echo hello");
    let planned = plan(SpawnRequest {
        module: ModuleSource::Fixed(&docker),
        ..request(&world, &policy, &op, SpawnSource::Agent)
    })
    .await
    .expect("planned");
    assert_eq!(planned.plan.executor, PlannedExecutor::Docker);
    let applied = planned.apply(&policy);
    let overrides = applied.docker_overrides.expect("overrides");
    assert_eq!(overrides.memory_limit_mb, Some(256));
    assert_eq!(overrides.cpu_limit, Some(0.5));
    assert_eq!(
        overrides.network.as_deref(),
        Some("none"),
        "the container joins no network"
    );
    assert!(!applied.allow_network);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn another_subject_gets_its_own_scope_and_ledger() {
    let world = world(executor(
        IsolationLevel2::Kernel,
        ConstraintEnforcement2::Enforced,
    ))
    .await;
    let policy = policy(&world, false);
    let op = operation(&world, "echo hello");
    let first = plan(request(&world, &policy, &op, SpawnSource::Agent))
        .await
        .expect("first");
    let second = plan(request(&world, &policy, &op, SpawnSource::Untrusted));
    // The untrusted source needs kernel isolation, which this jail declares.
    let second = second.await.expect("second");
    assert_ne!(first.request.scope_id, second.request.scope_id);
    assert_eq!(first.subject, second.subject);
    assert_eq!(first.stream.stream_id(), second.stream.stream_id());
}

#[tokio::test]
async fn a_module_that_cannot_be_reached_refuses_the_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let mut config = Config {
        workspace_dir: base.join("workspace"),
        action_dir: base.join("action"),
        ..Config::default()
    };
    config.modules.enabled = false;
    let policy = SandboxPolicy {
        backend: SandboxBackendKind::Local,
        workspace_root: config.action_dir.clone(),
        state_dir: config.workspace_dir.clone(),
        read_only_mounts: vec![],
        read_write_mounts: vec![],
        allow_network: false,
        env_passthrough: vec![],
        docker_overrides: None,
        mount_workspace: true,
    };
    let op = SpawnOperation {
        command: "echo hello",
        cwd: &config.action_dir,
        env: vec![],
        stdin: None,
        preserve_pipeline_status: false,
    };
    let refused = plan(SpawnRequest {
        config: &config,
        policy: &policy,
        source: SpawnSource::Agent,
        operation: &op,
        operator_off: false,
        host_run: false,
        module: ModuleSource::Production,
    })
    .await
    .unwrap_err();
    assert!(matches!(refused, PlanError::Unavailable(_)), "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_host_retires_the_least_recently_used_scope_instead_of_growing_without_bound() {
    use tinysecurity_bus::{RegisterSandboxScopeRequest2, SandboxScopeOutcome2};

    let world = world(executor(
        IsolationLevel2::Kernel,
        ConstraintEnforcement2::Enforced,
    ))
    .await;
    let stream = "9".repeat(32);
    let scope_for = |index: usize| {
        let mut policy = policy(&world, false);
        policy.workspace_root = world.action.join(format!("w{index}"));
        facts::scope(&facts::FactsInput {
            config: &world.config,
            policy: &policy,
            source: SpawnSource::Agent,
            subject: tinysecurity_bus::SubjectScope {
                user: "local".into(),
                workspace: "evict".into(),
                agent: "agent".into(),
            },
            saas: false,
            operator_off: false,
            executors: world.module.executors.clone(),
        })
        .expect("scope facts")
    };
    let first = scope_for(0);
    registry::ensure(&world.module.proxy, &first, &stream)
        .await
        .expect("first scope");
    for index in 1..=300 {
        registry::ensure(&world.module.proxy, &scope_for(index), &stream)
            .await
            .expect("later scope");
    }
    // The first scope fell out of the host's bound and was retired in the module: it is
    // registered as a new scope, not found already there.
    let receipt = client::register_scope(
        &world.module.proxy,
        RegisterSandboxScopeRequest2 {
            scope: first,
            audit_stream_id: stream,
            report_handle: false,
        },
    )
    .await
    .expect("registration");
    assert_eq!(receipt.outcome, SandboxScopeOutcome2::Created);
}

fn jail(isolation: IsolationLevel2, enforcement: ConstraintEnforcement2) -> SandboxExecutor2 {
    executor(isolation, enforcement)
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_module_chooses_the_jail_when_the_policy_names_no_executor() {
    let world = world_with(vec![
        jail(IsolationLevel2::Kernel, ConstraintEnforcement2::Enforced),
        executors::docker(true),
    ])
    .await;
    let mut policy = policy(&world, false);
    policy.backend = SandboxBackendKind::None;
    let op = operation(&world, "echo hello");
    let planned = plan(request(&world, &policy, &op, SpawnSource::Agent))
        .await
        .expect("planned");
    assert_eq!(planned.plan.executor, PlannedExecutor::NativeJail);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_module_falls_back_to_the_container_when_the_jail_is_below_the_floor() {
    // Untrusted text needs kernel isolation: a folder-confined jail cannot run it, so the
    // module plans the container instead of the host refusing or degrading.
    let world = world_with(vec![
        jail(IsolationLevel2::Process, ConstraintEnforcement2::BestEffort),
        executors::docker(true),
    ])
    .await;
    let mut policy = policy(&world, false);
    policy.backend = SandboxBackendKind::None;
    let op = operation(&world, "echo hello");
    let planned = plan(request(&world, &policy, &op, SpawnSource::Untrusted))
        .await
        .expect("planned");
    assert_eq!(planned.plan.executor, PlannedExecutor::Docker);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_named_container_is_planned_even_when_a_jail_is_declared_first() {
    let world = world_with(vec![
        jail(IsolationLevel2::Kernel, ConstraintEnforcement2::Enforced),
        executors::docker(true),
    ])
    .await;
    let mut policy = policy(&world, false);
    policy.backend = SandboxBackendKind::Docker;
    let op = operation(&world, "echo hello");
    let planned = plan(request(&world, &policy, &op, SpawnSource::Agent))
        .await
        .expect("planned");
    assert_eq!(planned.plan.executor, PlannedExecutor::Docker);
    // A required executor that is not available is a refusal, not another executor.
    let down = world_with(vec![
        jail(IsolationLevel2::Kernel, ConstraintEnforcement2::Enforced),
        executors::docker(false),
    ])
    .await;
    let policy = SandboxPolicy {
        workspace_root: down.action.clone(),
        state_dir: down.config.workspace_dir.clone(),
        ..policy
    };
    assert_eq!(
        plan(request(&down, &policy, &op, SpawnSource::Agent))
            .await
            .unwrap_err(),
        PlanError::Refused(SandboxDenial2::Unsupported)
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_workspace_less_container_is_planned_with_no_workspace_grant() {
    let world = world_with(vec![executors::docker(true)]).await;
    let mut policy = policy(&world, false);
    policy.backend = SandboxBackendKind::Docker;
    policy.mount_workspace = false;
    let op = operation(&world, "echo hello");
    let planned = plan(request(&world, &policy, &op, SpawnSource::Agent))
        .await
        .expect("the module plans a container with no workspace");
    assert_eq!(planned.plan.executor, PlannedExecutor::Docker);
    assert!(!planned.plan.workspace);
    assert!(planned.plan.jail.read_write.is_empty());
    consume(&planned).await.expect("admitted once");
    assert!(!planned.apply(&policy).mount_workspace);
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn the_operator_with_the_sandbox_off_gets_a_module_planned_run_on_the_host() {
    let world = world_with(vec![]).await;
    let policy = policy(&world, false);
    let op = operation(&world, "echo hello");
    let planned = plan(SpawnRequest {
        operator_off: true,
        host_run: true,
        ..request(&world, &policy, &op, SpawnSource::Agent)
    })
    .await
    .expect("the trusted operator's switch is honoured by the module");
    assert_eq!(planned.plan.executor, PlannedExecutor::Host);
    consume(&planned)
        .await
        .expect("a host run is admitted once, like any plan");
    assert_eq!(
        consume(&planned).await.unwrap_err(),
        PlanError::Refused(SandboxDenial2::InvalidContext),
        "a host run is single use"
    );
    let tail = planned
        .stream
        .correlation(1)
        .await
        .expect("correlation")
        .expected_tail;
    assert!(
        tail >= 2,
        "plan and consume were both audited (tail {tail})"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn untrusted_text_never_gets_a_run_on_the_host() {
    let world = world_with(vec![]).await;
    let policy = policy(&world, false);
    let op = operation(&world, "echo hello");
    assert_eq!(
        plan(SpawnRequest {
            operator_off: true,
            host_run: true,
            ..request(&world, &policy, &op, SpawnSource::Untrusted)
        })
        .await
        .unwrap_err(),
        PlanError::Refused(SandboxDenial2::Floor),
        "a hook, a scheduled job or a registry server is refused, not run unjailed"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn a_run_on_the_host_needs_the_operators_setting_in_the_scope() {
    let world = world_with(vec![]).await;
    let policy = policy(&world, false);
    let op = operation(&world, "echo hello");
    assert_eq!(
        plan(SpawnRequest {
            operator_off: false,
            host_run: true,
            ..request(&world, &policy, &op, SpawnSource::Agent)
        })
        .await
        .unwrap_err(),
        PlanError::Refused(SandboxDenial2::Floor)
    );
}
