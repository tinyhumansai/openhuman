use super::*;

#[test]
fn project_skills_skipped_when_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    // No trust marker.
    let skill_dir = ws.join(".openhuman").join("skills").join("unsafe");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: unsafe\ndescription: should not load\n---\n",
    );
    let skills = load_skills_ws(ws);
    assert!(skills.is_empty(), "got {skills:?}");
}

#[test]
fn project_scope_shadows_user_scope_on_collision() {
    let user_dir = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();
    write(&ws_dir.path().join(".openhuman").join("trust"), "");

    let user_skill = user_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("greet");
    write(
        &user_skill.join("SKILL.md"),
        "---\nname: greet\ndescription: USER COPY\n---\n",
    );

    let proj_skill = ws_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("greet");
    write(
        &proj_skill.join("SKILL.md"),
        "---\nname: greet\ndescription: PROJECT COPY\n---\n",
    );

    let skills = discover_workflows(Some(user_dir.path()), Some(ws_dir.path()), true);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].description, "PROJECT COPY");
    assert!(skills[0].warnings.iter().any(|w| w.contains("shadowed")));
}

#[test]
fn load_skills_surfaces_user_scope() {
    // load_workflow_metadata now delegates to discover_workflows with dirs::home_dir(),
    // so user-scope skills reach production callers that still hit the
    // backwards-compat shim. Simulate this with an explicit tempdir home
    // via discover_workflows — we can't safely override the process HOME in
    // unit tests.
    let user_dir = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();

    let user_skill = user_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("user-only");
    write(
        &user_skill.join("SKILL.md"),
        "---\nname: user-only\ndescription: from user home\n---\n",
    );

    let skills = discover_workflows(
        Some(user_dir.path()),
        Some(ws_dir.path()),
        is_workspace_trusted(ws_dir.path()),
    );
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "user-only");
    assert_eq!(skills[0].scope, WorkflowScope::User);
}

#[test]
fn read_skill_resource_happy_path() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    write(
        &skill_dir.join("scripts").join("hello.sh"),
        "#!/bin/sh\necho hi\n",
    );

    let got = read_workflow_resource(ws, "demo", Path::new("scripts/hello.sh"))
        .expect("read should succeed");
    assert_eq!(got, "#!/bin/sh\necho hi\n");
}

#[test]
fn read_skill_resource_uses_directory_id_when_display_name_differs() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = ws.join("skills").join("demo-slug");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: Demo Display\ndescription: test skill\n---\n",
    );
    write(&skill_dir.join("references").join("note.md"), "slug read");

    let got = read_workflow_resource(ws, "demo-slug", Path::new("references/note.md"))
        .expect("directory id should resolve");
    assert_eq!(got, "slug read");
}

