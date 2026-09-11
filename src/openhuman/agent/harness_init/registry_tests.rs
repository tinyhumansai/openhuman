use super::*;

#[test]
fn all_steps_have_stable_ids_and_are_non_required() {
    let steps = all_steps();
    let ids: Vec<_> = steps.iter().map(|s| s.id).collect();
    let mut expected = vec![
        "python_runtime",
        "spacy",
        "kompress",
        "runtime_python_server",
    ];
    // `node_runtime` is a registration-site gate: it is absent (not
    // dead-but-listed) when the managed Node runtime is compiled out. `cfg!`
    // (not `#[cfg]`) keeps `expected` mutable-and-used in both builds — same
    // idiom as `tools/ops_tests.rs`.
    if cfg!(feature = "runtime-node") {
        expected.push("node_runtime");
    }
    assert_eq!(ids, expected);
    assert!(steps.iter().all(|s| !s.required));
    assert!(steps.iter().all(|s| !s.label.is_empty()));
}

/// GH-5047: only genuine install/download steps may surface the blocking
/// overlay. `runtime_python_server` is routine service startup and must be
/// classified non-provisioning so a warm restart never re-shows setup.
#[test]
fn provisioning_classification_excludes_service_startup() {
    for step in all_steps() {
        let expected = step.id != "runtime_python_server";
        assert_eq!(
            step.provisioning, expected,
            "step {} provisioning flag mismatch",
            step.id
        );
    }
}

/// #5056: on a fresh install (`Config::default()`) `runtime_python.enabled`
/// is `true` but no Python backend (spaCy/Kompress) is on, so the
/// `python_runtime` step must report itself already `Done` and `run` must
/// be a no-op — proving the eager managed-CPython download is skipped
/// when nothing at boot needs it. This is a pure gating check
/// (`python_needed_eagerly` returns `false`): it never touches disk or
/// resolves a real interpreter, so it stays hermetic.
#[tokio::test]
async fn python_runtime_step_is_done_by_default_with_no_backend_enabled() {
    let config = Config::default();
    assert!(
        !python_needed_eagerly(&config),
        "default config should not need Python eagerly (no backend enabled)"
    );
    assert!(
        python_is_done(&config).await,
        "python_runtime step should be Done without provisioning when no backend is enabled"
    );
    assert!(
        python_run(&config).await.is_ok(),
        "python_runtime run should no-op when no backend is enabled"
    );
}

/// Inverse of the above: once a backend (spaCy) is enabled, the step must
/// no longer be trivially `Done` via the eager-skip branch — proving the
/// gate still allows provisioning when a backend genuinely needs Python.
/// We only assert the gating predicate here (not `is_done`/`run`), so the
/// test never attempts a real interpreter probe/download.
#[test]
fn python_needed_eagerly_true_when_spacy_backend_enabled() {
    let mut config = Config::default();
    config.runtime_python.enabled = true;
    config.memory_tree.spacy_enabled = true;
    assert!(
        python_needed_eagerly(&config),
        "python should be needed eagerly once a Python backend is enabled"
    );
}

#[tokio::test]
async fn disabled_runtimes_report_done_without_work() {
    let mut config = Config::default();
    config.runtime_python.enabled = false;
    config.node.enabled = false;
    for step in all_steps() {
        assert!(
            (step.is_done)(&config).await,
            "step {} should be done when its runtime is disabled",
            step.id
        );
        assert!(
            (step.run)(&config).await.is_ok(),
            "step {} run should no-op when disabled",
            step.id
        );
    }
}
