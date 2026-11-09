use super::*;
use crate::profiles::ProfileIdMode;

fn id() -> ProfileId {
    ProfileId::for_user("layout-user", ProfileIdMode::Raw).unwrap()
}

#[test]
fn every_path_sits_under_the_profile_directory() {
    let layout = profile_layout(Path::new("/srv/oh"), &id());
    assert_eq!(layout.dir, Path::new("/srv/oh/users").join(id().as_str()));
    assert_eq!(layout.meta_path, layout.dir.join("profile.toml"));
    for path in [
        &layout.meta_path,
        &layout.config_path,
        &layout.workspace_dir,
        &layout.sandbox_dir,
    ] {
        assert!(path.starts_with(&layout.dir), "{}", path.display());
    }
}

#[test]
fn the_saas_layout_is_the_desktop_user_layout() {
    let root = Path::new("/srv/oh");
    let layout = profile_layout(root, &id());
    assert_eq!(
        layout.dir,
        crate::config::schema::user_openhuman_dir(root, id().as_str())
    );
    assert_eq!(layout.dir.parent(), Some(users_dir(root).as_path()));
}

#[test]
fn the_config_is_rooted_in_the_profile_and_bound_to_its_memory() {
    let layout = profile_layout(Path::new("/srv/oh"), &id());
    let config = profile_config(&layout, &id());
    assert_eq!(config.config_path, layout.config_path);
    assert_eq!(config.workspace_dir, layout.workspace_dir);
    assert_eq!(config.action_dir, layout.sandbox_dir);
    assert_eq!(config.memory.agent_id.as_deref(), Some(id().as_str()));
    assert_eq!(config.memory.root, Some(memory_root(&id())));
}

#[test]
fn the_memory_root_is_a_valid_layout_root() {
    tinymemory_tools::layout::identity::validate_root(&memory_root(&id())).unwrap();
}

/// Under `users/<id>`, `memory::scope::user_root` reads an account id out of
/// the config path. A raw profile id that looks like a TinyHumans account
/// (24 hex) would give `org:<id>`; layout v3 would bind the engine there and
/// ignore `user:<id>`. The pin keeps a profile on the root it is forced to.
#[test]
fn a_profile_binds_its_user_root_never_an_org_root() {
    use crate::memory::scope;
    let id = ProfileId::for_user("65f1c0ffee0123456789abcd", ProfileIdMode::Raw).unwrap();
    assert_eq!(id.as_str(), "65f1c0ffee0123456789abcd");
    let layout = profile_layout(Path::new("/srv/oh"), &id);
    let config = profile_config(&layout, &id);

    // The hazard the pin closes.
    assert_eq!(
        scope::user_root(&config).as_deref(),
        Some("org:65f1c0ffee0123456789abcd")
    );
    // The pin: legacy layout, so `memory::engine::resolve` binds no org root
    // and every operation is confined to `user:<id>`.
    assert_eq!(config.memory.layout, MemoryLayoutMode::Legacy);
    assert_ne!(config.memory.layout, MemoryLayoutMode::V3);
    let bound = tinymemory_tools::layout::identity::Identity::root()
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(bound.root().to_string(), "user:65f1c0ffee0123456789abcd");
    assert_eq!(bound.agent_id, id.as_str());
    assert_eq!(bound.root().to_string(), "user:65f1c0ffee0123456789abcd");
}

#[test]
fn the_memory_binding_wins_over_pins_and_teams() {
    let layout = profile_layout(Path::new("/srv/oh"), &id());
    let mut config = profile_config(&layout, &id());
    config.memory.agents.insert(
        "planner".into(),
        toml::from_str("agent_id = \"other\"\nroot = \"team:shared\"").unwrap(),
    );
    let resolved = tinymemory_tools::layout::identity::Identity::team_member("shared", "planner")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(resolved.agent_id, id().as_str());
    assert_eq!(resolved.root().to_string(), memory_root(&id()));
}

#[test]
fn the_policy_is_on_and_closed() {
    let layout = profile_layout(Path::new("/srv/oh"), &id());
    let autonomy = profile_config(&layout, &id()).autonomy;
    assert!(autonomy.enabled);
    assert_eq!(autonomy.level, AutonomyLevel::Supervised);
    assert!(autonomy.workspace_only);
    assert!(!autonomy.auto_approve_all);
    assert!(!autonomy.allow_tool_install);
    assert!(autonomy.trusted_roots.is_empty());
}

#[test]
fn artifacts_land_in_the_profiles_sandbox() {
    let id = ProfileId::for_user("alice", ProfileIdMode::Raw).unwrap();
    let layout = profile_layout(Path::new("/srv/oh"), &id);
    let config = profile_config(&layout, &id);
    assert!(config.files_dir().starts_with(&layout.sandbox_dir));
}

#[test]
fn profiles_never_use_the_unrestricted_empty_channel_permission_map() {
    let layout = profile_layout(Path::new("/srv/oh"), &id());
    let config = profile_config(&layout, &id());
    assert!(!config.agent.channel_permissions.is_empty());
    assert!(config
        .agent
        .channel_permissions
        .values()
        .all(|v| v == "readonly"));
}