#[test]
fn read_skill_resource_rejects_directory_name_collision() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();

    let named_demo = ws.join("skills").join("alpha");
    write(
        &named_demo.join("SKILL.md"),
        "---\nname: demo\ndescription: display-name collision\n---\n",
    );
    write(&named_demo.join("references").join("note.md"), "alpha");

    let slug_demo = ws.join("skills").join("demo");
    write(
        &slug_demo.join("SKILL.md"),
        "---\nname: slug-demo\ndescription: slug collision\n---\n",
    );
    write(&slug_demo.join("references").join("note.md"), "demo");

    let err = read_workflow_resource(ws, "demo", Path::new("references/note.md"))
        .expect_err("ambiguous directory/name collision must be rejected");
    assert!(
        err.to_lowercase().contains("matches both"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_unknown_skill() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();

    let err = read_workflow_resource(ws, "does-not-exist", Path::new("scripts/x.sh"))
        .expect_err("unknown skill must be rejected");
    assert!(
        err.to_lowercase().contains("not found"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_empty_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    make_legacy_skill(ws, "demo");

    let err = read_workflow_resource(ws, "", Path::new("scripts/x.sh"))
        .expect_err("empty skill_id must be rejected");
    assert!(err.to_lowercase().contains("skill_id"), "unexpected: {err}");

    let err = read_workflow_resource(ws, "demo", Path::new(""))
        .expect_err("empty relative_path must be rejected");
    assert!(err.contains("non-empty relative path"), "unexpected: {err}");
}

// ── `discovery_home_dir`: the per-agent "no user roots" switch ───────────

use crate::skills::ops_discover::discovery_home_dir;

#[tokio::test]
async fn discovery_home_dir_is_hidden_by_a_context_without_user_skill_roots() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    // Outside any scope the ambient default applies: the operator's home.
    // (Whether a DEFAULT_CONTEXT was built by an earlier test in this process
    // does not matter — every booted or test context keeps user roots on.)
    assert_eq!(discovery_home_dir(), dirs::home_dir());

    let mut config = crate::config::Config::default();
    config.workspace_dir = std::path::PathBuf::from("/tmp/discovery-home-dir-test");
    let parent = CoreContext::for_test(DomainSet::full(), None);
    let hidden = parent.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), Default::default())
            .without_user_skill_roots(),
    );
    let visible = parent.derive_with(ContextOverlay::new(
        config,
        DomainSet::full(),
        Default::default(),
    ));

    let under_hidden = CoreContext::scope(hidden, async { discovery_home_dir() }).await;
    assert_eq!(under_hidden, None, "user-scope roots must be hidden");

    let under_visible = CoreContext::scope(visible, async { discovery_home_dir() }).await;
    assert_eq!(under_visible, dirs::home_dir());
}

#[tokio::test]
async fn load_workflow_metadata_skips_user_roots_under_a_hidden_context() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    // A user-scope skill under a fake home would normally be surfaced by
    // `load_workflow_metadata`; a derived context without user roots must not
    // see it, while workspace-scope skills are unaffected.
    let ws_dir = tempfile::tempdir().unwrap();
    let ws_skill = ws_dir.path().join("skills").join("ws-only");
    write(
        &ws_skill.join("SKILL.md"),
        "---\nname: ws-only\ndescription: workspace skill\n---\n",
    );

    let mut config = crate::config::Config::default();
    config.workspace_dir = ws_dir.path().to_path_buf();
    let hidden = CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(config, DomainSet::full(), Default::default())
            .without_user_skill_roots(),
    );

    let ws = ws_dir.path().to_path_buf();
    let names: Vec<String> = CoreContext::scope(hidden, async move {
        load_workflow_metadata(&ws)
            .into_iter()
            .map(|w| w.name)
            .collect()
    })
    .await;
    assert!(
        names.iter().any(|n| n == "ws-only"),
        "workspace skills survive: {names:?}"
    );
    assert!(
        names.iter().all(|n| n == "ws-only"),
        "no user-scope skill may leak into a hidden-root context: {names:?}"
    );
}

#[tokio::test]
async fn workflow_metadata_is_reused_until_invalidated() {
    // Scope an explicit skills-on context: the process default context other
    // tests install may have the component off.
    let ctx = crate::core::runtime::CoreContext::for_test_with_components(
        crate::core::runtime::ComponentSet::all(),
    );
    crate::core::runtime::CoreContext::scope(ctx, async {
        workflow_metadata_is_reused_body();
    })
    .await;
}

fn workflow_metadata_is_reused_body() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = workspace.path().join("skills").join("cached-skill");
    // The metadata cache is process-global and other tests invalidate it
    // concurrently, so retry the prime-then-reuse phase until one attempt
    // runs without an interleaved invalidation.
    let mut reused = false;
    for _ in 0..50 {
        write(
            &skill.join("SKILL.md"),
            "---\nname: cached-skill\ndescription: first version\n---\n",
        );
        let first = load_workflow_metadata(workspace.path());
        assert!(first.iter().any(|item| item.name == "cached-skill"));
        std::fs::remove_dir_all(&skill).unwrap();
        if load_workflow_metadata(workspace.path())
            .iter()
            .any(|item| item.name == "cached-skill")
        {
            reused = true;
            break;
        }
    }
    assert!(reused, "a turn reuses the startup snapshot");

    crate::skills::ops_discover::invalidate_workflow_metadata_cache();
    assert!(
        !load_workflow_metadata(workspace.path())
            .iter()
            .any(|item| item.name == "cached-skill"),
        "a skill change refreshes the snapshot"
    );
}

