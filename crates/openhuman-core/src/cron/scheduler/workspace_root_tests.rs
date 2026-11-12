use super::*;
use tempfile::TempDir;

/// A checkout that holds the core workspace, as a deployment that keeps its
/// brain inside the action dir does: `<root>/brain` is the workspace and
/// `<root>` the action dir.
fn checkout(tmp: &TempDir) -> Config {
    let root = tmp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("brain")).unwrap();
    std::fs::create_dir_all(root.join("work/job")).unwrap();
    std::fs::create_dir_all(root.join("openhuman")).unwrap();
    Config {
        workspace_dir: root.join("brain"),
        action_dir: root.clone(),
        config_path: root.join("openhuman/config.toml"),
        ..Config::default()
    }
}

fn rooted_at(root: Option<&str>) -> AgentDefinition {
    let mut definition =
        crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only()
            .get("orchestrator")
            .cloned()
            .unwrap();
    definition.id = "rooted".into();
    definition.extensions.scheduled_workspace = root.map(str::to_owned);
    definition
}

#[test]
fn an_agent_that_names_no_root_keeps_the_action_dir() {
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp);
    assert_eq!(scheduled_workspace(&config, &rooted_at(None)), Ok(None));
}

#[test]
fn a_blank_root_is_refused_rather_than_read_as_unset() {
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp);
    let refused = scheduled_workspace(&config, &rooted_at(Some("  "))).unwrap_err();
    assert!(refused.contains("blank"), "{refused}");
}

#[test]
fn a_relative_root_resolves_against_the_action_dir() {
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp);
    assert_eq!(
        scheduled_workspace(&config, &rooted_at(Some("work/job"))),
        Ok(Some(config.action_dir.join("work/job")))
    );
}

#[cfg(unix)]
#[test]
fn an_absolute_root_is_followed_through_symlinks() {
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp);
    let link = config.action_dir.join("current");
    std::os::unix::fs::symlink(config.action_dir.join("work/job"), &link).unwrap();
    assert_eq!(
        scheduled_workspace(&config, &rooted_at(Some(link.to_str().unwrap()))),
        Ok(Some(config.action_dir.join("work/job")))
    );
}

#[test]
fn a_root_that_is_missing_or_not_a_directory_is_refused() {
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp);
    let missing = scheduled_workspace(&config, &rooted_at(Some("work/gone"))).unwrap_err();
    assert!(
        missing.contains("rooted") && missing.contains("work/gone"),
        "{missing}"
    );
    std::fs::write(config.action_dir.join("work/file"), "x").unwrap();
    let file = scheduled_workspace(&config, &rooted_at(Some("work/file"))).unwrap_err();
    assert!(file.contains("not a directory"), "{file}");
}

#[test]
fn a_root_that_holds_or_sits_in_the_core_workspace_is_refused() {
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp);
    std::fs::create_dir_all(config.workspace_dir.join("inner")).unwrap();
    for root in [".", "brain", "brain/inner"] {
        let refused = scheduled_workspace(&config, &rooted_at(Some(root))).unwrap_err();
        assert!(refused.contains("core workspace"), "{root}: {refused}");
    }
}

#[test]
fn a_root_that_holds_the_core_credentials_is_refused() {
    let tmp = TempDir::new().unwrap();
    let mut config = checkout(&tmp);
    // The config file is one of the core's credential stores.
    config.config_path = config.action_dir.join("work/job/config.toml");
    let refused = scheduled_workspace(&config, &rooted_at(Some("work"))).unwrap_err();
    assert!(refused.contains("credential"), "{refused}");
}

#[cfg(unix)]
#[test]
fn a_root_reached_through_an_alias_of_a_store_not_yet_written_is_refused() {
    let tmp = TempDir::new().unwrap();
    let mut config = checkout(&tmp);
    let alias = config.action_dir.join("alias");
    std::os::unix::fs::symlink(config.action_dir.join("work/job"), &alias).unwrap();
    // No config file exists yet; it would be written inside work/job.
    config.config_path = alias.join("config.toml");
    let refused = scheduled_workspace(&config, &rooted_at(Some("work/job"))).unwrap_err();
    assert!(refused.contains("credential"), "{refused}");
}

#[test]
fn the_master_key_file_is_matched_as_the_key_loader_reads_it() {
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp);
    let key = config.action_dir.join("work/job/master.key");
    std::fs::write(&key, "k").unwrap();
    let padded = std::ffi::OsString::from(format!("  {}  ", key.display()));
    assert!(credential_stores_with(&config, Some(padded)).contains(&key));
    assert!(credential_stores_with(&config, Some("   ".into()))
        .iter()
        .all(|store| store != Path::new("")));
}

#[test]
fn a_rooted_run_is_built_from_a_config_whose_action_dir_is_the_root() {
    let tmp = TempDir::new().unwrap();
    let config = checkout(&tmp);
    let root = config.action_dir.join("work/job");
    let rooted = rooted_config(&config, &root);
    assert_eq!(rooted.action_dir, root);
    assert_eq!(rooted.workspace_dir, config.workspace_dir);
    assert!(rooted.autonomy.trusted_roots.iter().any(|trusted| {
        trusted.path == root.to_string_lossy()
            && trusted.access == crate::security::TrustedAccess::ReadWrite
    }));
    // Idempotent, and an operator's own entry for the root is left alone.
    assert_eq!(
        rooted_config(&rooted, &root).autonomy.trusted_roots.len(),
        rooted.autonomy.trusted_roots.len()
    );
}
