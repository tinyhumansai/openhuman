use super::*;

use crate::config::schema::MemoryAgentConfig;
use tinymemory_api::{MetaFilter, Namespace, Reach};
use tinymemory_tools::confine::{Confinement, Policy};
use tinymemory_tools::layout::identity::{
    account_root, actor_of_root, chat_node, validate_root, Identity, ResolvedIdentity,
    DEFAULT_AGENT_ID,
};

fn ns(value: &str) -> Namespace {
    value.parse().unwrap()
}

#[test]
fn config_identity_adapter_preserves_host_definition_recall_and_v3_pins() {
    let mut config = Config::default();
    config.memory.agent_id = Some("host-agent".into());
    config.memory.root = Some("team:host".into());
    config.memory.recall.enabled = false;
    config.memory.layout = MemoryLayoutMode::V3;
    config.memory.agents.insert(
        "planner".into(),
        MemoryAgentConfig {
            agent_id: Some("planner-memory".into()),
            root: Some("project:q4".into()),
            recall: Some(true),
        },
    );

    let pins = super::identity_pins(&config);
    assert_eq!(pins.agent_id.as_deref(), Some("host-agent"));
    assert_eq!(pins.root.as_deref(), Some("team:host"));
    assert!(!pins.recall);
    assert!(pins.v3);
    assert_eq!(pins.agents.len(), 1);
    assert_eq!(
        pins.agents["planner"].agent_id.as_deref(),
        Some("planner-memory")
    );
    assert_eq!(pins.agents["planner"].root.as_deref(), Some("project:q4"));
    assert_eq!(pins.agents["planner"].recall, Some(true));

    let resolved: ResolvedIdentity = Identity::team_member("other", "planner").resolve(&pins);
    assert_eq!(resolved.agent_id, "host-agent");
    assert_eq!(resolved.root(), &ns("team:host"));
    assert!(resolved.recall);
    assert_eq!(
        resolved.layout.pooled_conversations_node(),
        Some(&ns("team:host/ws:main"))
    );
}

#[test]
fn upstream_identity_and_confinement_keep_embedded_root_and_rpc_policy() {
    let mut config = Config::default();
    config.memory.root = Some("project:embed/team:acme".into());
    let resolved = Identity::agent("planner").resolve(&super::identity_pins(&config));
    assert_eq!(resolved.root(), &ns("project:embed/team:acme"));

    let allowed = Reach::subtree(resolved.root().clone());
    let rpc = Confinement {
        allowed: allowed.clone(),
        policy: Policy::Refuse,
    };
    let narrow = ns("project:embed/team:acme/agent:planner");
    assert_eq!(
        rpc.filter(Some(MetaFilter {
            reach: Some(Reach::exact(narrow.clone())),
            ..MetaFilter::default()
        }))
        .unwrap()
        .reach,
        Some(Reach::exact(narrow))
    );
    assert!(rpc
        .filter(Some(MetaFilter {
            reach: Some(Reach::subtree(ns("project:embed/team:other"))),
            ..MetaFilter::default()
        }))
        .is_err());

    let saas = Confinement {
        allowed,
        policy: Policy::Clamp,
    };
    let clamped = saas
        .filter(Some(MetaFilter {
            reach: Some(Reach::subtree(Namespace::ROOT)),
            ..MetaFilter::default()
        }))
        .unwrap();
    assert_eq!(clamped.reach, Some(Reach::subtree(resolved.root().clone())));
}

#[test]
fn account_v3_identity_pools_at_the_absolute_layout_workspace() {
    let mut config = Config::default();
    config.config_path = "/srv/openhuman/users/6512ab0f6512ab0f6512ab0f/config.toml".into();
    config.memory.layout = MemoryLayoutMode::V3;

    let resolved = Identity::root().resolve(&super::identity_pins(&config));
    assert_eq!(
        user_root(&config).as_deref(),
        Some("org:6512ab0f6512ab0f6512ab0f")
    );
    assert_eq!(resolved.root(), &Namespace::ROOT);
    assert_eq!(
        resolved.layout.pooled_conversations_node(),
        Some(&ns("ws:main"))
    );
}

