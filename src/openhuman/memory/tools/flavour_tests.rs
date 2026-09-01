use super::*;
use tempfile::TempDir;

// `lookup_flavour` reads through `MemoryTree::flavour_profile` now (#5560), so
// a test that expects a real "not built yet" answer needs a driver serving the
// Tree family — the null driver a test workspace otherwise resolves to would
// answer `Unsupported`, which this tool reports as a failure rather than as an
// absent profile. `install_tinycortex_for_test` binds the very driver the
// loaded module wraps, so these tests exercise the same lookup production runs.

fn test_config() -> (TempDir, Arc<Config>) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, Arc::new(cfg))
}

#[test]
fn name_and_schema() {
    let (_tmp, cfg) = test_config();
    let tool = MemoryFlavourTool::new(cfg);
    assert_eq!(tool.name(), "memory_flavour");
    assert_eq!(tool.parameters_schema()["required"], json!(["flavour"]));
    assert!(tool.parameters_schema()["properties"]["flavour"].is_object());
}

#[test]
fn permission_level_is_read_only() {
    let (_tmp, cfg) = test_config();
    let tool = MemoryFlavourTool::new(cfg);
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
}

#[test]
fn permission_level_with_args_is_always_read_only() {
    let (_tmp, cfg) = test_config();
    let tool = MemoryFlavourTool::new(cfg);
    assert_eq!(
        tool.permission_level_with_args(&json!({})),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        tool.permission_level_with_args(&json!({"flavour": "communication"})),
        PermissionLevel::ReadOnly
    );
}

#[tokio::test]
async fn missing_flavour_is_error() {
    let (_tmp, cfg) = test_config();
    let tool = MemoryFlavourTool::new(cfg);
    let result = tool.execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn empty_flavour_is_error() {
    let (_tmp, cfg) = test_config();
    let tool = MemoryFlavourTool::new(cfg);
    let result = tool.execute(json!({"flavour": "   "})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn unknown_flavour_is_error() {
    let (_tmp, cfg) = test_config();
    let tool = MemoryFlavourTool::new(cfg);
    let result = tool.execute(json!({"flavour": "astrology"})).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Unknown flavour"));
}

#[tokio::test]
async fn valid_flavour_with_no_tree_yet_returns_no_profile_message() {
    let (_tmp, cfg) = test_config();
    crate::openhuman::memory::test_support::install_tinycortex_for_test(&cfg);
    let tool = MemoryFlavourTool::new(cfg);
    let result = tool
        .execute(json!({"flavour": "coding_style"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("No profile built yet"));
}

#[tokio::test]
async fn aliases_are_accepted() {
    for alias in ["comms", "coding", "env", "rules", "dislikes"] {
        let (_tmp, cfg) = test_config();
        crate::openhuman::memory::test_support::install_tinycortex_for_test(&cfg);
        let tool = MemoryFlavourTool::new(cfg);
        let result = tool.execute(json!({"flavour": alias})).await;
        assert!(result.is_ok(), "alias `{alias}` should be accepted");
        let result = result.unwrap();
        assert!(!result.is_error, "alias `{alias}` should not error");
        assert!(result.output().contains("No profile built yet"));
    }
}

// ── PersonaFacet, brought home from the engine in #5560 ──────────────────────
//
// These pin the two mappings that are an on-disk / agent-facing contract
// rather than cosmetics. `tree_scope` is the key a flavoured tree is stored
// under, so a drifted string does not fail loudly — it silently stops finding
// a tree that is still there. Asserting the literals is the point: a test that
// re-derived them from the enum would drift with it.

#[test]
fn persona_facet_tree_scopes_match_the_persisted_keys() {
    let expected = [
        (PersonaFacet::Communication, "persona/communication"),
        (PersonaFacet::CodingStyle, "persona/coding_style"),
        (PersonaFacet::Stack, "persona/stack"),
        (PersonaFacet::Workflow, "persona/workflow"),
        (PersonaFacet::Environment, "persona/environment"),
        (PersonaFacet::Directives, "persona/directives"),
        (PersonaFacet::AntiPreferences, "persona/anti_preferences"),
    ];
    for (facet, scope) in expected {
        assert_eq!(facet.tree_scope(), scope, "scope drift for {facet:?}");
    }
}

#[test]
fn persona_facet_parse_loose_accepts_every_documented_alias() {
    let cases = [
        ("communication", PersonaFacet::Communication),
        ("comms", PersonaFacet::Communication),
        ("tone", PersonaFacet::Communication),
        ("coding_style", PersonaFacet::CodingStyle),
        ("code_style", PersonaFacet::CodingStyle),
        ("coding", PersonaFacet::CodingStyle),
        ("style", PersonaFacet::CodingStyle),
        ("stack", PersonaFacet::Stack),
        ("tech_stack", PersonaFacet::Stack),
        ("technology", PersonaFacet::Stack),
        ("workflow", PersonaFacet::Workflow),
        ("process", PersonaFacet::Workflow),
        ("environment", PersonaFacet::Environment),
        ("env", PersonaFacet::Environment),
        ("tooling", PersonaFacet::Environment),
        ("directives", PersonaFacet::Directives),
        ("rules", PersonaFacet::Directives),
        ("directive", PersonaFacet::Directives),
        ("anti_preferences", PersonaFacet::AntiPreferences),
        ("anti_preference", PersonaFacet::AntiPreferences),
        ("antipreferences", PersonaFacet::AntiPreferences),
        ("dislikes", PersonaFacet::AntiPreferences),
        ("pet_peeves", PersonaFacet::AntiPreferences),
    ];
    for (input, want) in cases {
        assert_eq!(
            PersonaFacet::parse_loose(input),
            Some(want),
            "alias {input}"
        );
    }
}

#[test]
fn persona_facet_parse_loose_normalises_case_spaces_and_dashes() {
    // The normalisation the engine applied: trim, lowercase, then fold both
    // spaces and dashes to underscores.
    assert_eq!(
        PersonaFacet::parse_loose("  Coding-Style "),
        Some(PersonaFacet::CodingStyle)
    );
    assert_eq!(
        PersonaFacet::parse_loose("PET PEEVES"),
        Some(PersonaFacet::AntiPreferences)
    );
    assert_eq!(PersonaFacet::parse_loose("nonsense"), None);
    assert_eq!(PersonaFacet::parse_loose(""), None);
}

#[test]
fn persona_facet_headings_are_present_and_distinct() {
    let facets = [
        PersonaFacet::Communication,
        PersonaFacet::CodingStyle,
        PersonaFacet::Stack,
        PersonaFacet::Workflow,
        PersonaFacet::Environment,
        PersonaFacet::Directives,
        PersonaFacet::AntiPreferences,
    ];
    let mut seen = std::collections::HashSet::new();
    for facet in facets {
        let heading = facet.heading();
        assert!(!heading.is_empty(), "empty heading for {facet:?}");
        assert!(seen.insert(heading), "duplicate heading {heading}");
    }
}
