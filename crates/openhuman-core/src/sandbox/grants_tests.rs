use super::*;
use crate::config::test_env::EnvVarGuard;
use std::fs;
use std::path::{Path, PathBuf};

fn fake_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    for d in [
        ".ssh", ".gnupg", ".aws", ".cargo", ".rustup", ".nvm", ".npm",
    ] {
        fs::create_dir_all(home.path().join(d)).unwrap();
    }
    home
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap()
}

fn has(grants: &[PathBuf], p: &Path) -> bool {
    grants.iter().any(|g| g == p)
}

fn all(g: &JailGrants) -> Vec<PathBuf> {
    g.read_only.iter().chain(&g.read_write).cloned().collect()
}

fn isolated_toolchain_env() -> EnvVarGuard {
    EnvVarGuard::locked()
        .without("RUSTUP_HOME")
        .without("CARGO_HOME")
}

/// A grant reaches a credential store if it IS one, is inside one, or contains
/// one (Landlock grants are recursive, so a parent grant exposes the child).
fn reaches_credentials(grants: &[PathBuf], home: &Path) -> Option<PathBuf> {
    for cred in [".ssh", ".gnupg", ".aws"] {
        let cred = home.join(cred);
        for g in grants {
            if g.starts_with(&cred) || cred.starts_with(g) {
                return Some(g.clone());
            }
        }
    }
    None
}

#[test]
fn credential_dirs_are_never_granted_by_default() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    assert_eq!(reaches_credentials(&all(&g), home.path()), None);
}

#[test]
fn credential_dirs_are_never_granted_even_when_extras_ask() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    let cfg = LocalJailConfig {
        extra_read_only: vec![
            "~/.ssh".into(),
            "~/.gnupg/".into(),
            format!("{}", home.path().join(".aws").display()),
            "~".into(),
            format!("{}", home.path().display()),
            "/".into(),
        ],
        extra_read_write: vec!["~/.ssh".into(), "~".into()],
        ..LocalJailConfig::default()
    };
    let g = resolve_local_jail_grants(Some(home.path()), &cfg);
    assert_eq!(reaches_credentials(&all(&g), home.path()), None);
    assert!(!g.read_only.iter().any(|p| p == Path::new("/")));
}

#[test]
fn credential_floor_holds_for_a_symlink_into_a_credential_dir() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    std::os::unix::fs::symlink(home.path().join(".ssh"), home.path().join("innocent")).unwrap();
    let cfg = LocalJailConfig {
        extra_read_only: vec!["~/innocent".into()],
        ..LocalJailConfig::default()
    };
    let g = resolve_local_jail_grants(Some(home.path()), &cfg);
    assert_eq!(reaches_credentials(&all(&g), home.path()), None);
}

#[test]
fn toolchain_homes_are_granted_when_they_exist() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    let cargo = home.path().join(".cargo");
    for dir in ["bin", "registry", "git"] {
        fs::create_dir_all(cargo.join(dir)).unwrap();
    }
    for file in ["config.toml", "config", "env"] {
        fs::write(cargo.join(file), "").unwrap();
    }
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let h = canon(home.path());
    let cargo = h.join(".cargo");
    assert!(
        !has(&g.read_write, &cargo),
        "Cargo home must never be writable"
    );
    assert!(has(&g.read_only, &cargo.join("bin")));
    assert!(has(&g.read_write, &cargo.join("registry")));
    assert!(has(&g.read_write, &cargo.join("git")));
    for file in ["config.toml", "config", "env"] {
        assert!(
            has(&g.read_only, &cargo.join(file)),
            "{file} should be read-only"
        );
    }
    for ro in [".rustup", ".nvm", ".npm"] {
        assert!(has(&g.read_only, &h.join(ro)), "{ro} should be read-only");
        assert!(
            !has(&g.read_write, &h.join(ro)),
            "{ro} must not be writable"
        );
    }
    for sys in ["/usr/local", "/opt"] {
        if Path::new(sys).exists() {
            assert!(has(&g.read_only, &canon(Path::new(sys))), "{sys} missing");
        }
    }
}

