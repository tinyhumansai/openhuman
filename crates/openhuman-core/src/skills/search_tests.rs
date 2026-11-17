//! Tests for `skill_search`.

use super::*;

fn skill(dir: &str, name: &str, description: &str, tags: &[&str]) -> Workflow {
    Workflow {
        dir_name: dir.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    }
}

fn corpus() -> Vec<Workflow> {
    vec![
        skill(
            "flow-authoring",
            "flow-authoring",
            "The tinyflows authoring reference: expression syntax, node configuration, dry runs.",
            &["flows", "workflows", "reference"],
        ),
        skill(
            "changelog",
            "Release changelog",
            "Turn a git commit range into a written changelog for a release.",
            &["git", "release"],
        ),
        skill(
            "ascii-art",
            "ASCII art",
            "Render text as ASCII art via pyfiglet.",
            &["fun"],
        ),
    ]
}

#[test]
fn the_projection_omits_the_body_and_caps_the_description() {
    // The whole point of projecting: `list_workflows` serialises frontmatter,
    // resources and location for every skill. If this ever returns the struct
    // the tool has stopped saving anything.
    let long = "x".repeat(1000);
    let workflow = skill("s", "S", &long, &["t"]);
    let value = serde_json::to_value(project(&workflow)).expect("json");
    let obj = value.as_object().expect("object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["description", "id", "name", "scope", "tags"]);
    assert!(
        obj["description"].as_str().expect("str").len() <= tinyskills::MAX_DESCRIPTION + 8,
        "description must be capped"
    );
}

#[tokio::test]
async fn an_empty_query_is_an_error_rather_than_every_skill() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };
    let tool = SkillSearchTool::new(Arc::new(config));
    let result = tool
        .execute(json!({ "query": "   " }))
        .await
        .expect("dispatch");
    assert!(result.is_error);
}

#[test]
fn a_miss_says_so_instead_of_returning_a_bare_empty_list() {
    // Against `render`, not `execute`. The tool discovers through
    // `dirs::home_dir()`, so an `execute`-level miss test ranks against the
    // developer's own installed skills — this exact test failed that way,
    // matching five real skills in a supposedly empty temp workspace.
    let value = render(&[], 14);
    assert_eq!(value["matched"], 0);
    assert_eq!(value["installed"], 14);
    assert!(
        value["hint"]
            .as_str()
            .expect("hint")
            .contains("skill_registry_search"),
        "a miss must say what to do next"
    );
}

#[test]
fn a_hit_reports_how_many_were_searched() {
    // `installed` is what lets the model tell "nothing matched" from "you have
    // three skills and none is close" without a second call.
    let all = corpus();
    let hits = tinyskills::rank(&all, "changelog", 5);
    let value = render(&hits, all.len());
    assert_eq!(value["matched"], 1);
    assert_eq!(value["installed"], 3);
    assert!(value["hint"].is_null(), "a hit carries no miss hint");
}

#[cfg(all(feature = "flows", feature = "skills"))]
#[tokio::test]
async fn module_owned_flow_skill_is_findable_after_scoped_prewarm() {
    // Use TinyFlows' linked test module to exercise the same bus-owned
    // ListResources adapter as the published native-module integration test.
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };
    let context =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None)
            .derive_with(
                crate::core::runtime::ContextOverlay::new(
                    config.clone(),
                    crate::core::runtime::DomainSet::full(),
                    crate::tools::toolpacks::ToolGroups::none(),
                )
                .profile("skill-search")
                .without_user_skill_roots(),
            );
    let result = crate::core::runtime::CoreContext::scope(context, async {
        let ctx = crate::core::runtime::DispatchCtx::ambient();
        crate::modules::tinyflows::copilot_proxy(&config)
            .await
            .expect("linked TinyFlows fixture is admitted and serving");
        crate::skills::bundled::flow_authoring::prewarm(&ctx, &config, None).await;
        let tool = SkillSearchTool::new(Arc::new(config));
        tool.execute(json!({ "query": "flow authoring" }))
            .await
            .expect("dispatch")
    })
    .await;
    let text = format!("{result:?}");
    assert!(
        text.contains("flow-authoring"),
        "the bundled skill must be discoverable through search: {text}"
    );
}
