use super::*;
async fn project(root: &std::path::Path) -> Work {
    super::super::store::start(
        root,
        "project",
        "Build and validate a complete system".into(),
        vec!["Integration and quality benchmarks pass".into()],
        None,
    )
    .await
    .unwrap()
}
fn task(id: &str, parent: Option<&str>, deps: &[&str]) -> Task {
    Task {
        id: id.into(),
        parent: parent.map(str::to_string),
        title: id.into(),
        acceptance_criteria: vec![format!("{id}: measured acceptance test passes")],
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        state: TaskState::Pending,
        evidence: vec![],
        revision: 0,
    }
}
#[tokio::test]
async fn ten_levels_persist_and_an_eleventh_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut work = project(tmp.path()).await;
    for level in 0..10 {
        add(
            &mut work,
            task(
                &format!("n{level}"),
                (level > 0).then(|| format!("n{}", level - 1)).as_deref(),
                &[],
            ),
        )
        .unwrap();
    }
    assert!(add(&mut work, task("too-deep", Some("n9"), &[])).is_err());
    super::super::store::update(tmp.path(), "project", &work.id, |w| {
        w.plan = work.plan.clone();
        Ok(())
    })
    .await
    .unwrap();
    let restored = super::super::store::get(tmp.path(), "project")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.plan.len(), 10);
    assert_eq!(restored.plan[9].parent.as_deref(), Some("n8"));
    assert!(context(&restored).contains("n9"));
}
#[tokio::test]
async fn dependencies_and_implicit_parent_edges_cannot_form_a_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let mut work = project(tmp.path()).await;
    add(&mut work, task("world", None, &[])).unwrap();
    add(&mut work, task("renderer", None, &["world"])).unwrap();
    assert!(add(&mut work, task("leaf", Some("world"), &["renderer"])).is_err());
    assert_eq!(work.plan.len(), 2);
    assert!(add(&mut work, task("leaf", Some("missing"), &[])).is_err());
    assert!(add(&mut work, task("leaf", Some("world"), &["world"])).is_err());
    assert!(begin(&mut work, "renderer").is_err());
    assert!(request_review(&mut work, "world").is_err());
}
#[tokio::test]
async fn changing_a_leaf_invalidates_parent_and_dependent_integration_but_not_siblings() {
    let tmp = tempfile::tempdir().unwrap();
    let mut work = project(tmp.path()).await;
    for node in [
        task("world", None, &[]),
        task("foliage", Some("world"), &[]),
        task("physics", Some("world"), &[]),
        task("benchmark", None, &["world"]),
    ] {
        add(&mut work, node).unwrap();
    }
    for node in &mut work.plan {
        node.state = TaskState::Verified;
        node.evidence = vec!["old benchmark".into()];
    }
    invalidate(&mut work, "foliage").unwrap();
    for id in ["world", "foliage", "benchmark"] {
        assert_eq!(
            work.plan[index(&work, id).unwrap()].state,
            TaskState::Pending
        );
        assert!(work.plan[index(&work, id).unwrap()].evidence.is_empty());
    }
    assert_eq!(
        work.plan[index(&work, "physics").unwrap()].state,
        TaskState::Verified
    );
    assert!(!all_verified(&work));
    begin(&mut work, "world").unwrap();
    assert!(request_review(&mut work, "world").is_err());
}
#[tokio::test]
async fn host_reviews_each_component_then_requires_a_separate_project_review() {
    use super::super::{observation::Observed, service, store};
    let tmp = tempfile::tempdir().unwrap();
    let work = project(tmp.path()).await;
    let mut work = store::update(tmp.path(), "project", &work.id, |w| {
        add(w, task("component", None, &[]))?;
        begin(w, "component")?;
        request_review(w, "component")?;
        w.lease = Some("review-component".into());
        w.finish_requested = true;
        w.evidence = vec!["Actual component test and inspection".into()];
        Ok(())
    })
    .await
    .unwrap();
    // No successful tools: model claims alone do not verify the package.
    let failed = service::settle(
        tmp.path(),
        &work,
        "review-component",
        &Observed::default(),
        None,
    )
    .await
    .unwrap();
    assert_ne!(failed.plan[0].state, TaskState::Verified);
    work = store::update(tmp.path(), "project", &work.id, |w| {
        request_review(w, "component")?;
        w.lease = Some("review-again".into());
        w.finish_requested = true;
        w.evidence = vec!["Actual component test and inspection".into()];
        Ok(())
    })
    .await
    .unwrap();
    let observed = Observed {
        successful: 2,
        shell: 1,
        failed: 0,
        fingerprint: 1,
    };
    let reviewed = service::settle(tmp.path(), &work, "review-again", &observed, None)
        .await
        .unwrap();
    assert_eq!(reviewed.plan[0].state, TaskState::Verified);
    assert_eq!(reviewed.phase, Phase::Working);
    assert!(
        super::super::store::get(tmp.path(), "project")
            .await
            .unwrap()
            .unwrap()
            .review_task
            .is_none()
    );
    let final_review = store::update(tmp.path(), "project", &work.id, |w| {
        w.phase = Phase::Reviewing;
        w.lease = Some("integration".into());
        w.finish_requested = true;
        w.evidence = vec!["Actual integration benchmark".into()];
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        service::settle(tmp.path(), &final_review, "integration", &observed, None)
            .await
            .unwrap()
            .phase,
        Phase::Complete
    );
}

#[tokio::test]
async fn large_plan_reports_are_paged_and_retry_notices_do_not_require_a_reply() {
    let tmp = tempfile::tempdir().unwrap();
    let mut work = project(tmp.path()).await;
    for n in 0..40 {
        add(&mut work, task(&format!("n{n}"), None, &[])).unwrap();
    }
    let page = report(&work, &serde_json::json!({"offset":16,"task_id":"n39"}));
    assert_eq!(page["plan"].as_array().unwrap().len(), 16);
    assert_eq!(page["plan_next_offset"], 32);
    assert_eq!(page["selected_task"]["id"], "n39");
    work.next_run_ms = 120000;
    assert!(work.continuation_notice(0).contains("120 seconds"));
    assert!(work.continuation_notice(0).contains("No reply is required"));
    work.phase = Phase::Blocked;
    assert!(!work.continuation_notice(0).contains("automatically"));
}

#[tokio::test]
async fn parent_contract_change_reopens_children_but_adding_a_child_preserves_verified_siblings() {
    let tmp = tempfile::tempdir().unwrap();
    let mut work = project(tmp.path()).await;
    add(&mut work, task("art-direction", None, &[])).unwrap();
    add(&mut work, task("sword", Some("art-direction"), &[])).unwrap();
    for node in &mut work.plan {
        node.state = TaskState::Verified;
    }
    add(&mut work, task("foliage", Some("art-direction"), &[])).unwrap();
    assert_eq!(work.plan[1].state, TaskState::Verified);
    invalidate(&mut work, "art-direction").unwrap();
    assert!(work.plan.iter().all(|t| t.state == TaskState::Pending));
}
