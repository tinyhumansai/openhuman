//! A workspace written by the previous release resumes in place: resuming a
//! committed fixture neither renames, rewrites nor re-adopts anything, so the
//! transcripts the store holds are exactly what the import brought in.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;
use tinyinference_llm::model::ModelResponse;

use super::transcript_compat_tests::{
    build_host, model, place_fixture, run_async, store_tree, thread_id, Layout, SCENARIOS,
};

fn tree(
    root: &Path,
    thread: &str,
    host: &crate::agent::OpenHumanSessionHost,
) -> BTreeMap<String, Value> {
    store_tree(root, thread, host.session.as_ref().expect("bound session"))
}

#[test]
fn resuming_a_committed_workspace_leaves_every_stem_and_row_untouched() {
    for scenario in SCENARIOS
        .iter()
        .filter(|scenario| scenario.layout != Layout::Legacy)
    {
        run_async(async move {
            let root = tempfile::tempdir().expect("tempdir");
            let thread = thread_id(scenario.name);
            let next = model(vec![ModelResponse::assistant("unused")], scenario.native);
            let mut host = build_host(root.path(), next, scenario.native, &thread);
            let stem = host.session_id().expect("bound session id");
            place_fixture(root.path(), scenario, &stem, "");
            let before = tree(root.path(), &thread, &host);
            assert!(!before.is_empty(), "{}: fixture placed", scenario.name);

            assert!(host.resume_bound_session().await.expect("resume"));

            assert_eq!(
                before,
                tree(root.path(), &thread, &host),
                "{}: resume must not rename, create or rewrite transcripts",
                scenario.name
            );
        });
    }
}

#[test]
fn a_legacy_workspace_is_adopted_once_and_the_next_process_resumes_in_place() {
    let scenario = SCENARIOS
        .iter()
        .find(|scenario| scenario.layout == Layout::Legacy)
        .expect("legacy scenario");
    run_async(async move {
        let root = tempfile::tempdir().expect("tempdir");
        let thread = thread_id(scenario.name);
        let legacy_stems = ["1700000001_orchestrator", "1700000002_orchestrator"];
        let legacy_files: Vec<_> = legacy_stems
            .iter()
            .map(|stem| super::transcript_compat_tests::classic_path(root.path(), stem))
            .collect();
        let legacy_bytes;
        let after_first;
        let canonical_stem;
        {
            let next = model(vec![ModelResponse::assistant("unused")], scenario.native);
            let mut host = build_host(root.path(), next, scenario.native, &thread);
            let stem = host.session_id().expect("bound session id");
            canonical_stem = stem.clone();
            let import = place_fixture(root.path(), scenario, &stem, "");
            assert_eq!(import, (2, 2, 0), "legacy fixture import counts");
            legacy_bytes = legacy_files
                .iter()
                .map(|path| std::fs::read(path).expect("read placed legacy transcript"))
                .collect::<Vec<_>>();
            assert!(host.resume_bound_session().await.expect("first resume"));
            after_first = tree(root.path(), &thread, &host);
        }
        // Adoption writes the canonical session stem without changing the old
        // source files that the one-time importer read.
        assert_eq!(
            legacy_files
                .iter()
                .map(|path| std::fs::read(path).expect("read legacy transcript after resume"))
                .collect::<Vec<_>>(),
            legacy_bytes,
            "legacy source files must stay byte-for-byte untouched"
        );
        assert!(
            after_first
                .keys()
                .any(|stem| !legacy_stems.contains(&stem.as_str())),
            "adopted transcript written"
        );
        let adopted = after_first
            .get(&canonical_stem)
            .expect("legacy roots are adopted into the session's canonical stem");
        assert_eq!(
            adopted["messages"].as_array().unwrap().len(),
            5,
            "both timestamped roots are preserved in the adopted conversation"
        );
        let adopted_json = serde_json::to_string(adopted).unwrap();
        for message in [
            "legacy system prompt",
            "legacy question 1",
            "legacy answer 1",
            "legacy question 2",
            "legacy answer 2",
        ] {
            assert!(
                adopted_json.contains(message),
                "adopted transcript lost {message}"
            );
        }

        // A fresh process over the same workspace resumes without adopting again.
        let next = model(vec![ModelResponse::assistant("unused")], scenario.native);
        let mut host = build_host(root.path(), next, scenario.native, &thread);
        assert!(host.resume_bound_session().await.expect("second resume"));
        assert_eq!(
            legacy_files
                .iter()
                .map(|path| std::fs::read(path).expect("read legacy transcript after restart"))
                .collect::<Vec<_>>(),
            legacy_bytes,
            "a second process must not modify the legacy source files"
        );
        assert_eq!(
            after_first,
            tree(root.path(), &thread, &host),
            "second resume must not re-adopt or rewrite anything"
        );
    });
}
