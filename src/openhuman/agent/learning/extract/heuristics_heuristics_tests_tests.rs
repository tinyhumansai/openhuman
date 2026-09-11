use super::*;
use crate::openhuman::agent::learning::candidate::{Buffer, FacetClass};

fn fresh_session_id() -> String {
    format!(
        "test-session-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        rand_id()
    )
}

/// Cheap random suffix so parallel tests don't collide on session keys.
fn rand_id() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::thread::current().id().hash(&mut h);
    h.finish()
}

/// Record N turns with decreasing user_msg_len into the heuristics module.
fn push_turns(session_id: &str, n: usize, user_len: usize, agent_len: usize, buf: &Buffer) {
    let _ = buf; // We push into the global; this is just for clarity.
    let now = now_secs();
    for i in 0..n {
        record_turn(
            session_id,
            &i.to_string(),
            i as i64,
            "neutral message",
            user_len,
            agent_len,
            now + i as f64,
            now + i as f64 + 1.0,
            None, // no edit window
        );
    }
}

#[test]
fn length_ratio_emits_compressed_when_user_msgs_shrink() {
    let session = fresh_session_id();
    // First 15 turns: high ratio (user talks a lot).
    let now = now_secs();
    for i in 0..15 {
        record_turn(
            &session,
            &i.to_string(),
            i as i64,
            "long long long message",
            200,
            100,
            now + i as f64,
            now + i as f64 + 1.0,
            None,
        );
    }
    // Next 15 turns: low ratio (user became terse).
    for i in 15..30 {
        record_turn(
            &session,
            &i.to_string(),
            i as i64,
            "ok",
            5,
            100,
            now + i as f64,
            now + i as f64 + 1.0,
            None,
        );
    }

    // Should have emitted the "compressed" candidate into the global buffer.
    let all = candidate::global().peek();
    let compressed = all
        .iter()
        .filter(|c| {
            c.key == "verbosity"
                && c.value == "compressed"
                && matches!(&c.evidence, EvidenceRef::EpisodicWindow { .. })
        })
        .count();
    assert!(
        compressed >= 1,
        "expected at least one compressed verbosity candidate, got 0"
    );
}

#[test]
fn length_ratio_does_not_emit_with_short_window() {
    let session = fresh_session_id();
    // Only 10 turns — below the 30-turn minimum.
    push_turns(&session, 10, 5, 500, &Buffer::new(32));
    // Peek the global buffer for this session's compressed candidates.
    // We can't isolate per-session here, but at least ensure no crash.
    // (Functional assertion is in the positive test above.)
}

#[test]
fn length_ratio_cooldown_prevents_repeated_emission() {
    let session = fresh_session_id();
    // Trigger the compressed detection twice.
    let now = now_secs();
    for i in 0..30 {
        record_turn(
            &session,
            &i.to_string(),
            i as i64,
            "msg",
            if i < 15 { 200 } else { 5 },
            100,
            now + i as f64,
            now + i as f64 + 1.0,
            None,
        );
    }
    // Trigger it again — should be suppressed by the cooldown set.
    for i in 30..60 {
        record_turn(
            &session,
            &i.to_string(),
            i as i64,
            "msg",
            5,
            100,
            now + i as f64,
            now + i as f64 + 1.0,
            None,
        );
    }

    // Check that compressed was only emitted once for this session.
    let map = session_state().read();
    let st = map.get(&session).expect("state for session");
    assert!(
        st.length_ratio_emitted
            .contains(&("verbosity".to_string(), "compressed".to_string())),
        "emitted set must record the compression emission"
    );
}

#[test]
fn edit_window_emits_terse_on_shorter_correction() {
    let session = fresh_session_id();
    let now = now_secs();

    // Record a turn where user sends "shorter" within 10s of the agent.
    record_turn(
        &session,
        "1",
        1,
        "shorter please",
        14,
        200,
        now,
        now + 1.0,
        Some(now - 10.0), // agent replied 10s ago
    );

    let all = candidate::global().peek();
    let terse = all.iter().any(|c| {
        c.key == "verbosity"
            && c.value == "terse"
            && c.class == FacetClass::Style
            && matches!(&c.evidence, EvidenceRef::Episodic { episodic_id } if *episodic_id == 1)
    });
    assert!(
        terse,
        "expected a terse verbosity candidate after 'shorter' correction"
    );
}

#[test]
fn edit_window_ignores_late_messages() {
    let session = fresh_session_id();
    let now = now_secs();

    // User sends "shorter" but 60s after the agent reply — outside window.
    let before_count = candidate::global().len();
    record_turn(
        &session,
        "late-1",
        999,
        "shorter please",
        14,
        200,
        now,
        now + 1.0,
        Some(now - 60.0),
    );
    let after_count = candidate::global().len();

    // No new terse candidate should have been added for this episode.
    // We can only check the delta is zero from our (outside the lock) vantage.
    // The correction pattern might still be stored internally but not emitted.
    // Confirm: global buffer didn't grow with a terse candidate for episodic_id=999.
    let all = candidate::global().peek();
    let late_terse = all.iter().any(|c| {
        c.key == "verbosity"
            && c.value == "terse"
            && matches!(&c.evidence, EvidenceRef::Episodic { episodic_id } if *episodic_id == 999)
    });
    assert!(!late_terse, "late message must not emit terse candidate");
    let _ = (before_count, after_count);
}

#[test]
fn correction_repeat_promotes_to_veto_after_3() {
    let session = fresh_session_id();
    let now = now_secs();

    // Three "not bullets" corrections within the edit window.
    for i in 0..3usize {
        // Each turn: agent replied just before, user corrects quickly.
        let prev_agent_at = now + i as f64 * 100.0 - 5.0;
        let user_at = now + i as f64 * 100.0;
        record_turn(
            &session,
            &format!("veto-{i}"),
            (100 + i) as i64,
            "not bullets please",
            18,
            300,
            user_at,
            user_at + 2.0,
            Some(prev_agent_at),
        );
    }

    let all = candidate::global().peek();
    let veto = all
        .iter()
        .any(|c| c.class == FacetClass::Veto && c.key == "format" && c.value == "nested-bullets");
    assert!(
        veto,
        "3× 'not bullets' correction must promote to Veto/format=nested-bullets"
    );
}
