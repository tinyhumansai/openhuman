use super::types::{ACCOUNT_CONFIG_FILE, ACCOUNT_INTERNAL_FILES};
use super::*;

/// The reserved paths the host hands TinySecurity, as plain path buffers.
fn reserved(policy: &SecurityPolicy) -> Vec<PathBuf> {
    policy
        .native_path_policy()
        .reserved_paths
        .into_iter()
        .map(|reservation| reservation.path)
        .collect()
}

fn policy_with(workspace: &str, account: Option<&str>) -> SecurityPolicy {
    SecurityPolicy {
        enabled: true,
        workspace_dir: PathBuf::from(workspace),
        account_dir: account.map(PathBuf::from),
        ..SecurityPolicy::default()
    }
}

#[test]
fn the_account_root_reserves_every_internal_file_not_just_the_config() {
    // The split layout: internal state was moved, the credential root stayed.
    let policy = policy_with("/proj/workspace", Some("/proj/.openhuman"));
    let reserved = reserved(&policy);

    for name in ACCOUNT_INTERNAL_FILES {
        let expected = PathBuf::from("/proj/.openhuman").join(name);
        assert!(
            reserved.contains(&expected),
            "{name} is not reserved under the account root: {reserved:?}"
        );
    }
    // `.secret_key` and the rest are secrets, so the carve-out must cover more
    // than the one name #5505 carved out.
    assert!(ACCOUNT_INTERNAL_FILES.len() > 1);
}

#[test]
fn the_workspace_parent_fallback_keeps_only_the_config_once_the_roots_diverge() {
    let policy = policy_with("/proj/workspace", Some("/proj/.openhuman"));
    let reserved = reserved(&policy);

    // The parent is not the account dir here, so reserving its other files
    // would guard the wrong directory.
    assert!(reserved.contains(&PathBuf::from("/proj").join(ACCOUNT_CONFIG_FILE)));
    for name in ACCOUNT_INTERNAL_FILES
        .iter()
        .filter(|name| **name != ACCOUNT_CONFIG_FILE)
    {
        assert!(
            !reserved.contains(&PathBuf::from("/proj").join(name)),
            "{name} must not be reserved under the workspace parent: {reserved:?}"
        );
    }
}

#[test]
fn without_an_account_root_only_the_legacy_config_carve_out_applies() {
    let policy = policy_with("/proj/workspace", None);
    let reserved = reserved(&policy);

    assert!(reserved.contains(&PathBuf::from("/proj").join(ACCOUNT_CONFIG_FILE)));
    assert!(!reserved.contains(&PathBuf::from("/proj").join(".secret_key")));
}

#[test]
fn a_coincident_account_root_reserves_the_config_once() {
    // The default layout: `<openhuman_dir>/` holds both config and workspace.
    let policy = policy_with("/home/u/.openhuman/workspace", Some("/home/u/.openhuman"));
    let config = PathBuf::from("/home/u/.openhuman").join(ACCOUNT_CONFIG_FILE);

    assert_eq!(
        reserved(&policy)
            .iter()
            .filter(|path| **path == config)
            .count(),
        1,
        "the account root and the workspace parent agree here, so the \
         reservation must not be pushed twice"
    );
}

fn config_with(workspace: &str, config_path: &str) -> crate::config::Config {
    crate::config::Config {
        workspace_dir: PathBuf::from(workspace),
        action_dir: PathBuf::from(workspace),
        config_path: PathBuf::from(config_path),
        ..crate::config::Config::default()
    }
}

#[test]
fn every_config_constructor_carries_the_config_paths_account_root() {
    // The split layout again: the account root is `config_path`'s parent,
    // not `workspace_dir`'s, so it can only come from the config.
    let config = config_with("/proj/workspace", "/proj/.openhuman/config.toml");
    let account = PathBuf::from("/proj/.openhuman");

    for (name, policy) in [
        ("for_config", SecurityPolicy::for_config(&config)),
        (
            "for_config_in",
            SecurityPolicy::for_config_in(&config, std::path::Path::new("/proj/turn")),
        ),
    ] {
        assert_eq!(
            policy.account_dir.as_deref(),
            Some(account.as_path()),
            "{name}"
        );
        let reserved = reserved(&policy);
        assert!(
            reserved.contains(&account.join(".secret_key")),
            "{name} does not reserve the account files: {reserved:?}"
        );
    }
}

/// A policy built from a whole `Config` must go through `for_config` /
/// `for_config_in`, or it silently loses the account root and guards only
/// `config.toml`. Only the sites that build from dirs of their own and chain
/// `with_account_dir` themselves may call `from_config` directly.
#[test]
fn production_code_builds_config_policies_through_for_config() {
    const ALLOWED: &[&str] = &[
        "openhuman-core/src/core/runtime/bootstrap.rs",
        "openhuman-core/src/security/live_policy.rs",
        "openhuman-core/src/security/policy/enforcement.rs",
    ];
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read source dir") {
            let path = entry.expect("source dir entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "tests") {
                    walk(&path, out);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && !path.to_string_lossy().ends_with("tests.rs")
            {
                out.push(path);
            }
        }
    }

    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir");
    let mut files = Vec::new();
    for krate in ["openhuman-core", "openhuman-embed", "openhuman-tinyhumans"] {
        walk(&crates.join(krate).join("src"), &mut files);
    }
    // The scan has to see the tree, or an empty walk would pass.
    assert!(files.len() > 100, "only {} source files found", files.len());

    let offenders: Vec<String> = files
        .iter()
        .filter_map(|path| {
            let rel = path
                .strip_prefix(crates)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            if ALLOWED.contains(&rel.as_str()) {
                return None;
            }
            let source = std::fs::read_to_string(path).ok()?;
            let hits: Vec<usize> = source
                .lines()
                .enumerate()
                .filter(|(_, line)| {
                    !line.trim_start().starts_with("//")
                        && line.contains("SecurityPolicy::from_config(")
                })
                .map(|(index, _)| index + 1)
                .collect();
            (!hits.is_empty()).then(|| format!("{rel}:{hits:?}"))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "use SecurityPolicy::for_config / for_config_in instead: {offenders:?}"
    );
}
