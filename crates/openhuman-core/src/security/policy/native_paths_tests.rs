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
