use super::*;
use std::path::PathBuf;

#[test]
fn skips_only_files_named_after_a_disabled_library() {
    let skip = vec!["libtinysearch".to_string(), "tinysearch".to_string()];
    assert!(is_skipped(&PathBuf::from("/x/libtinysearch.so"), &skip));
    assert!(is_skipped(&PathBuf::from("/x/tinysearch.dll"), &skip));
    assert!(!is_skipped(&PathBuf::from("/x/libtinydocs.so"), &skip));
}
