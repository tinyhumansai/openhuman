use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::sandbox::boxes::OwnerScope;

fn settings() -> BoxSettings {
    BoxSettings {
        image: "registry.example/agent:1".into(),
        cpu_millis: 750,
        memory_bytes: 256 * 1024 * 1024,
        workdir: "/sandbox".into(),
        owner_scope: OwnerScope::default(),
    }
}

fn info(state: &str) -> contract::ResourceInfo {
    contract::ResourceInfo {
        resource: contract::ResourceId("r-1".into()),
        backend: OPENSHELL_BACKEND.into(),
        state: state.into(),
        published_ports: Vec::new(),
    }
}

#[test]
fn a_box_is_an_openshell_image_with_no_network_no_ports_and_no_host_env() {
    let request = create_request(
        contract::ResourceId("r-1".into()),
        BoxBackend::OpenShell,
        &settings(),
        None,
    );

    assert_eq!(request.resource, contract::ResourceId("r-1".into()));
    assert_eq!(request.backend, "openshell");
    assert_eq!(
        request.workspace,
        contract::Workspace::Image("registry.example/agent:1".into())
    );
    assert_eq!(request.network, contract::NetworkPolicy::Denied);
    assert_eq!(request.host, contract::HostConfig::Local);
    assert!(request.ports.is_empty());
    assert!(request.env.is_empty());
    assert_eq!(request.resources.cpu_millis, 750);
    assert_eq!(request.resources.memory_bytes, 256 * 1024 * 1024);
    assert_eq!(
        request.resources.pids_max,
        contract::ResourceLimits::default().pids_max
    );
    assert_eq!(
        request.resources.disk_bytes,
        contract::ResourceLimits::default().disk_bytes
    );
}

#[test]
fn an_openshell_box_is_never_retained() {
    let request = create_request(
        contract::ResourceId("r-1".into()),
        BoxBackend::OpenShell,
        &settings(),
        None,
    );
    assert_eq!(request.retention, None);
}

#[test]
fn an_agent37_box_starts_from_its_template_and_carries_its_owners_retention() {
    let settings = BoxSettings::from_agent37(&crate::config::Agent37RuntimeConfig::default())
        .expect("the default agent37 settings are valid");
    let retention = Retention {
        key: "oh-abc".into(),
        idle_sleep_secs: 900,
        delete_after_secs: 2_592_000,
    };

    let request = create_request(
        contract::ResourceId("r-1".into()),
        BoxBackend::Agent37,
        &settings,
        Some(&retention),
    );

    assert_eq!(request.backend, "agent37");
    assert_eq!(
        request.workspace,
        contract::Workspace::Image("agent37-base".into())
    );
    assert_eq!(request.network, contract::NetworkPolicy::Denied);
    assert_eq!(request.resources.cpu_millis, 2_000);
    assert_eq!(request.resources.memory_bytes, 4 * 1024 * 1024 * 1024);
    assert_eq!(
        request.retention,
        Some(contract::Retention {
            key: "oh-abc".into(),
            idle_sleep_secs: 900,
            delete_after_secs: 2_592_000,
        })
    );
}

#[test]
fn an_exec_carries_argv_workdir_and_only_the_given_env() {
    let mut env = BTreeMap::new();
    env.insert("LANG".to_owned(), "C.UTF-8".to_owned());
    let exec = BoxExec {
        argv: vec!["sh".into(), "-c".into(), "true".into()],
        cwd: "/sandbox".into(),
        env: env.clone(),
        deadline: Duration::from_secs(40),
    };

    let request = exec_request("r-1", &exec);

    assert_eq!(request.resource, contract::ResourceId("r-1".into()));
    assert_eq!(request.argv, exec.argv);
    assert_eq!(request.cwd.as_deref(), Some("/sandbox"));
    assert_eq!(request.env, env);
    assert_eq!(request.stdin, None);
}

#[test]
fn the_bus_waits_past_the_registry_backstop() {
    assert_eq!(
        bus_deadline(Duration::from_secs(40)),
        Duration::from_secs(40) + BUS_REPLY_GRACE
    );
}

#[test]
fn a_failed_exec_in_a_live_box_is_a_driver_error() {
    for state in ["ready", "running"] {
        assert_eq!(
            exec_failure("exec broke".into(), Ok(info(state))),
            BoxError::Driver("exec broke".into())
        );
    }
}

#[test]
fn a_failed_exec_in_a_dead_box_is_an_unknown_resource() {
    for state in ["failed", "stopped", "archived"] {
        assert_eq!(
            exec_failure("exec broke".into(), Ok(info(state))),
            BoxError::UnknownResource
        );
    }
}

#[test]
fn a_box_the_module_refuses_to_inspect_is_an_unknown_resource() {
    let refused = tinybus::module::ModuleCallError::ModuleFault.to_string();
    assert_eq!(
        exec_failure("exec broke".into(), Err(refused)),
        BoxError::UnknownResource
    );
}

