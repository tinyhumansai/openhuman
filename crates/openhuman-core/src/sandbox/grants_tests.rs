use super::*;
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
    let home = fake_home();
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    assert_eq!(reaches_credentials(&all(&g), home.path()), None);
}

#[test]
fn credential_dirs_are_never_granted_even_when_extras_ask() {
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
    let home = fake_home();
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    let h = canon(home.path());
    assert!(has(&g.read_write, &h.join(".cargo")));
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
fn absent_toolchain_homes_are_not_granted() {
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
    let cfg = LocalJailConfig {
        toolchain_homes: false,
        ..LocalJailConfig::default()
    };
    let g = resolve_local_jail_grants(Some(home.path()), &cfg);
    assert!(g.read_only.is_empty() && g.read_write.is_empty(), "{g:?}");
}

#[test]
fn cargo_home_with_registry_credentials_is_granted_piecewise() {
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
    let home = fake_home();
    fs::write(home.path().join(".gitconfig"), "[include]\n path = ~/b\n").unwrap();
    fs::write(home.path().join("b"), "[include]\n path = ~/.gitconfig\n").unwrap();
    let g = resolve_local_jail_grants(Some(home.path()), &LocalJailConfig::default());
    assert!(has(&g.read_only, &canon(&home.path().join("b"))));
}

#[test]
fn no_home_yields_only_system_toolchain_dirs() {
    let g = resolve_local_jail_grants(None, &LocalJailConfig::default());
    assert!(g.read_write.is_empty());
    assert!(g
        .read_only
        .iter()
        .all(|p| p.starts_with("/usr/local") || p.starts_with("/opt")));
}
