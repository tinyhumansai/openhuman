use super::*;

fn bind(root: &str) -> Result<Memory> {
    Memory::bind(Config::default(), root)
}

#[test]
fn binds_a_tenant_root_and_pins_it_on_the_config() {
    let memory = bind(" team:acme ").expect("a team root binds");
    assert_eq!(memory.root(), "team:acme");
    assert_eq!(memory.config.memory.root.as_deref(), Some("team:acme"));
    assert_eq!(memory.config.memory.agent_id, None);
    assert_eq!(memory.agent_node("ceo").unwrap(), "team:acme/agent:ceo");
}

#[test]
fn refuses_the_store_root_and_invalid_roots() {
    assert!(matches!(bind(""), Err(Error::InvalidRequest(_))));
    assert!(matches!(
        bind("not a namespace"),
        Err(Error::InvalidRequest(_))
    ));
}

#[test]
fn one_agents_listing_reads_only_its_own_node() {
    let memory = bind("team:acme").unwrap();
    let reach = memory.reach_of(Some("ceo")).unwrap();
    assert_eq!(reach.at.to_string(), "team:acme/agent:ceo");
    assert!(!reach.inherit);
    assert!(reach.descendants);
    let whole = memory.reach_of(Some("  ")).unwrap();
    assert_eq!(whole, Reach::subtree("team:acme".parse().unwrap()));
}

#[test]
fn one_agents_recall_inherits_the_root_but_not_a_sibling() {
    let memory = bind("team:acme").unwrap();
    let reach = memory.reading(Some("ceo")).unwrap().reach.unwrap();
    assert!(reach.admits(&"team:acme".parse().unwrap()));
    assert!(reach.admits(&"team:acme/agent:ceo".parse().unwrap()));
    assert!(!reach.admits(&"team:acme/agent:cfo".parse().unwrap()));
    assert!(!reach.admits(&"team:other".parse().unwrap()));
}
