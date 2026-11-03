use super::*;

#[test]
fn wrapping_retains_wide_and_combining_characters() {
    assert_eq!(wrap_cells("界e\u{301}界", 3), vec!["界e\u{301}", "界"]);
}

#[test]
fn large_transcript_uses_wide_offsets_and_only_returns_the_viewport() {
    let mut state = TranscriptState::new("test");
    for _ in 0..40000 {
        state.push_system("one line");
    }
    let mut cache = ViewportCache::default();
    let (rows, max_scroll) = cache.rows(&state, 80, 20, 0);
    assert_eq!(rows.len(), 20);
    assert_eq!(max_scroll, 79980);
    assert_eq!(rows[0].entry, 39990);
    let (rows, _) = cache.rows(&state, 80, 20, max_scroll);
    assert_eq!(rows[0].entry, 0);
    assert!(
        cache
            .blocks
            .iter()
            .filter(|block| !block.rows.is_empty())
            .count()
            <= 193
    );
}

#[test]
fn changing_a_message_invalidates_only_its_cached_block() {
    let mut state = TranscriptState::new("test");
    state.begin_user_turn("hi");
    let mut cache = ViewportCache::default();
    cache.rows(&state, 80, 20, 0);
    state.push_system("new");
    let (rows, _) = cache.rows(&state, 80, 20, 0);
    assert!(rows.iter().any(|row| row.text == "new"));
    assert_eq!(cache.recomputed, 1);
}

#[test]
fn idle_and_old_entry_edits_do_not_revisit_settled_history() {
    let mut state = TranscriptState::new("test");
    for _ in 0..10_000 {
        state.push_system("one line");
    }
    let mut cache = ViewportCache::default();
    cache.rows(&state, 80, 20, 0);
    assert_eq!(cache.recomputed, 10_000);
    cache.rows(&state, 80, 20, 0);
    assert_eq!(cache.recomputed, 0);
    state.toggle_entry(1);
    cache.rows(&state, 80, 20, 0);
    assert_eq!(cache.recomputed, 1);
    assert!(cache.resident.len() <= 193);
    state.push_system("tail");
    cache.rows(&state, 80, 20, 0);
    assert_eq!(cache.recomputed, 1);
}

#[test]
fn indexed_rows_equal_full_wrapping_after_updates_resizes_and_expired_journal() {
    let mut state = TranscriptState::new("test");
    state.apply_event(
        &serde_json::from_value(serde_json::json!({
            "event":"tool_call","client_id":"test","thread_id":"thread-1",
            "request_id":"turn","tool_call_id":"first","tool_name":"tool"
        }))
        .unwrap(),
    );
    for index in 0..150 {
        state.push_system(format!("entry {index} 界e\u{301}\nnext"));
    }
    let mut cache = ViewportCache::default();
    for step in 0..12 {
        if step == 3 {
            state.toggle_entry(0);
        }
        if step == 4 {
            state.apply_event(
                &serde_json::from_value(serde_json::json!({
                    "event":"tool_result","client_id":"test","thread_id":"thread-1",
                    "request_id":"turn","tool_call_id":"first",
                    "output":"a\nb\nc","success":true
                }))
                .unwrap(),
            );
        }
        if step == 5 {
            for _ in 0..140 {
                state.push_system("new tail");
            }
        }
        if step == 10 {
            state.clear();
            state.push_system("replacement");
        }
        let width = [7, 30, 80][step % 3];
        let reference: Vec<_> = state
            .entries()
            .iter()
            .enumerate()
            .flat_map(|(index, entry)| {
                entry_rows(entry, width)
                    .into_iter()
                    .map(move |text| (index, text))
            })
            .collect();
        for offset in [0, 5, 71, usize::MAX] {
            // A new explicit offset disables resize anchoring for this reference comparison.
            cache.offset = offset.wrapping_add(1);
            let (rows, max) = cache.rows(&state, width, 13, offset);
            assert_eq!(max, reference.len().saturating_sub(13));
            let top = max.saturating_sub(offset.min(max));
            assert_eq!(
                rows.iter()
                    .map(|row| (row.entry, row.text.clone()))
                    .collect::<Vec<_>>(),
                reference
                    .iter()
                    .skip(top)
                    .take(13)
                    .cloned()
                    .collect::<Vec<_>>()
            );
            assert!(cache.resident.len() <= 193);
        }
    }
}

#[test]
fn resize_keeps_scrolled_entry_visible_and_latest_follows_bottom() {
    let mut state = TranscriptState::new("test");
    for index in 0..100 {
        state.push_system(format!("entry {index}: {}", "long text ".repeat(8)));
    }
    let mut cache = ViewportCache::default();
    let (before, _) = cache.rows(&state, 80, 15, 100);
    let (after, _) = cache.rows(&state, 40, 31, 100);
    assert_eq!(before[0].entry, after[0].entry);
    assert_ne!(cache.resolved_offset(), 100);
    let (latest, _) = cache.rows(&state, 120, 15, 0);
    assert_eq!(latest.last().unwrap().entry, 99);
    assert_eq!(cache.resolved_offset(), 0);
}

#[test]
fn switching_between_cloned_transcripts_does_not_reuse_a_foreign_snapshot() {
    let mut state = TranscriptState::new("test");
    state.push_system("original");
    let mut fork = state.clone();
    let mut cache = ViewportCache::default();
    state.push_system("left");
    cache.rows(&state, 80, 20, 0);
    fork.push_system("right");
    let (rows, _) = cache.rows(&fork, 80, 20, 0);
    assert!(rows.iter().any(|row| row.text == "right"));
    assert!(!rows.iter().any(|row| row.text == "left"));
}

#[test]
fn changing_an_old_tools_height_updates_prefix_lookup_without_rebuilding_history() {
    use openhuman_rpc::embed::chat_surface::WebChannelEvent;
    let mut state = TranscriptState::new("test");
    let event = |name: &str, fields: serde_json::Value| {
        let mut value = serde_json::json!({
            "event":name,"client_id":"test","thread_id":"thread","request_id":"turn",
            "tool_call_id":"old","tool_name":"tool"
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        serde_json::from_value::<WebChannelEvent>(value).unwrap()
    };
    state.apply_event(&event("tool_call", serde_json::json!({})));
    for _ in 0..300 {
        state.push_system("settled");
    }
    let mut cache = ViewportCache::default();
    cache.rows(&state, 40, 20, 0);
    let before = cache.total_rows();
    state.apply_event(&event(
        "tool_result",
        serde_json::json!({"output":"first\nsecond\nthird\nfourth","success":true}),
    ));
    state.toggle_entry(0);
    cache.rows(&state, 40, 20, 0);
    assert_eq!(cache.recomputed, 1);
    assert!(cache.total_rows() > before);
    for offset in [0, 5, 589, usize::MAX] {
        let (rows, max) = cache.rows(&state, 40, 20, offset);
        assert_eq!(cache.recomputed, 0);
        let reference: Vec<_> = state
            .entries()
            .iter()
            .enumerate()
            .flat_map(|(index, entry)| {
                entry_rows(entry, 40)
                    .into_iter()
                    .map(move |text| (index, text))
            })
            .collect();
        let top = max.saturating_sub(offset.min(max));
        assert_eq!(
            rows.into_iter()
                .map(|row| (row.entry, row.text))
                .collect::<Vec<_>>(),
            reference.into_iter().skip(top).take(20).collect::<Vec<_>>()
        );
    }
    state.toggle_entry(0);
    cache.rows(&state, 40, 20, 0);
    assert_eq!(cache.recomputed, 1);
    assert_eq!(cache.total_rows(), before);
}
