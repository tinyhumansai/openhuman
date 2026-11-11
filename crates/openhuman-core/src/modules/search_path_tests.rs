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
