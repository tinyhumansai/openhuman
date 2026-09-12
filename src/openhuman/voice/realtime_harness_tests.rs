use super::*;
use serde_json::json;

#[test]
fn extracts_the_last_user_string_message() {
    let messages = vec![
        json!({ "role": "system", "content": "be nice" }),
        json!({ "role": "user", "content": "first" }),
        json!({ "role": "assistant", "content": "ok" }),
        json!({ "role": "user", "content": "what is the weather" }),
    ];
    assert_eq!(extract_prompt(&messages), "what is the weather");
}

#[test]
fn joins_multimodal_text_parts() {
    let messages = vec![json!({
        "role": "user",
        "content": [
            { "type": "text", "text": "hello" },
            { "type": "text", "text": "there" },
        ],
    })];
    assert_eq!(extract_prompt(&messages), "hello there");
}

#[test]
fn returns_empty_when_no_user_message() {
    let messages = vec![json!({ "role": "assistant", "content": "hi" })];
    assert_eq!(extract_prompt(&messages), "");
    assert_eq!(extract_prompt(&[]), "");
}

#[test]
fn history_pairs_keep_user_and_assistant_turns_and_drop_system() {
    let messages = vec![
        json!({ "role": "system", "content": "eleven agent prompt" }),
        json!({ "role": "user", "content": "what is the weather" }),
        json!({ "role": "assistant", "content": "sunny" }),
        json!({ "role": "user", "content": "what about tomorrow" }),
    ];
    let pairs = messages_to_history_pairs(&messages);
    assert_eq!(
        pairs,
        vec![
            ("user".to_string(), "what is the weather".to_string()),
            ("assistant".to_string(), "sunny".to_string()),
            ("user".to_string(), "what about tomorrow".to_string()),
        ]
    );
}

#[test]
fn history_pairs_flatten_multimodal_and_skip_empty() {
    let messages = vec![
        json!({ "role": "user", "content": [{ "type": "text", "text": "hello" }] }),
        json!({ "role": "assistant", "content": "   " }),
    ];
    let pairs = messages_to_history_pairs(&messages);
    assert_eq!(pairs, vec![("user".to_string(), "hello".to_string())]);
}

#[test]
fn spoken_delta_forwards_only_top_level_assistant_text() {
    assert_eq!(
        spoken_delta(&AgentProgress::TextDelta {
            delta: "hey there".to_string(),
            iteration: 1,
        }),
        Some("hey there")
    );
    // Whitespace-only deltas carry word boundaries and are still spoken text —
    // the empty-skip lives in the forwarder, not here.
    assert_eq!(
        spoken_delta(&AgentProgress::TextDelta {
            delta: " ".to_string(),
            iteration: 2,
        }),
        Some(" ")
    );
}

#[test]
fn spoken_delta_suppresses_internal_events() {
    // Reasoning must never be voiced.
    assert_eq!(
        spoken_delta(&AgentProgress::ThinkingDelta {
            delta: "let me think".to_string(),
            iteration: 1,
        }),
        None
    );
    // A delegated sub-agent's narration is internal, not the spoken answer.
    assert_eq!(
        spoken_delta(&AgentProgress::SubagentTextDelta {
            agent_id: "a".to_string(),
            task_id: "t".to_string(),
            delta: "fetching inbox".to_string(),
            iteration: 1,
        }),
        None
    );
    // Lifecycle events carry no spoken text.
    assert_eq!(
        spoken_delta(&AgentProgress::TurnCompleted { iterations: 1 }),
        None
    );
}

#[test]
fn speak_back_armed_for_a_genuine_answer_turn() {
    assert!(should_arm_speak_back("summarize my unread emails"));
    assert!(should_arm_speak_back("what's on my calendar tomorrow?"));
}

