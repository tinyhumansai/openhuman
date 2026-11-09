use super::*;
use std::path::PathBuf;

#[test]
fn status_posture_preserves_the_existing_rpc_isolation_field() {
    assert_eq!(
        status_isolation("landlock", true, true, true),
        Some(SandboxIsolation::Isolated)
    );
    assert_eq!(
        status_isolation("seatbelt", true, false, false),
        Some(SandboxIsolation::FolderConfined)
    );
    assert_eq!(status_isolation("noop", true, false, false), None);
}

fn planned_test_jail() -> crate::modules::tinybox::contract::NativeJailConfig {
    crate::modules::tinybox::contract::NativeJailConfig {
        read_only: Vec::new(),
        read_write: Vec::new(),
        allow_net: true,
        allow_subprocess: true,
        enforcement: crate::modules::tinybox::contract::JailEnforcement::Enforced,
        require_process_isolation: true,
        deny_read: Vec::new(),
        resources: None,
    }
}

#[cfg(feature = "modules")]
#[test]
fn approved_environment_carries_only_the_callers_overrides() -> anyhow::Result<()> {
    let extra = HashMap::from([(OsString::from("CUSTOM_SAFE"), OsString::from("value"))]);
    let env = approved_environment(&extra)?;

    // The module builds the safe snapshot and the scratch TMPDIR itself; the
    // host forwards nothing of the parent environment.
    assert_eq!(env.len(), 1);
    assert_eq!(env.get("CUSTOM_SAFE").map(String::as_str), Some("value"));
    assert!(approved_environment(&HashMap::new())?.is_empty());
    Ok(())
}