#[test]
fn configured_absolute_rust_homes_outside_home_use_selective_grants() {
    let home = fake_home();
    let install = tempfile::tempdir().unwrap();
    let rustup = install.path().join("rustup-home");
    let cargo = install.path().join("cargo-home");
    fs::create_dir_all(&rustup).unwrap();
    for dir in ["bin", "registry", "git"] {
        fs::create_dir_all(cargo.join(dir)).unwrap();
    }
    for file in ["config.toml", "config", "env", "credentials.toml"] {
        fs::write(cargo.join(file), "").unwrap();
    }

    let _env = EnvVarGuard::locked()
        .with("RUSTUP_HOME", &rustup)
        .with("CARGO_HOME", &cargo);
    let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let rustup = canon(&rustup);
    let cargo = canon(&cargo);

    assert!(has(&grants.read_only, &rustup));
    assert!(!has(&grants.read_write, &rustup));
    assert!(!all(&grants).iter().any(|path| path == &cargo));
    assert!(has(&grants.read_only, &cargo.join("bin")));
    for file in ["config.toml", "config", "env"] {
        assert!(
            has(&grants.read_only, &cargo.join(file)),
            "{file} should be read-only"
        );
    }
    assert!(has(&grants.read_write, &cargo.join("registry")));
    assert!(has(&grants.read_write, &cargo.join("git")));
    assert!(!all(&grants)
        .iter()
        .any(|path| path == &cargo.join("credentials.toml")));
    assert_eq!(reaches_credentials(&all(&grants), home.path()), None);
}

#[cfg(unix)]
#[test]
fn cargo_symlink_aliases_cannot_reach_root_or_credentials() {
    let home = fake_home();
    let install = tempfile::tempdir().unwrap();
    let cargo = install.path().join("cargo-home");
    let external_cache = tempfile::tempdir().unwrap();
    let cache = external_cache.path().join("dedicated-cache");
    fs::create_dir_all(cargo.join("bin")).unwrap();
    fs::create_dir_all(&cache).unwrap();
    fs::write(cargo.join("credentials.toml"), "token = \"secret\"\n").unwrap();
    std::os::unix::fs::symlink(&cargo, cargo.join("registry")).unwrap();
    std::os::unix::fs::symlink(cargo.join("credentials.toml"), cargo.join("config.toml")).unwrap();
    std::os::unix::fs::symlink(&cache, cargo.join("git")).unwrap();

    // The parent also contains Cargo credentials, so a broad explicit
    // RUSTUP_HOME grant must be refused while the narrow cache remains usable.
    let _env = EnvVarGuard::locked()
        .with("RUSTUP_HOME", install.path())
        .with("CARGO_HOME", &cargo);
    let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let cargo = canon(&cargo);
    let cache = canon(&cache);
    let credential = canon(&cargo.join("credentials.toml"));

    assert!(!has(&grants.read_write, &cargo));
    assert!(!has(&grants.read_only, install.path()));
    assert!(!all(&grants)
        .iter()
        .any(|path| path == &credential || path.starts_with(&credential)));
    assert!(has(&grants.read_only, &cargo.join("bin")));
    assert!(has(&grants.read_write, &cache));

    drop(_env);
    let mode_cargo = install.path().join("mode-cargo-home");
    fs::create_dir_all(mode_cargo.join("bin")).unwrap();
    std::os::unix::fs::symlink(mode_cargo.join("bin"), mode_cargo.join("registry")).unwrap();
    let _env = EnvVarGuard::locked()
        .without("RUSTUP_HOME")
        .with("CARGO_HOME", &mode_cargo);
    let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let bin = canon(&mode_cargo.join("bin"));
    assert!(has(&grants.read_only, &bin));
    assert!(!has(&grants.read_write, &bin));
}