#[tokio::test]
async fn skills_component_off_yields_no_catalogue_and_skips_bundled_install() {
    use crate::core::runtime::{ComponentSet, CoreContext};

    let ws_dir = tempfile::tempdir().unwrap();
    write(
        &ws_dir
            .path()
            .join("skills")
            .join("ws-only")
            .join("SKILL.md"),
        "---\nname: ws-only\ndescription: workspace skill\n---\n",
    );
    let mut off = ComponentSet::all();
    off.skills = false;
    let ctx = CoreContext::for_test_with_components(off);

    let ws = ws_dir.path().to_path_buf();
    let (names, installed) = CoreContext::scope(ctx, async move {
        crate::skills::install_bundled_skills(&ws);
        (
            load_workflow_metadata(&ws).len(),
            crate::skills::bundled::builtin_root(&ws).exists(),
        )
    })
    .await;
    assert_eq!(names, 0, "disabled skills expose no catalogue");
    assert!(!installed, "disabled skills materialize no bundled skills");
}

#[cfg(all(feature = "flows", feature = "skills"))]
#[tokio::test]
async fn flow_authoring_is_hidden_when_tinyflows_component_is_disabled() {
    use crate::core::runtime::{ComponentSet, ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let workspace = tempfile::tempdir().unwrap();
    let profile = |profile_id: &str, enabled: bool| {
        let mut components = ComponentSet::all();
        let mut domains = DomainSet::full();
        let mut config = crate::config::Config::default();
        config.workspace_dir = workspace.path().to_path_buf();
        if !enabled {
            components.flows = false;
            components.tinyflows = false;
            config.components.flows = false;
            config.components.tinyflows = false;
            domains.flows = false;
        }
        let context = CoreContext::for_test_with_components(components).derive_with(
            ContextOverlay::new(config.clone(), domains, ToolGroups::none())
                .profile(profile_id)
                .without_user_skill_roots(),
        );
        (context, config, domains)
    };

    let (enabled_ctx, enabled_config, enabled_domains) = profile("enabled", true);
    crate::modules::tinyflows::copilot_proxy(&enabled_config)
        .await
        .expect("linked TinyFlows fixture is admitted and serving");
    let visible = CoreContext::scope(enabled_ctx, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        let gate = crate::skills::ops_discover::DiscoveryGate::from_config(
            &dispatch,
            &enabled_config,
            Some(enabled_domains),
        );
        assert!(
            gate.flow_authoring_enabled(),
            "unexpected flow gate: {gate:?}"
        );
        crate::skills::bundled::flow_authoring::prewarm(
            &dispatch,
            &enabled_config,
            Some(enabled_domains),
        )
        .await;
        assert!(crate::modules::runtime::is_module_serving(
            crate::modules::tinyflows::MODULE_ID
        ));
        crate::skills::load_workflow_metadata_for(&dispatch, &enabled_config)
    })
    .await;
    let (disabled_ctx, disabled_config, _) = profile("disabled", false);
    let hidden = CoreContext::scope(disabled_ctx, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        crate::skills::load_workflow_metadata_for(&dispatch, &disabled_config)
    })
    .await;

    assert!(
        visible.iter().any(|skill| skill.id() == "flow-authoring"),
        "the enabled profile sees its module-backed flow-authoring skill"
    );
    assert!(
        !hidden.iter().any(|skill| skill.id() == "flow-authoring"),
        "a profile with flows, TinyFlows, and the Flows domain disabled must not inherit another profile's flow-authoring builtin"
    );
}

