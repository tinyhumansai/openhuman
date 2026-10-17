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
fn native_jail_request_preserves_grants_and_requires_enforcement() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let readonly = tempfile::tempdir()?;
    let readwrite = tempfile::tempdir()?;
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
    let account = tempfile::tempdir()?;
    let mut config = Config::default();
    config.config_path = account.path().join("config.toml");
    std::fs::write(&config.config_path, "")?;
    let _keyring = crate::security::secret_storage::store::test_scope::ScopedWorkspace::new(
        account.path().join("workspace"),
    );
    let request = jail_request(&config, &policy, SandboxIsolation::Isolated)?;

    assert_eq!(request.read_only, [readonly.path().to_str().unwrap()]);
    // TinyBox appends its own per-call scratch directory to the writable grants.
    assert_eq!(request.read_write, [readwrite.path().to_str().unwrap()]);
    assert!(!request.allow_net);
    assert!(request.allow_subprocess);
    assert_eq!(request.enforcement, contract::JailEnforcement::Enforced);
    assert!(request.require_process_isolation);

    // Folder confinement keeps the grants and the network decision, and asks
    // TinyBox for what Seatbelt can give: declared constraints, no process
    // isolation.
    let confined = jail_request(&config, &policy, SandboxIsolation::FolderConfined)?;
    assert_eq!(confined.read_only, request.read_only);
    assert_eq!(confined.read_write, request.read_write);
    assert!(!confined.allow_net);
    assert_eq!(confined.enforcement, contract::JailEnforcement::Declared);
    assert!(!confined.require_process_isolation);

    // Landlock is deny-default, so the isolated jail needs no read denials;
    // Seatbelt reads allow-default, so folder confinement names the
    // credential stores this host has and OpenHuman's own credential files.
    assert!(request.deny_read.is_empty());
    let account_config = config.config_path.canonicalize()?;
    assert!(confined
        .deny_read
        .contains(&account_config.to_str().unwrap().to_owned()));
    for denied in &confined.deny_read {
        let denied = std::path::Path::new(denied);
        assert!(
            crate::security::SecurityPolicy::is_always_forbidden(denied)
                || denied == account_config,
            "{} is not a protected credential store",
            denied.display()
        );
    }
    Ok(())
}

/// A home folder holding the given credential stores: a trailing `/` makes a
/// directory, anything else a file.
fn home_with(stores: &[&str]) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    for store in stores {
        let path = home.path().join(store.trim_end_matches('/'));
        if store.ends_with('/') {
            std::fs::create_dir_all(&path).unwrap();
        } else {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "secret").unwrap();
        }
    }
    home
}

fn real(home: &tempfile::TempDir, store: &str) -> String {
    home.path()
        .join(store)
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned()
}

#[test]
fn folder_confinement_denies_the_credential_stores_that_exist() {
    let home = home_with(&[
        ".ssh/",
        ".aws/",
        "Library/Keychains/",
        ".netrc",
        ".cargo/credentials.toml",
    ]);
    let root = tempfile::tempdir().unwrap();

    let denied = credential_denials(
        crate::security::policy::home_credential_stores(home.path()),
        root.path(),
        &[],
    )
    .unwrap();

    // Absent stores (`.gnupg`, `.kube`, `.npmrc`, ...) are not sent.
    assert_eq!(
        denied,
        [
            real(&home, ".ssh"),
            real(&home, ".aws"),
            real(&home, "Library/Keychains"),
            real(&home, ".netrc"),
            real(&home, ".cargo/credentials.toml"),
        ]
    );
}

#[test]
fn a_workspace_inside_a_credential_store_is_refused() {
    let home = home_with(&[".ssh/", ".aws/project/"]);

    // Denying `~/.aws` would take the workspace away, and skipping it would
    // leave `~/.aws/credentials` readable, so the jail is refused.
    let err = credential_denials(
        crate::security::policy::home_credential_stores(home.path()),
        &home.path().join(".aws/project"),
        &[],
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains("`.aws`"), "{err}");
    // The error names the store, never the user's path to it.
    assert!(!err.contains(home.path().to_str().unwrap()), "{err}");
}

#[test]
fn a_writable_grant_inside_a_credential_store_is_refused() {
    let home = home_with(&[".kube/"]);
    let root = tempfile::tempdir().unwrap();

    let err = credential_denials(
        crate::security::policy::home_credential_stores(home.path()),
        root.path(),
        &[home.path().join(".kube")],
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains("`.kube`"), "{err}");
}

