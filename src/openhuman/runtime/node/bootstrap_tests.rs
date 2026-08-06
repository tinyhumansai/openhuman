use super::*;

fn touch(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, b"#!/bin/sh\n").unwrap();
}

fn managed_config(cache_root: &Path) -> NodeConfig {
    NodeConfig {
        enabled: true,
        version: NodeConfig::default().version,
        cache_dir: cache_root.to_string_lossy().to_string(),
        // Force the managed path so the probe never depends on a host node.
        prefer_system: false,
    }
}

/// GH-5047: a warm restart is a fresh process, so the in-memory
/// `try_cached` memo is empty. The durable probe must still recover
/// readiness from the on-disk managed install — otherwise `is_done` reports
/// "not ready" every launch and the harness-init overlay re-appears.
#[tokio::test]
async fn probe_installed_true_from_disk_after_simulated_restart() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_root = tmp.path();

    let config = managed_config(cache_root);
    let dist = NodeDistribution::for_host(&config.version).expect("host arch supported");
    let bootstrap = NodeBootstrap::new(config, tmp.path().join("ws"), Client::new());

    // Lay down a managed install exactly where the bootstrap expects it.
    let bin_dir = managed_bin_dir(&bootstrap.install_dir(&dist));
    let (node_name, npm_name) = if cfg!(windows) {
        ("node.exe", "npm.cmd")
    } else {
        ("node", "npm")
    };
    touch(&bin_dir.join(node_name));
    touch(&bin_dir.join(npm_name));

    // Simulated cold process: nothing memoised yet.
    assert!(
        bootstrap.try_cached().is_none(),
        "precondition: process-local cache is empty right after a restart"
    );

    // The durable probe recovers readiness from disk (and never downloads).
    assert!(
        bootstrap.probe_installed().await.is_some(),
        "durable probe should detect the on-disk managed node install"
    );
    assert!(
        bootstrap.try_cached().is_some(),
        "a probe hit should memoise into the shared cache for the rest of the process"
    );
}

/// A fresh machine (empty cache, no install) must report "not installed" so
/// a genuine first-run download still runs and the overlay still shows.
#[tokio::test]
async fn probe_installed_none_when_nothing_on_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bootstrap = NodeBootstrap::new(
        managed_config(tmp.path()),
        tmp.path().join("ws"),
        Client::new(),
    );
    assert!(
        bootstrap.probe_installed().await.is_none(),
        "no on-disk install → provisioning still required"
    );
}

/// A disabled runtime is "nothing to provision", not "installed".
#[tokio::test]
async fn probe_installed_none_when_disabled() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = managed_config(tmp.path());
    config.enabled = false;
    let bootstrap = NodeBootstrap::new(config, tmp.path().join("ws"), Client::new());
    assert!(bootstrap.probe_installed().await.is_none());
}