#[test]
fn by_default_an_agent_is_its_definition_under_the_default_root() {
    let config = Config::default();
    let researcher = tinymemory_tools::layout::identity::Identity::agent("researcher")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(researcher.agent_id, "researcher");
    assert_eq!(researcher.root(), &Namespace::ROOT);
    assert!(researcher.recall);

    let nobody = tinymemory_tools::layout::identity::Identity::root()
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(nobody.agent_id, DEFAULT_AGENT_ID);
    assert_eq!(
        tinymemory_tools::layout::identity::Identity::agent("  ")
            .resolve(&crate::memory::scope::identity_pins(&config))
            .agent_id,
        DEFAULT_AGENT_ID
    );
}

#[test]
fn a_team_member_shares_its_teams_root() {
    let config = Config::default();
    let writer = tinymemory_tools::layout::identity::Identity::team_member("acme", "writer")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(writer.root(), &ns("team:acme"));
    assert_eq!(writer.agent_id, "writer");
    assert_eq!(
        writer.layout.conversations("writer").unwrap(),
        ns("team:acme/agent:writer")
    );
    let child =
        tinymemory_tools::layout::identity::Identity::team_member("acme", "lead").child("scout");
    assert_eq!(
        child
            .resolve(&crate::memory::scope::identity_pins(&config))
            .root(),
        &ns("team:acme")
    );
}

#[test]
fn a_definition_pin_beats_the_team_and_the_default() {
    let mut config = Config::default();
    config.memory.agents.insert(
        "analyst".into(),
        MemoryAgentConfig {
            agent_id: Some("q4-desk".into()),
            root: Some("project:q4".into()),
            recall: Some(false),
        },
    );
    config.memory.agents.insert(
        "broken".into(),
        MemoryAgentConfig {
            root: Some("not a namespace".into()),
            ..MemoryAgentConfig::default()
        },
    );
    let analyst = tinymemory_tools::layout::identity::Identity::team_member("acme", "analyst")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(analyst.agent_id, "q4-desk");
    assert_eq!(analyst.root(), &ns("project:q4"));
    assert!(!analyst.recall);
    assert_eq!(
        tinymemory_tools::layout::identity::Identity::agent("broken")
            .resolve(&crate::memory::scope::identity_pins(&config))
            .root(),
        &Namespace::ROOT,
        "an invalid root falls back to the default"
    );
}

#[test]
fn a_host_binding_beats_everything() {
    let mut config = Config::default();
    config.memory.agent_id = Some("employee-7".into());
    config.memory.root = Some("project:acme".into());
    config.memory.agents.insert(
        "researcher".into(),
        MemoryAgentConfig {
            agent_id: Some("ignored".into()),
            root: Some("project:ignored".into()),
            recall: None,
        },
    );
    for identity in [
        tinymemory_tools::layout::identity::Identity::agent("researcher"),
        tinymemory_tools::layout::identity::Identity::team_member("acme", "writer"),
        tinymemory_tools::layout::identity::Identity::root(),
    ] {
        let resolved = identity.resolve(&crate::memory::scope::identity_pins(&config));
        assert_eq!(resolved.agent_id, "employee-7");
        assert_eq!(resolved.root(), &ns("project:acme"));
    }
}

#[test]
fn a_child_stays_in_the_parents_team() {
    let lead = tinymemory_tools::layout::identity::Identity::team_member("acme", "lead");
    assert_eq!(
        lead.child("lead").child("scout"),
        tinymemory_tools::layout::identity::Identity::team_member("acme", "scout")
    );
    assert_eq!(
        tinymemory_tools::layout::identity::Identity::root().child("solo"),
        tinymemory_tools::layout::identity::Identity::agent("solo")
    );
}

#[test]
fn validate_root_accepts_nodes_and_refuses_junk() {
    assert!(validate_root("team:acme").is_ok());
    assert!(validate_root("project:q4/team:ops").is_ok());
    assert!(validate_root("not a namespace").is_err());
    assert!(validate_root("company:acme").is_err());
}

#[test]
fn an_account_root_is_the_lowercased_account_id() {
    assert_eq!(
        account_root("6512AB0F6512ab0f6512ab0f").as_deref(),
        Some("org:6512ab0f6512ab0f6512ab0f")
    );
    assert_eq!(account_root("local-megamind-macbook"), None);
    assert_eq!(account_root(""), None);
}

