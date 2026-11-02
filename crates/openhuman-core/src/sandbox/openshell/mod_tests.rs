use super::*;
use crate::config::OpenShellRuntimeConfig;
use crate::core::runtime::NoTenant;
use scripted_driver::ScriptedDriver;
use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::Arc;

fn embedded(id: &str) -> Tenant {
    Tenant {
        profile: None,
        agent: Some(id.into()),
    }
}

#[test]
fn a_call_without_a_tenant_is_refused() {
    let refused = tenant_for_box(Err(NoTenant)).unwrap_err();
    assert_eq!(refused, BoxError::NoTenant);
    assert_eq!(
        refused.to_string(),
        "no tenant scope; refusing to run a command in a shared sandbox"
    );
}

#[test]
fn the_whole_tenant_keys_the_box_not_just_its_profile() {
    assert_eq!(tenant_for_box(Ok(embedded("u1"))), Ok(embedded("u1")));
}

#[test]
fn settings_come_from_the_runtime_table() {
    let config = OpenShellRuntimeConfig {
        image: "registry.example/agent:3".into(),
        cpu_millis: 250,
        memory_mb: 64,
        idle_ttl_secs: 120,
        max_live: 4,
    };

    assert_eq!(
        BoxSettings::from_config(&config),
        Ok(BoxSettings {
            image: "registry.example/agent:3".into(),
            cpu_millis: 250,
            memory_bytes: 64 * 1024 * 1024,
        })
    );
    assert_eq!(
        RegistryLimits::from_config(&config),
        RegistryLimits {
            idle_ttl: Duration::from_secs(120),
            max_live: 4,
        }
    );
}

#[test]
fn memory_too_large_for_bytes_is_refused() {
    let config = OpenShellRuntimeConfig {
        memory_mb: u64::MAX,
        ..OpenShellRuntimeConfig::default()
    };
    let refused = BoxSettings::from_config(&config).unwrap_err();
    assert_eq!(
        refused,
        BoxError::InvalidMemory {
            memory_mb: u64::MAX
        }
    );
    assert!(refused.to_string().contains("too large"));
}

#[test]
fn unusable_box_settings_are_refused_with_a_config_error() {
    let defaults = OpenShellRuntimeConfig::default();
    for config in [
        OpenShellRuntimeConfig {
            image: "  ".into(),
            ..defaults.clone()
        },
        OpenShellRuntimeConfig {
            cpu_millis: 0,
            ..defaults.clone()
        },
        OpenShellRuntimeConfig {
            memory_mb: 0,
            ..defaults.clone()
        },
    ] {
        let refused = BoxSettings::from_config(&config).unwrap_err();
        assert!(matches!(refused, BoxError::InvalidConfig(_)), "{refused:?}");
        assert!(refused.to_string().contains("[runtime.openshell]"));
    }
}

#[test]
fn zero_registry_limits_are_raised_to_one() {
    let config = OpenShellRuntimeConfig {
        idle_ttl_secs: 0,
        max_live: 0,
        ..OpenShellRuntimeConfig::default()
    };
    assert_eq!(
        RegistryLimits::from_config(&config),
        RegistryLimits {
            idle_ttl: Duration::from_secs(1),
            max_live: 1,
        }
    );
}

#[test]
fn the_command_env_is_only_what_the_request_carries() {
    let mut extra = HashMap::new();
    extra.insert(OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0"));

    let env = box_env(&extra).unwrap();

    assert_eq!(env.len(), 1);
    assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
}

#[cfg(unix)]
#[test]
fn a_non_utf8_env_is_refused_rather_than_altered() {
    use std::os::unix::ffi::OsStringExt;
    let mut extra = HashMap::new();
    extra.insert(OsString::from("BAD"), OsString::from_vec(vec![0xff]));

    assert_eq!(box_env(&extra).unwrap_err(), BoxError::NonUtf8Env);
}

#[tokio::test(start_paused = true)]
async fn an_unscoped_command_creates_no_box() {
    let driver = ScriptedDriver::new();
    let boxes = BoxRegistry::new(
        Arc::clone(&driver),
        RegistryLimits::from_config(&OpenShellRuntimeConfig::default()),
    );

    let refused = run_command(
        &boxes,
        Err(NoTenant),
        &OpenShellRuntimeConfig::default(),
        "id",
        &HashMap::new(),
        Duration::from_secs(10),
    )
    .await
    .unwrap_err();

    assert_eq!(refused, BoxError::NoTenant);
    assert!(driver.calls().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_scoped_command_runs_in_its_tenants_box() {
    let driver = ScriptedDriver::new();
    let config = OpenShellRuntimeConfig::default();
    let boxes = BoxRegistry::new(Arc::clone(&driver), RegistryLimits::from_config(&config));

    let result = run_command(
        &boxes,
        Ok(embedded("u1")),
        &config,
        "id",
        &HashMap::new(),
        Duration::from_secs(10),
    )
    .await
    .unwrap();

    assert_eq!(result.stdout, "ok");
    assert_eq!(
        driver.calls()[0],
        scripted_driver::Call::Create(BoxSettings::from_config(&config).unwrap())
    );
    let exec = driver.last_exec().unwrap();
    assert_eq!(exec.argv[3], "10");
    assert!(exec.env.is_empty());
}

#[cfg(not(feature = "modules"))]
#[tokio::test]
async fn without_module_support_execution_is_refused() {
    let error = execute(
        &crate::core::runtime::DispatchCtx::unbound(),
        &crate::config::Config::default(),
        "id",
        &HashMap::new(),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("module support is disabled"));
}

#[tokio::test]
async fn release_and_shutdown_before_any_box_exists_are_no_ops() {
    release(&embedded("u1")).await;
    shutdown().await;
}
