use super::*;

#[test]
fn generation_family_registration_preserves_tool_names_and_order() {
    let config = crate::config::Config::default();
    let tools = registered_tools(&config, Arc::new(SecurityPolicy::default()));
    let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

    assert_eq!(names, ["generate_presentation", "generate_document"]);
}
