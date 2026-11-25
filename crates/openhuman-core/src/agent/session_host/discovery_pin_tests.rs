//! A thread's `tool_search` options are chosen when it is created, recorded on
//! its first transcript row, and read back on resume: a config that changes
//! later reaches new threads only.

use std::path::Path;

use tinyinference_llm::model::ModelResponse;

use super::transcript_compat_tests::{
    build_host_with_config, model, run_async, store_tree, thread_id,
};
use crate::agent::tinyagents::discovery::DISCOVERY_METADATA_KEY;
use crate::config::schema::ToolSearchConfig;
use crate::config::AgentConfig;

fn agent_config(tool_search: ToolSearchConfig) -> AgentConfig {
    AgentConfig {
        tool_search,
        ..AgentConfig::default()
    }
}

fn no_promotion() -> ToolSearchConfig {
    ToolSearchConfig {
        promote_found_tools: false,
        collapse_search_results: true,
        ..ToolSearchConfig::default()
    }
}

/// Runs one turn of `thread` on a fresh host over `root`, as a new process
/// would, and returns how many stored rows carry the recorded options.
async fn turn_and_count_records(root: &Path, thread: &str, tool_search: ToolSearchConfig) -> usize {
    let next = model(vec![ModelResponse::assistant("ok")], true);
    let mut host = build_host_with_config(root, next, true, thread, agent_config(tool_search));
    host.turn("hello").await.expect("turn");
    let session = host.session.clone().expect("bound session");
    store_tree(root, thread, &session)
        .values()
        .map(|stored| stored.to_string().matches(DISCOVERY_METADATA_KEY).count())
        .sum()
}

#[test]
fn a_new_thread_records_options_that_differ_from_today_once() {
    run_async(async {
        let root = tempfile::tempdir().expect("tempdir");
        let thread = thread_id("pinned-new");
        assert_eq!(
            turn_and_count_records(root.path(), &thread, no_promotion()).await,
            1,
            "the turn that creates the thread records its options"
        );
        // A later process with a different config neither adds nor changes it.
        assert_eq!(
            turn_and_count_records(root.path(), &thread, ToolSearchConfig::default()).await,
            1,
            "a resumed thread keeps the record it started with"
        );
    });
}

#[test]
fn a_thread_created_with_todays_behaviour_records_nothing_and_stays_that_way() {
    run_async(async {
        let root = tempfile::tempdir().expect("tempdir");
        let thread = thread_id("legacy-new");
        assert_eq!(
            turn_and_count_records(root.path(), &thread, ToolSearchConfig::default()).await,
            0
        );
        // The config changes afterwards; the thread is not pinned to it.
        assert_eq!(
            turn_and_count_records(root.path(), &thread, no_promotion()).await,
            0
        );
    });
}

/// A thread whose durable history was imported has rows but no frozen prefix
/// and no recorded options: it ran before they existed, so it keeps today's
/// behaviour whatever the config says now, like any other unrecorded thread.
#[test]
fn a_thread_with_imported_history_and_no_record_keeps_todays_behaviour() {
    use tinyagents_session::transcript::TranscriptMessage;

    run_async(async {
        // No TinySecurity module runs in a unit test, so the screen would hold the input.
        let _off = crate::security::switch::off_for_this_thread();
        let root = tempfile::tempdir().expect("tempdir");
        let thread = thread_id("imported-unpinned");
        let next = model(vec![ModelResponse::assistant("ok")], true);
        let mut host = build_host_with_config(
            root.path(),
            next,
            true,
            &thread,
            agent_config(no_promotion()),
        );
        let imported = [
            TranscriptMessage::user("legacy question"),
            TranscriptMessage::assistant("legacy answer"),
        ];
        assert!(host.adopt_imported_history(&imported).expect("adopt"));
        host.turn("hello").await.expect("turn");
        let session = host.session.clone().expect("bound session");
        let records: usize = store_tree(root.path(), &thread, &session)
            .values()
            .map(|stored| stored.to_string().matches(DISCOVERY_METADATA_KEY).count())
            .sum();
        assert_eq!(
            records, 0,
            "the config of today must not be adopted by a thread that predates the pin"
        );
    });
}
