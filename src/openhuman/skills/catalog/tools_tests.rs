use super::*;

#[test]
fn install_tool_is_external_effect_so_it_routes_through_approval_gate() {
    let tool = SkillRegistryInstallTool::new(Arc::new(Config::default()));
    assert_eq!(tool.name(), "skill_registry_install");
    // #3993: installs must raise an inline approval card before writing.
    assert!(
        tool.external_effect(),
        "skill_registry_install must declare external_effect so the harness gates it"
    );
    assert!(matches!(tool.permission_level(), PermissionLevel::Write));
}

#[test]
fn read_only_skill_tools_are_not_gated() {
    // Browse/search/sources stay ungated — they only read the catalog.
    assert!(!SkillRegistryBrowseTool.external_effect());
    assert!(!SkillRegistrySearchTool.external_effect());
    assert!(!SkillRegistrySourcesTool.external_effect());
}