#[test]
fn a_local_session_root_is_a_recorded_install_id_not_the_hostname() {
    let tmp = tempfile::tempdir().unwrap();
    let user_dir = tmp.path().join("users").join("local-megamind-macbook");
    let mut config = Config::default();
    config.config_path = user_dir.join("config.toml");
    config.workspace_dir = user_dir.join("workspace");
    let root = user_root(&config).unwrap();
    assert!(root.starts_with("org:local-"), "{root}");
    assert!(
        !root.contains("user:") && !root.contains("megamind"),
        "{root}"
    );
    assert_ne!(
        actor_of_root(&root),
        crate::memory::local_root::legacy_root("local-megamind-macbook"),
        "a fresh install does not derive its root from the hostname"
    );
    assert_eq!(user_root(&config).as_deref(), Some(root.as_str()));
}

#[test]
fn an_install_with_a_recorded_legacy_root_keeps_resolving_to_it() {
    let tmp = tempfile::tempdir().unwrap();
    let user_dir = tmp.path().join("users").join("local-megamind-macbook");
    let mut config = Config::default();
    config.config_path = user_dir.join("config.toml");
    config.workspace_dir = user_dir.join("workspace");
    let legacy = crate::memory::local_root::legacy_root("local-megamind-macbook");
    let file = crate::memory::local_root::path(&config.workspace_dir);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, format!(r#"{{"root":"{legacy}","origin":"legacy"}}"#)).unwrap();
    let root = user_root(&config).unwrap();
    assert!(root.starts_with("org:local-"), "{root}");
    // The old root is the actor: the retired root that is still read.
    assert_eq!(actor_of_root(&root), legacy);
}

#[test]
fn the_user_root_is_read_from_where_the_config_lives() {
    let mut config = Config::default();
    config.config_path = "/h/.openhuman/users/6512ab0f6512ab0f6512ab0f/config.toml".into();
    assert_eq!(
        user_root(&config).as_deref(),
        Some("org:6512ab0f6512ab0f6512ab0f")
    );
    config.config_path = "/h/.openhuman/users/local/config.toml".into();
    assert_eq!(user_root(&config), None);
    config.config_path = "/somewhere/else/config.toml".into();
    assert_eq!(user_root(&config), None);
}

#[test]
fn layout_v3_pools_every_agents_chats_at_ws_main() {
    let mut config = Config::default();
    let legacy = tinymemory_tools::layout::identity::Identity::agent("researcher")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(
        legacy.layout.conversations("researcher").unwrap(),
        ns("agent:researcher")
    );
    assert_ne!(config.memory.layout, MemoryLayoutMode::V3);

    config.memory.layout = MemoryLayoutMode::V3;
    assert_eq!(config.memory.layout, MemoryLayoutMode::V3);
    let researcher = tinymemory_tools::layout::identity::Identity::agent("researcher")
        .resolve(&crate::memory::scope::identity_pins(&config));
    let coder = tinymemory_tools::layout::identity::Identity::agent("coder")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(
        researcher.layout.conversations("researcher").unwrap(),
        ns("ws:main")
    );
    assert_eq!(coder.layout.conversations("coder").unwrap(), ns("ws:main"));
    assert_eq!(chat_node(&researcher.layout), ns("ws:main"));
    let history = researcher.layout.conversations_filter(Some("researcher"));
    assert_eq!(history.agent_id.as_deref(), Some("researcher"));

    let team = tinymemory_tools::layout::identity::Identity::team_member("acme", "writer")
        .resolve(&crate::memory::scope::identity_pins(&config));
    assert_eq!(chat_node(&team.layout), ns("team:acme/ws:main"));
}

#[test]
fn a_roots_actor_is_its_user_spelling() {
    assert_eq!(
        actor_of_root("org:6512ab0f6512ab0f6512ab0f"),
        "user:6512ab0f6512ab0f6512ab0f"
    );
    assert_eq!(
        actor_of_root("org:local-0011223344556677"),
        "user:local-0011223344556677"
    );
    // A non-org root is its own actor.
    assert_eq!(actor_of_root("user:local-aa"), "user:local-aa");
}
