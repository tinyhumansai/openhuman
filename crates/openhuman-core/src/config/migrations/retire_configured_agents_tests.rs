use super::*;

#[test]
fn removes_only_the_top_level_agents_table_idempotently() {
    let mut document: toml::Value = toml::from_str(
        "default_model = 'kept'\n[agents.worker]\nmodel = 'retired'\n[memory.agents.local]\nrecall = true",
    )
    .unwrap();

    rewrite_document(&mut document);
    let first = document.clone();
    rewrite_document(&mut document);

    assert_eq!(document, first);
    assert_eq!(document["default_model"].as_str(), Some("kept"));
    assert!(document.get("agents").is_none());
    assert!(document["memory"]["agents"]["local"]["recall"]
        .as_bool()
        .unwrap());
}