#[test]
fn an_unreachable_module_is_a_driver_error_not_a_lost_box() {
    let unreachable = tinybus::module::ModuleCallError::Unavailable.to_string();
    assert_eq!(
        exec_failure("exec broke".into(), Err(unreachable)),
        BoxError::Driver("exec broke".into())
    );
}

#[test]
fn output_is_passed_through_unchanged() {
    let output = box_output(contract::ExecOutput {
        exit_code: 3,
        stdout: b"out".to_vec(),
        stderr: b"err".to_vec(),
    });
    assert_eq!(
        output,
        BoxOutput {
            exit_code: 3,
            stdout: b"out".to_vec(),
            stderr: b"err".to_vec(),
        }
    );
}

fn config_in(workspace: &str) -> Config {
    let mut config = Config::default();
    config.workspace_dir = workspace.into();
    config
}

#[tokio::test]
async fn each_call_reaches_the_module_with_its_own_configuration() {
    let driver = BusBoxDriver::new(config_in("/first"), BoxBackend::Agent37);
    assert_eq!(
        driver.config().workspace_dir,
        config_in("/first").workspace_dir
    );

    let seen = driver
        .serve(&config_in("/second"), async {
            driver.config().workspace_dir.clone()
        })
        .await;
    assert_eq!(seen, config_in("/second").workspace_dir);

    // Two calls in flight at once each keep their own.
    let (a, b) = tokio::join!(
        driver.serve(&config_in("/a"), async {
            tokio::task::yield_now().await;
            driver.config().workspace_dir.clone()
        }),
        driver.serve(&config_in("/b"), async {
            tokio::task::yield_now().await;
            driver.config().workspace_dir.clone()
        }),
    );
    assert_eq!(a, config_in("/a").workspace_dir);
    assert_eq!(b, config_in("/b").workspace_dir);

    // Work outside any call (the reaper) uses the most recent call's.
    assert_eq!(driver.config().workspace_dir, config_in("/b").workspace_dir);
}

#[tokio::test(start_paused = true)]
async fn a_reservation_dropped_before_create_answers_is_abandoned() {
    let abandoned = Arc::new(std::sync::Mutex::new(Vec::new()));
    let create = {
        let abandoned = Arc::clone(&abandoned);
        async move {
            let guard = AbandonGuard::new("r-1".to_owned(), move |resource| {
                abandoned.lock().unwrap().push(resource);
            });
            // Create never answers; the caller gives up here.
            std::future::pending::<()>().await;
            guard.keep();
        }
    };

    let gave_up = tokio::time::timeout(Duration::from_secs(1), create).await;

    assert!(gave_up.is_err());
    assert_eq!(*abandoned.lock().unwrap(), vec!["r-1".to_owned()]);
}

#[test]
fn a_reservation_kept_after_create_answers_is_not_abandoned() {
    let abandoned = std::cell::Cell::new(false);
    AbandonGuard::new("r-1", |_| abandoned.set(true)).keep();
    assert!(!abandoned.get());
}

#[tokio::test]
async fn a_call_with_modules_disabled_keeps_the_configuration_cleanups_use() {
    let mut enabled = config_in("/enabled");
    enabled.modules.enabled = true;
    let driver = BusBoxDriver::new(enabled, BoxBackend::Agent37);

    let mut disabled = config_in("/disabled");
    disabled.modules.enabled = false;
    driver.serve(&disabled, async {}).await;

    // A later cleanup, outside any call, must still reach the module with
    // the configuration that has it enabled.
    assert_eq!(
        driver.config().workspace_dir,
        config_in("/enabled").workspace_dir
    );
}

#[test]
fn a_failed_create_closes_its_reservation() {
    let abandoned = std::cell::RefCell::new(Vec::new());
    let guard = AbandonGuard::new("r-1".to_owned(), |resource| {
        abandoned.borrow_mut().push(resource);
    });

    let result = settle_create(guard, Err("create refused".to_owned()));

    assert_eq!(result, Err(BoxError::Driver("create refused".to_owned())));
    assert_eq!(*abandoned.borrow(), vec!["r-1".to_owned()]);
}

#[test]
fn a_created_box_keeps_its_reservation_and_is_returned() {
    let abandoned = std::cell::Cell::new(false);
    let guard = AbandonGuard::new("r-1".to_owned(), |_| abandoned.set(true));

    let result = settle_create(guard, Ok(info("ready")));

    assert_eq!(result, Ok("r-1".to_owned()));
    assert!(!abandoned.get());
}

#[tokio::test]
async fn a_box_is_cleaned_up_with_the_configuration_it_was_created_under() {
    let driver = BusBoxDriver::new(config_in("/created"), BoxBackend::Agent37);
    let created_under = driver.config();
    driver.remember("box-0", &created_under);

    // A later call replaces the most recent configuration.
    driver.serve(&config_in("/later"), async {}).await;

    assert_eq!(
        driver.config_of("box-0").workspace_dir,
        config_in("/created").workspace_dir
    );
    assert_eq!(
        driver.config_of("box-9").workspace_dir,
        config_in("/later").workspace_dir,
        "a box never recorded falls back to the most recent configuration"
    );
}