#[cfg(unix)]
#[test]
fn cargo_cache_aliases_cannot_upgrade_configured_rustup_home() {
    let home = fake_home();
    let install = tempfile::tempdir().unwrap();
    let rustup = install.path().join("rustup-home");
    let cache = install.path().join("dedicated-cache");
    fs::create_dir_all(rustup.join("toolchains")).unwrap();
    fs::create_dir_all(&cache).unwrap();

    for (name, target) in [
        ("equal", rustup.clone()),
        ("descendant", rustup.join("toolchains")),
        ("ancestor", install.path().to_path_buf()),
    ] {
        let cargo = install.path().join(format!("cargo-{name}"));
        fs::create_dir_all(cargo.join("bin")).unwrap();
        std::os::unix::fs::symlink(target, cargo.join("registry")).unwrap();
        std::os::unix::fs::symlink(&cache, cargo.join("git")).unwrap();

        let _env = EnvVarGuard::locked()
            .with("RUSTUP_HOME", &rustup)
            .with("CARGO_HOME", &cargo);
        let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
        let rustup = canon(&rustup);
        let cache = canon(&cache);
        assert!(has(&grants.read_only, &rustup), "{name}: Rustup RO missing");
        assert!(
            grants
                .read_write
                .iter()
                .all(|path| !path.starts_with(&rustup) && !rustup.starts_with(path)),
            "{name}: Cargo cache RW overlaps Rustup: {:?}",
            grants.read_write
        );
        assert!(
            has(&grants.read_write, &cache),
            "{name}: external cache RW missing"
        );
        drop(_env);
    }

    let default_rustup = canon(&home.path().join(".rustup"));
    let cargo = install.path().join("cargo-default-rustup");
    fs::create_dir_all(cargo.join("bin")).unwrap();
    std::os::unix::fs::symlink(&default_rustup, cargo.join("registry")).unwrap();
    std::os::unix::fs::symlink(&cache, cargo.join("git")).unwrap();
    let _env = EnvVarGuard::locked()
        .without("RUSTUP_HOME")
        .with("CARGO_HOME", &cargo);
    let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    assert!(has(&grants.read_only, &default_rustup));
    assert!(grants
        .read_write
        .iter()
        .all(|path| { !path.starts_with(&default_rustup) && !default_rustup.starts_with(path) }));
    assert!(has(&grants.read_write, &canon(&cache)));
}

#[test]
fn custom_rust_homes_inside_credential_stores_are_refused() {
    let home = fake_home();
    let rustup = home.path().join(".ssh").join("rustup");
    let cargo = home.path().join(".aws").join("cargo");
    fs::create_dir_all(&rustup).unwrap();
    for dir in ["bin", "registry", "git"] {
        fs::create_dir_all(cargo.join(dir)).unwrap();
    }
    for file in ["config.toml", "config", "env"] {
        fs::write(cargo.join(file), "").unwrap();
    }

    let _env = EnvVarGuard::locked()
        .with("RUSTUP_HOME", &rustup)
        .with("CARGO_HOME", &cargo);
    let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());

    assert!(!all(&grants).iter().any(|path| path.starts_with(&rustup)));
    assert!(!all(&grants).iter().any(|path| path.starts_with(&cargo)));
    assert_eq!(reaches_credentials(&all(&grants), home.path()), None);
}

#[test]
fn custom_rust_homes_skip_relative_and_missing_paths() {
    let home = fake_home();
    let cwd = std::env::current_dir().unwrap();
    let relative_rustup = tempfile::Builder::new()
        .prefix("relative-rustup-home-")
        .tempdir_in(&cwd)
        .unwrap();
    let relative_cargo = tempfile::Builder::new()
        .prefix("relative-cargo-home-")
        .tempdir_in(&cwd)
        .unwrap();
    for dir in ["bin", "registry", "git"] {
        fs::create_dir_all(relative_cargo.path().join(dir)).unwrap();
    }
    let relative_rustup_name = relative_rustup.path().file_name().unwrap();
    let relative_cargo_name = relative_cargo.path().file_name().unwrap();
    assert!(Path::new(relative_rustup_name).is_relative());
    assert!(Path::new(relative_cargo_name).is_relative());

    {
        let _env = EnvVarGuard::locked()
            .with("RUSTUP_HOME", relative_rustup_name)
            .with("CARGO_HOME", relative_cargo_name);
        let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
        let rustup = canon(relative_rustup.path());
        let cargo = canon(relative_cargo.path());
        assert!(!all(&grants).iter().any(|path| path.starts_with(&rustup)));
        assert!(!all(&grants).iter().any(|path| path.starts_with(&cargo)));
    }

    let missing_root = tempfile::tempdir().unwrap();
    let missing_rustup = missing_root.path().join("missing-rustup");
    let missing_cargo = missing_root.path().join("missing-cargo");
    {
        let _env = EnvVarGuard::locked()
            .with("RUSTUP_HOME", &missing_rustup)
            .with("CARGO_HOME", &missing_cargo);
        let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
        assert!(!all(&grants)
            .iter()
            .any(|path| path.starts_with(&missing_rustup)));
        assert!(!all(&grants)
            .iter()
            .any(|path| path.starts_with(&missing_cargo)));
    }
}

