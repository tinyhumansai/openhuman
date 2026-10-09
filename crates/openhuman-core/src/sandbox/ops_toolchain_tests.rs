use super::*;

#[cfg(unix)]
#[tokio::test]
async fn local_jail_forwards_nonempty_toolchain_homes_without_cargo() {
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let rustup_home = tempfile::tempdir().unwrap();
    let cargo_home = tempfile::tempdir().unwrap();
    let rustup_value = rustup_home.path().to_string_lossy().into_owned();
    let cargo_value = cargo_home.path().to_string_lossy().into_owned();
    let _env = crate::config::test_env::EnvVarGuard::locked_async()
        .await
        .with("RUSTUP_HOME", &rustup_value)
        .with("CARGO_HOME", &cargo_value);
    let policy = local_policy(action.path(), state.path());

    // These paths exercise forwarding alone. The child only prints them; no
    // Cargo command consumes them as though they held a real toolchain.
    let result = run_local(
        &policy,
        "printf '%s\\n' \"${RUSTUP_HOME-}\" \"${CARGO_HOME-}\"",
    )
    .await;
    assert!(
        result.success(),
        "toolchain-home probe failed: {}",
        result.stderr
    );
    assert_eq!(
        result.stdout,
        format!("{rustup_value}\n{cargo_value}\n"),
        "local sandbox must forward explicitly configured nonempty toolchain homes"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn landlock_custom_toolchain_homes_keep_selective_access() {
    if !landlock_in_force() {
        return;
    }
    let action = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let rustup_home = tempfile::tempdir().unwrap();
    let cargo_home = tempfile::tempdir().unwrap();
    let (Some(rustup_value), Some(cargo_value)) = (
        rustup_home.path().to_str().map(str::to_owned),
        cargo_home.path().to_str().map(str::to_owned),
    ) else {
        eprintln!("SKIP: generated toolchain fixture path is not Unicode");
        return;
    };
    for child in ["bin", "registry", "git"] {
        std::fs::create_dir(cargo_home.path().join(child)).unwrap();
    }
    std::fs::write(rustup_home.path().join("toolchain"), "rust-fixture").unwrap();
    std::fs::write(cargo_home.path().join("bin/tool"), "cargo-fixture").unwrap();
    std::fs::write(cargo_home.path().join("credentials.toml"), "fixture-secret").unwrap();
    let _env = crate::config::test_env::EnvVarGuard::locked_async()
        .await
        .with("RUSTUP_HOME", &rustup_value)
        .with("CARGO_HOME", &cargo_value);
    let policy = local_policy(action.path(), state.path());
    let reads = run_local(
        &policy,
        "cat \"$RUSTUP_HOME/toolchain\"; cat \"$CARGO_HOME/bin/tool\"",
    )
    .await;
    assert!(
        reads.success(),
        "custom toolchain reads failed: {}",
        reads.stderr
    );
    assert_eq!(reads.stdout, "rust-fixturecargo-fixture");
    let caches = run_local(
        &policy,
        "printf registry > \"$CARGO_HOME/registry/probe\" && printf git > \"$CARGO_HOME/git/probe\"",
    )
    .await;
    assert!(
        caches.success(),
        "Cargo cache writes failed: {}",
        caches.stderr
    );
    for command in [
        "printf denied > \"$RUSTUP_HOME/toolchain\"",
        "printf denied > \"$CARGO_HOME/bin/tool\"",
        "cat \"$CARGO_HOME/credentials.toml\"",
        "printf denied > \"$CARGO_HOME/credentials.toml\"",
    ] {
        let result = run_local(&policy, command).await;
        assert!(!result.success(), "selective access allowed {command}");
    }
    assert_eq!(
        std::fs::read_to_string(rustup_home.path().join("toolchain")).unwrap(),
        "rust-fixture"
    );
    assert_eq!(
        std::fs::read_to_string(cargo_home.path().join("bin/tool")).unwrap(),
        "cargo-fixture"
    );
    assert_eq!(
        std::fs::read_to_string(cargo_home.path().join("registry/probe")).unwrap(),
        "registry"
    );
    assert_eq!(
        std::fs::read_to_string(cargo_home.path().join("git/probe")).unwrap(),
        "git"
    );

    drop(_env);
    let alias_cargo = tempfile::tempdir().unwrap();
    std::fs::create_dir(alias_cargo.path().join("bin")).unwrap();
    std::fs::create_dir(alias_cargo.path().join("git")).unwrap();
    std::os::unix::fs::symlink(rustup_home.path(), alias_cargo.path().join("registry")).unwrap();
    let Some(alias_cargo_value) = alias_cargo.path().to_str().map(str::to_owned) else {
        eprintln!("SKIP: generated Cargo alias fixture path is not Unicode");
        return;
    };
    let _env = crate::config::test_env::EnvVarGuard::locked_async()
        .await
        .with("RUSTUP_HOME", &rustup_value)
        .with("CARGO_HOME", &alias_cargo_value);
    let alias_policy = local_policy(action.path(), state.path());
    let cache_alias_write = run_local(
        &alias_policy,
        "printf denied > \"$CARGO_HOME/registry/toolchain\"",
    )
    .await;
    assert!(
        !cache_alias_write.success(),
        "Cargo cache alias wrote through to RUSTUP_HOME"
    );
    assert_eq!(
        std::fs::read_to_string(rustup_home.path().join("toolchain")).unwrap(),
        "rust-fixture"
    );
    let alias_cache_write =
        run_local(&alias_policy, "printf git > \"$CARGO_HOME/git/probe\"").await;
    assert!(
        alias_cache_write.success(),
        "dedicated Cargo git cache lost RW access: {}",
        alias_cache_write.stderr
    );
    assert_eq!(
        std::fs::read_to_string(alias_cargo.path().join("git/probe")).unwrap(),
        "git"
    );
}