#[test]
fn a_store_inside_a_writable_grant_is_still_denied() {
    let home = home_with(&[".ssh/", ".cargo/credentials.toml"]);
    let root = tempfile::tempdir().unwrap();

    // Seatbelt carves the store out of the grant; a store that holds nothing
    // of the jail's is denied as usual.
    let denied = credential_denials(
        crate::security::policy::home_credential_stores(home.path()),
        root.path(),
        &[home.path().join(".cargo")],
    )
    .unwrap();

    assert_eq!(
        denied,
        [real(&home, ".ssh"), real(&home, ".cargo/credentials.toml")]
    );
}

#[test]
fn nothing_to_deny_means_no_credential_denials() {
    let root = tempfile::tempdir().unwrap();
    assert!(credential_denials(Vec::new(), root.path(), &[])
        .unwrap()
        .is_empty());
}

#[test]
fn openhumans_own_credential_files_are_denied_when_present() {
    // The account folder: config.toml and the credential stores beside it, the
    // core workspace below it with the keyring file and TinyBox's scratch.
    let account = tempfile::tempdir().unwrap();
    let workspace = account.path().join("workspace");
    let scratch = workspace.join("artifacts/sandbox-scratch/shell-1");
    std::fs::create_dir_all(&scratch).unwrap();
    let mut config = Config::default();
    config.config_path = account.path().join("config.toml");
    let _keyring =
        crate::security::secret_storage::store::test_scope::ScopedWorkspace::new(&workspace);
    let files = [
        config.config_path.clone(),
        account.path().join("auth-profiles.json"),
        account.path().join("http-credentials.json"),
        workspace.join("secrets.enc"),
    ];
    for file in &files {
        std::fs::write(file, "secret").unwrap();
    }
    let root = tempfile::tempdir().unwrap();

    let denied = credential_denials(
        crate::security::credentials::credential_files(&config),
        root.path(),
        &[scratch.clone()],
    )
    .unwrap();

    let mut expected: Vec<String> = files
        .iter()
        .map(|file| file.canonicalize().unwrap().to_str().unwrap().to_owned())
        .collect();
    let mut sorted = denied.clone();
    sorted.sort();
    expected.sort();
    assert_eq!(sorted, expected);
    // Files only: none of them is, or contains, the jail's scratch directory.
    let scratch = scratch.canonicalize().unwrap();
    for file in &denied {
        assert!(!scratch.starts_with(file), "{file} covers the scratch dir");
    }

    // A credential file that does not exist is not sent.
    std::fs::remove_file(account.path().join("http-credentials.json")).unwrap();
    let denied = credential_denials(
        crate::security::credentials::credential_files(&config),
        root.path(),
        &[],
    )
    .unwrap();
    assert_eq!(denied.len(), 3);
    assert!(!denied
        .iter()
        .any(|file| file.ends_with("http-credentials.json")));
}

#[test]
fn every_denied_store_is_one_the_path_floor_forbids() {
    let home = std::path::Path::new("/home/someone");
    let stores = crate::security::policy::home_credential_stores(home);

    assert!(stores.contains(&home.join(".ssh")));
    assert!(stores.contains(&home.join("Library/Keychains")));
    assert!(stores.contains(&home.join(".git-credentials")));
    for store in &stores {
        assert!(
            crate::security::SecurityPolicy::is_always_forbidden(store),
            "{} is not on the is_always_forbidden floor",
            store.display()
        );
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
        env_passthrough: Vec::new(),
        docker_overrides: None,
    };
    let mut config = crate::config::Config::default();
    config.modules.enabled = false;

    let result = execute(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &policy,
        &format!("touch '{}'", marker.display()),
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(5),
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
#[ignore = "requires the verified TinyBox v0.1.28 archive and a supported Linux jail"]
async fn released_tinybox_runs_a_native_jail_through_the_pinned_loader() -> anyhow::Result<()> {
    let _environment_lock = crate::config::TEST_ENV_LOCK.lock().await;
    const KEY: &str = "OPENHUMAN_BUNDLED_MODULES";
    let artifact_root = std::env::var_os(KEY).map(PathBuf::from).ok_or_else(|| {
        anyhow::anyhow!("set OPENHUMAN_BUNDLED_MODULES to the downloaded v0.1.28 release assets")
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
    };
    let output = execute(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &policy,
        "printf tinybox-jailed-ok; printf safe-error >&2",
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(10),
        false,
        None,
    )
    .await?;
    assert!(output.success());
    assert_eq!(output.stdout, "tinybox-jailed-ok");
    assert_eq!(output.stderr, "safe-error");

    let output = execute(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &policy,
        "i=0; while [ $i -lt 1500 ]; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; i=$((i + 1)); done",
        workspace.path(),
        &HashMap::new(),
        Duration::from_secs(10),
        false,
        None,
    )
    .await?;
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