#[test]
fn configured_rust_homes_deduplicate_existing_home_grants() {
    let home = fake_home();
    let cargo = home.path().join(".cargo");
    for dir in ["bin", "registry", "git"] {
        fs::create_dir_all(cargo.join(dir)).unwrap();
    }
    for file in ["config.toml", "config", "env"] {
        fs::write(cargo.join(file), "").unwrap();
    }
    let rustup = home.path().join(".rustup");

    let _env = EnvVarGuard::locked()
        .with("RUSTUP_HOME", &rustup)
        .with("CARGO_HOME", &cargo);
    let grants = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let rustup = canon(&rustup);
    let cargo = canon(&cargo);

    assert_eq!(
        grants
            .read_only
            .iter()
            .filter(|path| *path == &rustup)
            .count(),
        1
    );
    assert_eq!(
        grants
            .read_only
            .iter()
            .filter(|path| *path == &cargo.join("bin"))
            .count(),
        1
    );
    for file in ["config.toml", "config", "env"] {
        assert_eq!(
            grants
                .read_only
                .iter()
                .filter(|path| *path == &cargo.join(file))
                .count(),
            1,
            "{file} should appear once"
        );
    }
    assert_eq!(
        grants
            .read_write
            .iter()
            .filter(|path| *path == &cargo.join("registry"))
            .count(),
        1
    );
    assert_eq!(
        grants
            .read_write
            .iter()
            .filter(|path| *path == &cargo.join("git"))
            .count(),
        1
    );
}

#[test]
fn absent_toolchain_homes_are_not_granted() {
    let _env = isolated_toolchain_env();
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".nvm")).unwrap();
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let h = canon(home.path());
    assert!(has(&g.read_only, &h.join(".nvm")));
    assert!(!all(&g).iter().any(|p| p.starts_with(h.join(".cargo"))));
    assert!(!all(&g).iter().any(|p| p.starts_with(h.join(".rustup"))));
}

#[test]
fn toolchain_homes_can_be_switched_off() {
    let home = fake_home();
    fs::write(home.path().join(".gitconfig"), "[user]\n name = x\n").unwrap();
    let install = tempfile::tempdir().unwrap();
    let rustup = install.path().join("rustup-home");
    let cargo = install.path().join("cargo-home");
    fs::create_dir_all(&rustup).unwrap();
    for dir in ["bin", "registry", "git"] {
        fs::create_dir_all(cargo.join(dir)).unwrap();
    }
    let _env = EnvVarGuard::locked()
        .with("RUSTUP_HOME", &rustup)
        .with("CARGO_HOME", &cargo);
    let cfg = LocalJailConfig {
        toolchain_homes: false,
        ..LocalJailConfig::default()
    };
    let g = resolve_local_jail_grants(Some(home.path()), &cfg);
    assert!(g.read_only.is_empty() && g.read_write.is_empty(), "{g:?}");
}

#[test]
fn cargo_home_with_registry_credentials_is_granted_piecewise() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    let cargo = home.path().join(".cargo");
    for d in ["bin", "registry", "git"] {
        fs::create_dir_all(cargo.join(d)).unwrap();
    }
    fs::write(
        cargo.join("credentials.toml"),
        "[registry]\ntoken = \"x\"\n",
    )
    .unwrap();
    fs::write(cargo.join("config.toml"), "").unwrap();
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let c = canon(&cargo);
    assert!(
        !has(&g.read_write, &c),
        "whole ~/.cargo would expose credentials.toml"
    );
    assert!(!all(&g).iter().any(|p| p.ends_with("credentials.toml")));
    assert!(has(&g.read_write, &c.join("registry")));
    assert!(has(&g.read_write, &c.join("git")));
    assert!(has(&g.read_only, &c.join("bin")));
    assert!(has(&g.read_only, &c.join("config.toml")));
}

