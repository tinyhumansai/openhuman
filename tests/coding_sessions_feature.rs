//! Feature contract for TinyCortex Codex/Claude session discovery through the
//! OpenHuman adapter seam.

use std::fs;

use tempfile::tempdir;

use openhuman_core::openhuman::tinycortex::coding_session_status_for_roots;

#[test]
fn coding_session_sources_extract_human_turns_from_both_harnesses() {
    let temp = tempdir().expect("tempdir");
    let claude_root = temp.path().join("claude/projects/repo");
    let codex_root = temp.path().join("codex/sessions/2026/07/14");
    fs::create_dir_all(&claude_root).expect("claude root");
    fs::create_dir_all(&codex_root).expect("codex root");

    fs::write(
        claude_root.join("claude-session.jsonl"),
        concat!(
            "{\"type\":\"user\",\"sessionId\":\"claude-1\",\"cwd\":\"/repo\",\"timestamp\":\"2026-07-14T10:00:00Z\",\"message\":{\"content\":\"Use behavior-driven tests\"}}\n",
            "{\"type\":\"user\",\"isSidechain\":true,\"message\":{\"content\":\"subagent machine traffic\"}}\n"
        ),
    )
    .expect("claude fixture");
    fs::write(
        codex_root.join("rollout-codex-session.jsonl"),
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-1\",\"cwd\":\"/repo\"}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-07-14T10:00:00Z\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":[{\"type\":\"input_text\",\"text\":\"machine policy\"}]}}\n",
            "{\"type\":\"response_item\",\"timestamp\":\"2026-07-14T10:00:01Z\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Keep modules below 500 lines\"}]}}\n"
        ),
    )
    .expect("codex fixture");

    let statuses = coding_session_status_for_roots(
        &temp.path().join("claude/projects"),
        &temp.path().join("codex/sessions"),
    );

    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0].kind, "claude_code");
    assert_eq!(statuses[0].evidence_units, 1, "sidechain must be excluded");
    assert_eq!(statuses[1].kind, "codex");
    assert_eq!(
        statuses[1].evidence_units, 1,
        "developer policy must be excluded"
    );
}
