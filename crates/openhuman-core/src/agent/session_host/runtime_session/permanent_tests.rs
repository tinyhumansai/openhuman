use super::*;
use std::{collections::HashSet, sync::Arc};
use tinyagents_session::transcript::{session_stem, SessionRef};
use tinyinference_llm::message::Message;
use tinytools::ToolSpec;

fn spec(name: &str) -> Arc<ToolSpec> {
    Arc::new(ToolSpec {
        name: name.into(),
        description: "d".into(),
        parameters: serde_json::json!({"type": "object"}),
    })
}

#[test]
fn permanent_catalogue_uses_the_openhuman_marker_and_closing_tag() {
    let prefix = PrefixSnapshot::new(vec![Message::system("host prefix")]);
    let names: HashSet<String> = ["B_TOOL".to_string(), "A_TOOL".to_string()].into();
    let refreshed = tinyagents_runtime::refresh_prefix(
        &prefix,
        &[spec("B_TOOL"), spec("A_TOOL"), spec("OTHER")],
        &names,
        "<openhuman-permanent-tools>\n",
        "</openhuman-permanent-tools>",
    )
    .expect("catalogue is appended");
    let last = refreshed.messages().last().unwrap().text();
    assert!(last.starts_with("<openhuman-permanent-tools>\n"));
    assert!(last.ends_with("</openhuman-permanent-tools>"));
    assert!(last.find("A_TOOL").unwrap() < last.find("B_TOOL").unwrap());
    assert!(!last.contains("OTHER"));
    assert_eq!(refreshed.messages()[0].text(), "host prefix");
}

#[test]
fn transcript_stem_for_a_thread_and_agent_is_stable_and_timestamp_free() {
    let a = session_stem(&SessionRef::scoped("thread-1", "orchestrator"));
    let b = session_stem(&SessionRef::scoped("thread-1", "orchestrator"));
    assert_eq!(a, b);
    assert_ne!(
        a,
        session_stem(&SessionRef::scoped("thread-2", "orchestrator"))
    );
    assert!(!a.chars().any(|c| c == '/' || c == '\\'));
    assert_eq!(
        a,
        "thread-1-2c9d1afa3c382fdf440b08a5f1f46fef.orchestrator-57c4fd2d66355aa9fd1aea283ce94119"
    );
}
