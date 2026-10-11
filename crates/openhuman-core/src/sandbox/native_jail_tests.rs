//! Host-side request mapping for TinyBox native-jail execution.

use super::*;
#[cfg(all(feature = "modules", target_os = "linux"))]
use std::path::PathBuf;

#[test]
fn jail_status_is_ready_only_above_the_isolation_floor() {
    assert_eq!(
        super::super::ops::local_status_for_jail(true, true),
        SandboxStatus::Ready
    );
    assert_eq!(
        super::super::ops::local_status_for_jail(false, true),
        SandboxStatus::Inactive
    );
    assert_eq!(
        super::super::ops::local_status_for_jail(true, false),
        SandboxStatus::Inactive
    );
}

#[cfg(feature = "modules")]
#[test]
fn shell_request_uses_the_platform_shell_and_caller_known_process_resource() -> anyhow::Result<()> {
    let resource = contract::ResourceId("known-resource".to_owned());
    let cwd = tempfile::tempdir()?;
    let env = BTreeMap::from([("LANG".to_owned(), "C".to_owned())]);
    let request = shell_request(
        &resource,
        "printf 'one two'",
        cwd.path(),
        env.clone(),
        false,
    )?;

    let expected = platform_shell::build_std_command("printf 'one two'");
    let expected_argv = std::iter::once(expected.get_program().to_str().unwrap().to_owned())
        .chain(
            expected
                .get_args()
                .map(|arg| arg.to_str().unwrap().to_owned()),
        )
        .collect::<Vec<_>>();
    assert_eq!(request.resource, resource);
    assert_eq!(request.argv, expected_argv);
    assert_eq!(request.cwd.as_deref(), cwd.path().to_str());
    assert_eq!(request.env, env);
    assert_eq!(request.stdin, None);
    Ok(())
}

#[cfg(feature = "modules")]
#[test]
fn native_jail_request_preserves_grants_and_requires_enforcement() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let readonly = tempfile::tempdir()?;
    let readwrite = tempfile::tempdir()?;
    let scratch = tempfile::tempdir()?;
    let policy = SandboxPolicy {
        backend: super::super::types::SandboxBackendKind::Local,
        workspace_root: root.path().to_path_buf(),
        state_dir: root.path().to_path_buf(),
        read_only_mounts: vec![readonly.path().to_path_buf()],
        read_write_mounts: vec![readwrite.path().to_path_buf()],
        allow_network: false,
        env_passthrough: Vec::new(),
        docker_overrides: None,
    };
    let (_, writable_roots) = execution_environment(&policy, &HashMap::new(), scratch.path())?;
    let request = jail_request(&policy, writable_roots)?;

    assert_eq!(request.read_only, [readonly.path().to_str().unwrap()]);
    assert_eq!(
        request.read_write,
        [
            readwrite.path().to_str().unwrap(),
            scratch.path().to_str().unwrap()
        ]
    );
    assert!(!request.allow_net);
    assert!(request.allow_subprocess);
    assert_eq!(request.enforcement, contract::JailEnforcement::Enforced);
    assert!(request.require_process_isolation);
    Ok(())
}

#[cfg(feature = "modules")]
#[test]
fn native_jail_environment_keeps_the_sandbox_allowlist_and_scratch_grant() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let scratch = tempfile::tempdir()?;
    let policy = SandboxPolicy {
        backend: super::super::types::SandboxBackendKind::Local,
        workspace_root: root.path().to_path_buf(),
        state_dir: root.path().to_path_buf(),
        read_only_mounts: Vec::new(),
        read_write_mounts: Vec::new(),
        allow_network: true,
        env_passthrough: Vec::new(),
        docker_overrides: None,
    };
    let extra = HashMap::from([(OsString::from("CUSTOM_SAFE"), OsString::from("value"))]);
    let (env, read_write) = execution_environment(&policy, &extra, scratch.path())?;

    assert_eq!(env.get("CUSTOM_SAFE").map(String::as_str), Some("value"));
    assert_eq!(
        env.get("TMPDIR").map(String::as_str),
        scratch.path().to_str()
    );
    assert!(read_write
        .iter()
        .any(|path| path == scratch.path().to_str().unwrap()));
    assert!(!env
        .keys()
        .any(|name| name.contains("TOKEN") || name.contains("SECRET")));
    Ok(())
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
        false,
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

#[cfg(all(feature = "modules", target_os = "linux"))]
#[tokio::test]
#[ignore = "requires the verified TinyBox v0.1.20 archive and a supported Linux jail"]
async fn released_tinybox_runs_a_native_jail_through_the_pinned_loader() -> anyhow::Result<()> {
    let _environment_lock = crate::config::TEST_ENV_LOCK.lock().await;
    const KEY: &str = "OPENHUMAN_BUNDLED_MODULES";
    let artifact_root = std::env::var_os(KEY).map(PathBuf::from).ok_or_else(|| {
        anyhow::anyhow!("set OPENHUMAN_BUNDLED_MODULES to the downloaded v0.1.20 release assets")
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
    };
    let output = execute(
        &config,
        &policy,
        "printf tinybox-jailed-ok; printf safe-error >&2",
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(10),
        false,
    )
    .await?;
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
        false,
    )
    .await?;
    assert!(output.success());
    assert_eq!(
        output.stdout.len(),
        150_000,
        "all terminal output batches are read"
    );

    let output = execute_passthrough(
        &config,
        "printf cron-module-ok; printf cron-stderr >&2",
        workspace.path(),
        Duration::from_secs(10),
    )
    .await?;
    assert!(output.success());
    assert_eq!(output.stdout, "cron-module-ok");
    assert_eq!(output.stderr, "cron-stderr");

    let output = execute_passthrough(
        &config,
        "false | true",
        workspace.path(),
        Duration::from_secs(10),
    )
    .await?;
    assert!(output.success(), "cron preserves default pipeline status");

    let output = execute_passthrough(
        &config,
        "sleep 10",
        workspace.path(),
        Duration::from_millis(50),
    )
    .await?;
    assert!(output.timed_out, "cron timeout is acknowledged by TinyBox");
    Ok(())
}
