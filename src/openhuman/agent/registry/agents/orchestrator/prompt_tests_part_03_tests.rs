use super::*;

#[test]
fn build_routes_prompt_heavy_domains_to_specialists() {
    let body = build(&ctx_with(&[])).unwrap();
    assert!(body.contains("**Needs a specialist**"));
    assert!(body.contains("Capabilities not in your tool list"));
    assert!(!body.contains("## Presentation generation"));
    assert!(!body.contains("Before calling `generate_presentation`"));
    assert!(!body.contains("## Presentations with images"));
}