#[test]
fn a_writable_grant_subsumes_the_same_path_read_only() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    let cfg = LocalJailConfig {
        extra_read_only: vec!["~/.npm".into()],
        extra_read_write: vec!["~/.npm".into()],
        ..LocalJailConfig::default()
    };
    let g = resolve_local_jail_grants(Some(home.path()), &cfg);
    let npm = canon(&home.path().join(".npm"));
    assert!(has(&g.read_write, &npm));
    assert!(!has(&g.read_only, &npm));
}

#[test]
fn proc_is_off_by_default_and_only_the_toggle_grants_it() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    assert!(!all(&g).iter().any(|p| p.starts_with("/proc")));

    // An `extra_*` entry cannot smuggle it in: only `allow_proc` does.
    let cfg = LocalJailConfig {
        extra_read_only: vec!["/proc".into()],
        ..LocalJailConfig::default()
    };
    let g = resolve_local_jail_grants(Some(home.path()), &cfg);
    assert!(!all(&g).iter().any(|p| p.starts_with("/proc")));

    let cfg = LocalJailConfig {
        allow_proc: true,
        ..LocalJailConfig::default()
    };
    let g = resolve_local_jail_grants(Some(home.path()), &cfg);
    assert!(has(&g.read_only, Path::new("/proc")));
    assert!(!has(&g.read_write, Path::new("/proc")));
}

#[test]
fn extras_expand_tilde_and_skip_missing_paths() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    fs::create_dir_all(home.path().join("dotfiles")).unwrap();
    fs::create_dir_all(home.path().join("data")).unwrap();
    let cfg = LocalJailConfig {
        extra_read_only: vec!["~/dotfiles".into(), "~/nope".into()],
        extra_read_write: vec!["~/data/../data".into()],
        ..LocalJailConfig::default()
    };
    let g = resolve_local_jail_grants(Some(home.path()), &cfg);
    assert!(has(&g.read_only, &canon(&home.path().join("dotfiles"))));
    assert!(has(&g.read_write, &canon(&home.path().join("data"))));
    assert!(!all(&g).iter().any(|p| p.ends_with("nope")));
}

#[test]
fn gitconfig_symlink_and_include_targets_are_canonicalized() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    let dots = home.path().join("dotfiles");
    fs::create_dir_all(&dots).unwrap();
    fs::write(dots.join("work.gitconfig"), "[user]\n email = a@b\n").unwrap();
    fs::write(dots.join("local.gitconfig"), "").unwrap();
    fs::write(
        dots.join("gitconfig"),
        "[user]\n name = x\n[include]\n path = dotfiles/work.gitconfig\n\
         [includeIf \"gitdir:~/w/\"]\n\tpath = ~/dotfiles/local.gitconfig\n\
         [include]\n path = ~/.ssh/leaky\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(dots.join("gitconfig"), home.path().join(".gitconfig")).unwrap();
    // The credential include must be dropped even though the file exists.
    fs::write(home.path().join(".ssh/leaky"), "").unwrap();

    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let d = canon(&dots);
    assert!(has(&g.read_only, &d.join("gitconfig")), "{g:?}");
    assert!(has(&g.read_only, &d.join("work.gitconfig")), "{g:?}");
    assert!(has(&g.read_only, &d.join("local.gitconfig")), "{g:?}");
    assert_eq!(reaches_credentials(&all(&g), home.path()), None);
}

#[test]
fn gitconfig_include_cycles_terminate() {
    let _env = isolated_toolchain_env();
    let home = fake_home();
    fs::write(home.path().join(".gitconfig"), "[include]\n path = ~/b\n").unwrap();
    fs::write(home.path().join("b"), "[include]\n path = ~/.gitconfig\n").unwrap();
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    assert!(has(&g.read_only, &canon(&home.path().join("b"))));
}

#[test]
fn no_home_yields_only_system_toolchain_dirs() {
    let _env = isolated_toolchain_env();
    let g = resolve_local_jail_grants(None, &LocalJailConfig::default());
    assert!(g.read_write.is_empty());
    assert!(g
        .read_only
        .iter()
        .all(|p| p.starts_with("/usr/local") || p.starts_with("/opt")));
}