#[test]
fn speak_back_suppressed_for_a_read_back_turn() {
    // The bare prefix, and the real renderer shape (prefix + blank line +
    // payload, possibly with leading whitespace) must both be recognised so
    // the spoken copy never re-arms into an unbounded loop.
    assert!(!should_arm_speak_back(VOICE_READBACK_PREFIX));
    assert!(!should_arm_speak_back(&format!(
        "{VOICE_READBACK_PREFIX}\n\nHere is your inbox summary."
    )));
    assert!(!should_arm_speak_back(&format!(
        "   \n{VOICE_READBACK_PREFIX} trailing payload"
    )));
}

// A reply carrying only text and no tool call ends the turn, so an instruction
// to announce work before doing it makes the announcement the final answer:
// the caller hears a promise and never gets the summary. Observed live.
#[test]
fn the_directive_forbids_announcing_work_instead_of_doing_it() {
    assert!(
        VOICE_DIRECTIVE.contains("Do NOT announce"),
        "the directive must forbid a preface-only reply"
    );
    for banned in ["first say one short spoken sentence", "then proceed"] {
        assert!(
            !VOICE_DIRECTIVE.contains(banned),
            "directive must not ask the model to speak before acting: {banned:?}"
        );
    }
}

// The provider synthesises on sentence boundaries, so an unterminated line is
// buffered instead of spoken — the same defect that made the relay's fillers
// inaudible for eight seconds.
#[test]
fn every_handoff_line_is_a_terminated_sentence() {
    for line in VOICE_HANDOFF_LINES {
        assert!(
            line.trim_end().ends_with('.'),
            "handoff line must end a sentence: {line:?}"
        );
        assert!(
            line.ends_with(' '),
            "trailing space keeps speech unglued: {line:?}"
        );
        assert!(
            !line.contains("...") && !line.contains('…'),
            "an ellipsis is not a sentence end, in either form: {line:?}"
        );
    }
}

// A caller who hits the deadline twice in one call should not hear the same
// words back.
#[test]
fn handoff_lines_rotate() {
    let first = next_handoff_line();
    let second = next_handoff_line();
    assert_ne!(first, second);
}

#[test]
fn read_back_payload_is_the_text_to_speak() {
    assert_eq!(
        readback_payload(&format!(
            "{VOICE_READBACK_PREFIX}\n\nHere is your inbox summary."
        )),
        Some("Here is your inbox summary.")
    );
    // The renderer may prepend whitespace; the payload must survive it intact.
    assert_eq!(
        readback_payload(&format!(
            "  \n{VOICE_READBACK_PREFIX} two things need attention"
        )),
        Some("two things need attention")
    );
    // A prefix with nothing behind it is still a read-back — it just has
    // nothing to say, and must not be relayed as a question.
    assert_eq!(readback_payload(VOICE_READBACK_PREFIX), Some(""));
}

#[test]
fn ordinary_prompts_are_not_read_backs() {
    assert_eq!(readback_payload("summarize my emails"), None);
    assert_eq!(readback_payload("please read my emails to me"), None);
}

#[test]
fn recognition_artefacts_are_content_free() {
    // What the provider actually relayed for a pause during a filler-heavy
    // turn, plus the shapes next to it.
    assert!(is_content_free("..."));
    assert!(is_content_free("…"));
    assert!(is_content_free(" ? "));
    assert!(is_content_free("-"));
}

#[test]
fn real_questions_are_not_content_free() {
    assert!(!is_content_free("summarize my emails"));
    // A single digit is a real answer to "how many?" — and scripts other than
    // Latin must never be mistaken for punctuation.
    assert!(!is_content_free("3"));
    assert!(!is_content_free("मेरे ईमेल पढ़ो"));
    assert!(!is_content_free("总结我的邮件"));
}

#[test]
fn failure_notice_is_limited_to_turns_the_user_is_waiting_on() {
    assert!(is_answerable_prompt("summarize my emails"));
    // A read-back's answer is already in chat; a notice would report a failure
    // for a request the user never made.
    assert!(!is_answerable_prompt(&format!(
        "{VOICE_READBACK_PREFIX}\n\nHere is your inbox summary."
    )));
    assert!(!is_answerable_prompt("..."));
}