#[cfg(all(feature = "flows", feature = "skills"))]
#[tokio::test]
async fn module_builtin_gate_only_narrows_context_and_explicit_config() {
    use crate::core::runtime::{ComponentSet, ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let workspace = tempfile::tempdir().unwrap();
    let mut enabled_config = crate::config::Config::default();
    enabled_config.workspace_dir = workspace.path().to_path_buf();
    let all = ComponentSet::all();
    let base = CoreContext::for_test_with_components(all);
    let enabled_context = base.derive_with(
        ContextOverlay::new(
            enabled_config.clone(),
            DomainSet::full(),
            ToolGroups::none(),
        )
        .profile("shared")
        .without_user_skill_roots(),
    );

    let enabled = CoreContext::scope(enabled_context, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        crate::skills::bundled::flow_authoring::prewarm(&dispatch, &enabled_config, None).await;
        crate::skills::load_workflow_metadata_for(&dispatch, &enabled_config)
    })
    .await;
    assert!(enabled.iter().any(|skill| skill.id() == "flow-authoring"));

    let mut context_components = all;
    context_components.tinyflows = false;
    let context_component_context = CoreContext::for_test_with_components(context_components)
        .derive_with(
            ContextOverlay::new(
                enabled_config.clone(),
                DomainSet::full(),
                ToolGroups::none(),
            )
            .profile("shared")
            .without_user_skill_roots(),
        );
    let context_component_hidden = CoreContext::scope(context_component_context, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        crate::skills::load_workflow_metadata_for(&dispatch, &enabled_config)
    })
    .await;
    assert!(
        !context_component_hidden
            .iter()
            .any(|skill| skill.id() == "flow-authoring"),
        "an all-enabled Config cannot widen a context with TinyFlows disabled"
    );

    let mut narrowed_domains = DomainSet::full();
    narrowed_domains.flows = false;
    let context_domain_context = CoreContext::for_test_with_components(all).derive_with(
        ContextOverlay::new(enabled_config.clone(), narrowed_domains, ToolGroups::none())
            .profile("shared")
            .without_user_skill_roots(),
    );
    let context_domain_hidden = CoreContext::scope(context_domain_context, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        crate::skills::ops_discover::load_workflow_metadata_for_domains(
            &dispatch,
            &enabled_config,
            Some(DomainSet::full()),
        )
    })
    .await;
    assert!(
        !context_domain_hidden
            .iter()
            .any(|skill| skill.id() == "flow-authoring"),
        "an all-enabled Config cannot widen a context without the Flows domain"
    );

    let mut disabled_config = enabled_config.clone();
    disabled_config.components.tinyflows = false;
    let config_hidden = CoreContext::scope(
        CoreContext::for_test_with_components(all).derive_with(
            ContextOverlay::new(
                enabled_config.clone(),
                DomainSet::full(),
                ToolGroups::none(),
            )
            .profile("shared")
            .without_user_skill_roots(),
        ),
        async {
            let dispatch = crate::core::runtime::DispatchCtx::ambient();
            crate::skills::load_workflow_metadata_for(&dispatch, &disabled_config)
        },
    )
    .await;
    assert!(
        !config_hidden
            .iter()
            .any(|skill| skill.id() == "flow-authoring"),
        "a narrower supplied Config hides resources from an enabled parent context"
    );

    let mut modules_disabled = enabled_config.clone();
    modules_disabled.modules.enabled = false;
    let modules_hidden = CoreContext::scope(
        CoreContext::for_test_with_components(all).derive_with(
            ContextOverlay::new(
                enabled_config.clone(),
                DomainSet::full(),
                ToolGroups::none(),
            )
            .profile("shared")
            .without_user_skill_roots(),
        ),
        async {
            let dispatch = crate::core::runtime::DispatchCtx::ambient();
            crate::skills::load_workflow_metadata_for(&dispatch, &modules_disabled)
        },
    )
    .await;
    assert!(
        !modules_hidden
            .iter()
            .any(|skill| skill.id() == "flow-authoring"),
        "a profile with modules disabled cannot reuse another profile's warmed snapshot"
    );

    let mut skills_disabled = all;
    skills_disabled.skills = false;
    let skills_context_hidden = CoreContext::scope(
        CoreContext::for_test_with_components(skills_disabled).derive_with(
            ContextOverlay::new(
                enabled_config.clone(),
                DomainSet::full(),
                ToolGroups::none(),
            )
            .profile("shared")
            .without_user_skill_roots(),
        ),
        async {
            let dispatch = crate::core::runtime::DispatchCtx::ambient();
            crate::skills::load_workflow_metadata_for(&dispatch, &enabled_config)
        },
    )
    .await;
    assert!(
        !skills_context_hidden
            .iter()
            .any(|skill| skill.id() == "flow-authoring"),
        "an all-enabled Config cannot widen a context with Skills disabled"
    );

    // Separate tenants can share one workspace. Tampering with and clearing
    // one tenant's snapshot must not evict or authorize the other's bytes.
    let tenant_snapshot = |profile_id: &'static str| {
        let config = enabled_config.clone();
        let ctx = CoreContext::for_test_with_components(all).derive_with(
            ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::none())
                .profile(profile_id)
                .without_user_skill_roots(),
        );
        (ctx, config)
    };
    let (tenant_a_ctx, tenant_a_config) = tenant_snapshot("tenant-a");
    let tenant_a = CoreContext::scope(tenant_a_ctx.clone(), async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        crate::skills::bundled::flow_authoring::prewarm(&dispatch, &tenant_a_config, None).await;
        crate::skills::load_workflow_metadata_for(&dispatch, &tenant_a_config)
    })
    .await;
    let (tenant_b_ctx, tenant_b_config) = tenant_snapshot("tenant-b");
    let tenant_b = CoreContext::scope(tenant_b_ctx.clone(), async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        crate::skills::bundled::flow_authoring::prewarm(&dispatch, &tenant_b_config, None).await;
        crate::skills::load_workflow_metadata_for(&dispatch, &tenant_b_config)
    })
    .await;
    let flow_a = tenant_a
        .iter()
        .find(|skill| skill.id() == "flow-authoring")
        .expect("tenant A has its own warmed snapshot");
    let flow_b = tenant_b
        .iter()
        .find(|skill| skill.id() == "flow-authoring")
        .expect("tenant B has its own warmed snapshot");
    assert_eq!(flow_a.scope, tinyskills::SkillScope::Builtin);
    assert_eq!(flow_b.scope, tinyskills::SkillScope::Builtin);
    let flow_a_document = flow_a.location.as_deref().expect("tenant A document");
    assert!(
        flow_a_document.starts_with(workspace.path()),
        "tamper target must be inside the isolated temp workspace: {}",
        flow_a_document.display()
    );
    let flow_b_document = flow_b.location.as_deref().expect("tenant B document");
    assert!(flow_b_document.starts_with(workspace.path()));
    let flow_a_file = flow_a
        .location
        .as_deref()
        .expect("tenant A skill document")
        .parent()
        .expect("tenant A skill directory")
        .join(&flow_a.resources[0]);
    std::fs::write(&flow_a_file, "tampered tenant A snapshot").unwrap();
    let tenant_a_after_tamper = CoreContext::scope(tenant_a_ctx, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        crate::skills::load_workflow_metadata_for(&dispatch, &tenant_a_config)
    })
    .await;
    assert!(!tenant_a_after_tamper
        .iter()
        .any(|skill| skill.id() == "flow-authoring"));
    let tenant_b_after_tamper = CoreContext::scope(tenant_b_ctx, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        let latest = crate::skills::load_workflow_metadata_for(&dispatch, &tenant_b_config);
        let skill = latest
            .iter()
            .find(|skill| skill.id() == "flow-authoring")
            .expect("tenant B snapshot remains visible");
        crate::skills::read_workflow_resource_for(
            &dispatch,
            &tenant_b_config,
            "flow-authoring",
            &skill.resources[0],
        )
    })
    .await;
    assert!(
        tenant_b_after_tamper.is_ok(),
        "tenant A tampering must not evict tenant B"
    );
    let _ = flow_b;
}

#[test]
fn skills_component_enabled_without_context_is_on() {
    assert!(crate::skills::skills_component_enabled());
}
