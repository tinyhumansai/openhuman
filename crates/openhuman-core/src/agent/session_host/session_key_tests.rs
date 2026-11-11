use super::*;

use std::collections::HashSet;
use std::sync::Arc;

use crate::storage::sessions::splice_fixture_tests::{blank_meta, memory_locator};
use tinyagents_session::transcript::TranscriptLocator;

fn parts(key: &str) -> (&str, &str, &str) {
    let mut it = key.splitn(3, '_');
    (
        it.next().unwrap(),
        it.next().unwrap(),
        it.next().unwrap_or(""),
    )
}

#[test]
fn mint_is_unique_for_one_agent_within_a_second() {
    let keys: HashSet<String> = (0..1_000).map(|_| mint("orchestrator")).collect();
    assert_eq!(keys.len(), 1_000);
}

#[test]
fn mint_keeps_unix_seconds_first_and_a_token_without_underscores() {
    let key = mint("tools_agent");
    let (unix, token, agent) = parts(&key);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let unix: u64 = unix.parse().expect("first segment is unix seconds");
    assert!(unix <= now && now - unix < 60, "{key}");
    assert_eq!(token.len(), TOKEN_LEN, "{key}");
    assert!(token.bytes().all(|b| b.is_ascii_hexdigit()), "{key}");
    assert_eq!(agent, "tools_agent");
}

#[test]
fn mint_sanitizes_the_agent_name() {
    let key = mint("my agent/v2");
    assert!(key.ends_with("_my_agent_v2"), "{key}");
}

#[test]
fn rename_keeps_timestamp_and_token_across_names_with_underscores() {
    let key = mint("tools_agent");
    let renamed = rename(&key, "tools_agent", "workflow_builder");
    let (unix, token, _) = parts(&key);
    assert_eq!(renamed, format!("{unix}_{token}_workflow_builder"));
    let back = rename(&renamed, "workflow_builder", "orchestrator_thread-6ad6d");
    assert_eq!(back, format!("{unix}_{token}_orchestrator_thread-6ad6d"));
}

#[test]
fn rename_of_an_unexpected_key_mints_a_fresh_one() {
    for key in [
        "1713000000_orchestrator",
        "not-a-key",
        "",
        "x_abcdefabcdef_orchestrator",
    ] {
        let renamed = rename(key, "orchestrator", "planner");
        let (unix, token, agent) = parts(&renamed);
        assert!(unix.parse::<u64>().is_ok(), "{key} -> {renamed}");
        assert_eq!(token.len(), TOKEN_LEN, "{key} -> {renamed}");
        assert_eq!(agent, "planner", "{key} -> {renamed}");
    }
}

#[test]
fn rename_with_the_wrong_old_name_mints_a_fresh_one() {
    let key = mint("orchestrator");
    let renamed = rename(&key, "planner", "critic");
    assert!(renamed.ends_with("_critic"), "{renamed}");
    assert_ne!(parts(&renamed).1, parts(&key).1);
}

#[test]
fn overlapping_unthreaded_runs_of_one_agent_keep_separate_transcripts() {
    std::thread::Builder::new()
        .stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime")
                .block_on(overlapping_unthreaded_runs());
        })
        .expect("test thread")
        .join()
        .expect("test thread panicked");
}

fn scheduled_turn() -> crate::agent::session_host::TurnOverrides {
    crate::agent::session_host::TurnOverrides {
        suppress_transcript_autoload: true,
        ..Default::default()
    }
}

fn transcript_text(locator: &Arc<dyn TranscriptLocator>, stem: &str) -> String {
    locator
        .open_stem(stem, blank_meta())
        .expect("bind transcript")
        .messages()
        .expect("read transcript")
        .into_iter()
        .map(|row| row.content)
        .collect::<Vec<_>>()
        .join("\n")
}

async fn overlapping_unthreaded_runs() {
    let _ = crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global_builtins();
    crate::agent::session_host::test_support::install_model_fixture();
    let root = tempfile::tempdir().expect("tempdir");
    let locator = memory_locator();
    let new_host = |replies: Vec<String>| {
        let model = Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(replies).with_profile(
                tinyinference_llm::model::ModelProfile {
                    tool_calling: true,
                    ..Default::default()
                },
            ),
        );
        let chat_model: Arc<dyn tinyinference_llm::model::ChatModel<()>> = model;
        crate::agent::SessionHostBuilder::new()
            .with_session_history_locator(Arc::clone(&locator))
            .chat_model(chat_model)
            .tools(Vec::new())
            .workspace_dir(root.path().join("workspace"))
            .action_dir(root.path().to_path_buf())
            .tool_dispatcher(Box::new(tinytools_agent::dialect::NativeDialect))
            .build()
            .expect("session build")
    };
    for round in 1..=2 {
        let mut a = new_host(vec![format!("reply-a{round}")]);
        let mut b = new_host(vec![format!("reply-b{round}")]);
        assert_eq!(a.agent_definition_name(), b.agent_definition_name());
        if round == 2 {
            a.set_agent_definition_name("scheduled_agent");
            b.set_agent_definition_name("scheduled_agent");
        }
        assert_ne!(a.session_key(), b.session_key(), "round {round}");
        a.set_next_turn_overrides(scheduled_turn());
        b.set_next_turn_overrides(scheduled_turn());
        let prompt_a = format!("prompt-a{round}");
        let prompt_b = format!("prompt-b{round}");
        let (ra, rb) = tokio::join!(a.run_single(&prompt_a), b.run_single(&prompt_b));
        assert_eq!(ra.expect("run a"), format!("reply-a{round}"));
        assert_eq!(rb.expect("run b"), format!("reply-b{round}"));

        let text_a = transcript_text(&locator, a.session_key());
        let text_b = transcript_text(&locator, b.session_key());
        assert!(text_a.contains(&prompt_a) && text_a.contains(&format!("reply-a{round}")));
        assert!(text_b.contains(&prompt_b) && text_b.contains(&format!("reply-b{round}")));
        assert!(!text_a.contains(&prompt_b), "a's transcript holds b's turn");
        assert!(!text_b.contains(&prompt_a), "b's transcript holds a's turn");
    }
}
