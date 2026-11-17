use super::*;
use std::path::PathBuf;

#[test]
fn skips_only_files_named_after_a_disabled_library() {
    let skip = vec!["libtinysearch".to_string(), "tinysearch".to_string()];
    assert!(is_skipped(&PathBuf::from("/x/libtinysearch.so"), &skip));
    assert!(is_skipped(&PathBuf::from("/x/tinysearch.dll"), &skip));
    assert!(!is_skipped(&PathBuf::from("/x/libtinydocs.so"), &skip));
}

#[test]
fn recognizes_source_and_release_artifacts_for_configured_modules() {
    let stems = configured_module_lib_stems();
    for artifact in [
        "/modules/libtinymcp.so",
        "/modules/libtinymcp_module.so",
        "/modules/tinymcp_module.dll",
    ] {
        let path = PathBuf::from(artifact);
        assert!(is_skipped(&path, &stems));
        assert_eq!(module_id_for_artifact(&path), Some("tinymcp"));
    }
    for artifact in [
        "/modules/libtinyeventqueues.so",
        "/modules/libtinyeventqueues_module.so",
        "/modules/tinyeventqueues_module.dll",
    ] {
        let path = PathBuf::from(artifact);
        assert!(is_skipped(&path, &stems));
        assert_eq!(module_id_for_artifact(&path), Some("tinyeventqueues"));
    }
}

#[test]
fn defers_tinysecurity_artifacts_until_host_callback_admission() {
    for artifact in [
        "/modules/libtinysecurity_module.so",
        "/modules/libtinysecurity_module_module.so",
        "/modules/tinysecurity_module_module.dll",
    ] {
        assert!(
            is_skipped(&PathBuf::from(artifact), &configured_module_lib_stems()),
            "{artifact} must wait for TinySecurity's host callback binding"
        );
    }
    assert!(!should_defer_to_host_callback(&PathBuf::from(
        "/modules/libtinydocs.so"
    )));
}

/// A module that takes a host callback must not be admitted by the generic
/// search-path loader: it would be initialized with no binding and every later
/// callback-bound load would be refused as "already loaded without a callback
/// binding" (the `todo_list_*`, `thread_goal_*` and background-subagent
/// failures, which all reach `tinychannels`).
#[test]
fn defers_every_callback_bound_module_artifact_to_the_callback_load() {
    // The released library names (each module's `modules.toml` / archive).
    // Named here rather than derived from `CALLBACK_BOUND_MODULES`, so a
    // module dropped from that list fails this test.
    for lib in [
        "tinysecurity_module",
        "tinymcp",
        "tinyeventqueues",
        "tinychannels_module",
    ] {
        for artifact in [
            format!("/modules/lib{lib}.so"),
            format!("/modules/lib{lib}.dylib"),
            format!("/modules/{lib}.dll"),
        ] {
            let path = PathBuf::from(&artifact);
            assert!(
                should_defer_to_host_callback(&path),
                "{artifact} must wait for its host callback binding"
            );
            assert!(is_skipped(&path, &configured_module_lib_stems()));
        }
    }
}
