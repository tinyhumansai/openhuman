use super::*;
use std::collections::BTreeMap;

fn settings() -> BoxSettings {
    BoxSettings {
        image: "registry.example/agent:1".into(),
        cpu_millis: 750,
        memory_bytes: 256 * 1024 * 1024,
    }
}

fn info(state: &str) -> contract::ResourceInfo {
    contract::ResourceInfo {
        resource: contract::ResourceId("r-1".into()),
        backend: BACKEND.into(),
        state: state.into(),
        published_ports: Vec::new(),
    }
}

#[test]
fn a_box_is_an_openshell_image_with_no_network_no_ports_and_no_host_env() {
    let request = create_request(contract::ResourceId("r-1".into()), &settings());

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
