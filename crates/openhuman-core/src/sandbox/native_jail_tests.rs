//! Host-side request mapping for TinyBox native-jail execution.

use super::*;
#[cfg(all(feature = "modules", target_os = "linux"))]
use std::path::PathBuf;

#[test]
fn jail_status_is_ready_only_for_an_accepted_jail() {
    use super::super::ops::local_status_for_jail;
    assert_eq!(
        local_status_for_jail(Some(SandboxIsolation::Isolated)),
        SandboxStatus::Ready
    );
    assert_eq!(
        local_status_for_jail(Some(SandboxIsolation::FolderConfined)),
        SandboxStatus::Ready
    );
    assert_eq!(local_status_for_jail(None), SandboxStatus::Inactive);
}

#[test]
fn kernel_isolation_is_accepted_for_every_caller() {
    for untrusted in [false, true] {
        assert_eq!(
            jail_isolation("landlock", true, true, untrusted),
            Some(SandboxIsolation::Isolated)
        );
    }
}

#[test]
fn seatbelt_confines_the_local_desktop_sandbox_to_its_folder() {
    assert_eq!(
        jail_isolation("seatbelt", true, false, false),
        Some(SandboxIsolation::FolderConfined)
    );
}

#[test]
fn seatbelt_is_refused_where_untrusted_code_needs_isolation() {
    // An explicitly requested Landlock jail, or a SaaS tenant.
    assert_eq!(jail_isolation("seatbelt", true, false, true), None);
}

#[test]
fn an_unavailable_or_unconfining_jail_is_refused() {
    for untrusted in [false, true] {
        // Missing launcher: nothing would be confined.
        assert_eq!(jail_isolation("seatbelt", false, false, untrusted), None);
        assert_eq!(jail_isolation("landlock", false, true, untrusted), None);
        // The no-op backend confines nothing.
        assert_eq!(jail_isolation("noop", true, false, untrusted), None);
        // A Landlock host below the kernel floor is not folder confinement.
        assert_eq!(jail_isolation("landlock", true, false, untrusted), None);
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

/// A jail with no extra grants, standing in for the module's plan.
#[cfg(feature = "modules")]
fn open_jail() -> contract::NativeJailConfig {
    contract::NativeJailConfig {
        read_only: Vec::new(),
        read_write: Vec::new(),
        allow_net: true,
        allow_subprocess: true,
        enforcement: contract::JailEnforcement::Enforced,
        require_process_isolation: true,
        deny_read: Vec::new(),
        resources: None,
    }
}

#[cfg(feature = "modules")]
fn open_planned_jail() -> PlannedJail {
    PlannedJail {
        config: open_jail(),
    }
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
        env_passthrough: Vec::new(),
        docker_overrides: None,
        mount_workspace: true,
    };
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;

    let result = execute(
        &config,
        &policy,
        &format!("touch '{}'", marker.display()),
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(5),
        open_planned_jail(),
        false,
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
        env_passthrough: Vec::new(),
        docker_overrides: None,
        mount_workspace: true,
    };
    let output = execute(
        &config,
        &policy,
        "printf tinybox-jailed-ok; printf safe-error >&2",
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(10),
        open_planned_jail(),
        false,
        None,
    )
    .await?;
    let output = output;
    assert!(output.success());
    assert_eq!(output.stdout, "tinybox-jailed-ok");
    assert_eq!(output.stderr, "safe-error");

    let output = execute(
        &config,
        &policy,
        "i=0; while [ $i -lt 1500 ]; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; i=$((i + 1)); done",
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(10),
        open_planned_jail(),
        false,
        None,
    )
    .await?;
    let output = output;
    assert!(output.success());
    assert_eq!(
        output.stdout.len(),
        150_000,
        "all terminal output batches are read"
    );

    let output = super::super::shell_exec::run_host(
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