#[cfg(all(feature = "modules", unix))]
#[test]
fn approved_environment_refuses_values_the_bus_cannot_carry() {
    use std::os::unix::ffi::OsStringExt;

    let extra = HashMap::from([(
        OsString::from("RAW"),
        OsString::from_vec(b"before-\xff-after".to_vec()),
    )]);
    assert!(approved_environment(&extra).is_err());
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn disabled_tinybox_module_never_falls_back_to_host_execution() -> anyhow::Result<()> {
    let workspace = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let marker = workspace.path().join("must-not-run");
    let policy = SandboxPolicy {
        backend: super::super::types::SandboxBackendKind::Local,
        workspace_root: workspace.path().to_path_buf(),
        state_dir: state.path().to_path_buf(),
        read_only_mounts: Vec::new(),
        read_write_mounts: Vec::new(),
        allow_network: true,
        mount_workspace: true,
        env_passthrough: Vec::new(),
        docker_overrides: None,
    };
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;

    let result = execute(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &policy,
        &planned_test_jail(),
        &format!("touch '{}'", marker.display()),
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(5),
        false,
        None,
        None,
    )
    .await;

    assert!(result.is_err(), "native jail must refuse without TinyBox");
    assert!(!marker.exists(), "refusal must never run the host command");
    Ok(())
}

#[cfg(all(feature = "modules", target_os = "linux"))]
struct BundledModulesRestore(Option<OsString>);

#[cfg(all(feature = "modules", target_os = "linux"))]
impl Drop for BundledModulesRestore {
    fn drop(&mut self) {
        const KEY: &str = "OPENHUMAN_BUNDLED_MODULES";
        match self.0.take() {
            Some(value) => std::env::set_var(KEY, value),
            None => std::env::remove_var(KEY),
        }
    }
}

/// Turns the unit-test shell fixture off for the released-artifact test.
#[cfg(all(feature = "modules", target_os = "linux"))]
struct LiveModule;

#[cfg(all(feature = "modules", target_os = "linux"))]
impl LiveModule {
    fn enable() -> Self {
        super::super::shell_exec::USE_FIXTURE.store(false, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

#[cfg(all(feature = "modules", target_os = "linux"))]
impl Drop for LiveModule {
    fn drop(&mut self) {
        super::super::shell_exec::USE_FIXTURE.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(all(feature = "modules", target_os = "linux"))]
#[tokio::test]
#[ignore = "requires the verified pinned TinyBox archive and a supported Linux jail"]
async fn released_tinybox_runs_a_native_jail_through_the_pinned_loader() -> anyhow::Result<()> {
    let _environment_lock = crate::config::TEST_ENV_LOCK.lock().await;
    const KEY: &str = "OPENHUMAN_BUNDLED_MODULES";
    let artifact_root = std::env::var_os(KEY).map(PathBuf::from).ok_or_else(|| {
        anyhow::anyhow!("set OPENHUMAN_BUNDLED_MODULES to the downloaded pinned release assets")
    })?;
    let _environment_restore = BundledModulesRestore(std::env::var_os(KEY));
    let record = crate::modules::registry::find("tinybox")
        .ok_or_else(|| anyhow::anyhow!("TinyBox registry record is missing"))?;
    let asset = tinybus::module::platform::host_candidates()
        .iter()
        .find_map(|host_key| record.asset_for(host_key))
        .ok_or_else(|| anyhow::anyhow!("the pinned TinyBox release does not support this host"))?;
    let archive = artifact_root.join(asset.archive);
    anyhow::ensure!(
        archive.is_file(),
        "missing verified release archive: {archive:?}"
    );

    let bundled = tempfile::tempdir()?;
    let bundle_asset_dir = bundled
        .path()
        .join(record.id)
        .join(record.version)
        .join(asset.host_key);
    std::fs::create_dir_all(&bundle_asset_dir)?;
    let archive_file = std::fs::File::open(&archive)?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    tar::Archive::new(decoder).unpack(&bundle_asset_dir)?;
    std::fs::copy(&archive, bundle_asset_dir.join(asset.archive))?;
    std::fs::write(
        bundle_asset_dir.join(format!("{}.sha256", asset.archive)),
        format!("{}\n", asset.sha256),
    )?;
    std::env::set_var(KEY, bundled.path());

    let install = tempfile::tempdir()?;
    let mut config = crate::config::Config::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    config.modules.install_dir = Some(install.path().display().to_string());
    crate::modules::ops::ensure_loaded(&config, "tinybox")
        .await
        .map_err(|_| anyhow::anyhow!("the pinned TinyBox release could not be admitted"))?;

    // Drive the real bus lifecycle for the unjailed runs below too.
    let _live = LiveModule::enable();
    let workspace = tempfile::tempdir()?;
    let state = tempfile::tempdir()?;
    let policy = SandboxPolicy {
        backend: super::super::types::SandboxBackendKind::Local,
        workspace_root: workspace.path().to_path_buf(),
        state_dir: state.path().to_path_buf(),
        read_only_mounts: Vec::new(),
        read_write_mounts: Vec::new(),
        allow_network: true,
        mount_workspace: true,
        env_passthrough: Vec::new(),
        docker_overrides: None,
    };
    let output = execute(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &policy,
        &planned_test_jail(),
        "printf tinybox-jailed-ok; printf safe-error >&2",
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(10),
        false,
        None,
        None,
    )
    .await?;
    let output = finished(output);
    assert!(output.success());
    assert_eq!(output.stdout, "tinybox-jailed-ok");
    assert_eq!(output.stderr, "safe-error");

    let output = execute(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &policy,
        &planned_test_jail(),
        "i=0; while [ $i -lt 1500 ]; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; i=$((i + 1)); done",
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(10),
        false,
        None,
        None,
    )
    .await?;
    let output = finished(output);
    assert!(output.success());
    assert_eq!(
        output.stdout.len(),
        150_000,
        "all terminal output batches are read"
    );

    let output = super::super::shell_exec::run_host(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        "printf cron-module-ok; printf cron-stderr >&2",
        workspace.path(),
        state.path(),
        BTreeMap::new(),
        Duration::from_secs(10),
        true,
        None,
    )
    .await?;
    assert!(output.success());
    assert_eq!(output.stdout, "cron-module-ok");
    assert_eq!(output.stderr, "cron-stderr");

    let output = super::super::shell_exec::run_host(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        "false | true",
        workspace.path(),
        state.path(),
        BTreeMap::new(),
        Duration::from_secs(10),
        true,
        None,
    )
    .await?;
    assert!(output.success(), "cron preserves default pipeline status");

    let output = super::super::shell_exec::run_host(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        "sleep 10",
        workspace.path(),
        state.path(),
        BTreeMap::new(),
        Duration::from_millis(50),
        true,
        None,
    )
    .await?;
    assert!(output.timed_out, "cron timeout is acknowledged by TinyBox");
    Ok(())
}

/// The result of a jailed run that was not asked to yield.
fn finished(
    outcome: super::super::shell_exec::ShellRunOutcome,
) -> super::super::types::SandboxExecResult {
    match outcome {
        super::super::shell_exec::ShellRunOutcome::Finished(result) => result,
        super::super::shell_exec::ShellRunOutcome::Running(_) => {
            panic!("a run without a yield point came back while running")
        }
    }
}
