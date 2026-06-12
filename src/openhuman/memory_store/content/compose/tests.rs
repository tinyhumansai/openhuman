#[cfg(test)]
mod tests {
    use crate::openhuman::memory_store::chunks::types::{Chunk, Metadata, SourceKind, SourceRef};
    use crate::openhuman::memory_store::content::compose::chunk::{
        compose_chunk_file, rewrite_tags,
    };
    use crate::openhuman::memory_store::content::compose::summary::{
        compose_summary_md, rewrite_summary_tags, scope_short_label, SummaryComposeInput,
    };
    use crate::openhuman::memory_store::content::compose::yaml::{split_front_matter, yaml_scalar};
    use crate::openhuman::memory_store::content::compose::{
        MEMORY_ARTIFACT_FORMAT, OPENHUMAN_CORE_VERSION,
    };
    use crate::openhuman::memory_store::content::paths::SummaryTreeKind;
    use chrono::TimeZone;

    fn sample_chunk() -> Chunk {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: "abc123".into(),
            content: "## 2026-01-01T00:00:00Z — alice\nhello world".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice@example.com".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["person/Alice".into(), "org/Acme".into()],
                source_ref: Some(SourceRef::new("slack://m1".to_string())),
                path_scope: None,
            },
            token_count: 10,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        }
    }

    #[test]
    fn compose_produces_front_matter_and_body() {
        let chunk = sample_chunk();
        let (full, body) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        assert!(full_str.starts_with("---\n"), "must start with ---");
        assert!(full_str.contains("source_kind: chat"));
        assert!(full_str.contains("source_id: \"slack:#eng\""));
        assert!(full_str.contains("seq: 0"));
        assert!(full_str.contains("tags:"));
        assert!(full_str.contains("  - person/Alice"));
        assert!(full_str.ends_with("hello world"));
        assert_eq!(
            body,
            b"## 2026-01-01T00:00:00Z \xe2\x80\x94 alice\nhello world"
        );
    }

    #[test]
    fn compose_persists_path_scope_and_seeds_scoped_source_tag() {
        let mut chunk = sample_chunk();
        chunk.metadata.source_id = "notion:conn-1:page-123".into();
        chunk.metadata.path_scope = Some("notion:conn-1".into());

        let (full, _) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();

        assert!(full_str.contains("source_id: \"notion:conn-1:page-123\""));
        assert!(full_str.contains("path_scope: \"notion:conn-1\""));
        assert!(full_str.contains("  - source/notion-conn-1"));
        assert!(!full_str.contains("  - source/notion-conn-1-page-123"));
    }

    #[test]
    fn split_front_matter_round_trips() {
        let chunk = sample_chunk();
        let (full, body) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        let (fm, b) = split_front_matter(full_str).expect("split must succeed");
        assert!(fm.starts_with("---\n"));
        assert!(fm.ends_with("---\n"));
        assert_eq!(b.as_bytes(), body.as_slice());
    }

    #[test]
    fn rewrite_tags_preserves_body() {
        let chunk = sample_chunk();
        let (full, body) = compose_chunk_file(&chunk);
        let new_tags = vec!["person/Bob".into(), "project/Phoenix".into()];
        let rewritten = rewrite_tags(&full, &new_tags).unwrap();
        let rewritten_str = std::str::from_utf8(&rewritten).unwrap();
        assert!(rewritten_str.contains("  - person/Bob"));
        assert!(!rewritten_str.contains("  - person/Alice"));
        // Body must be unchanged.
        assert!(rewritten_str.ends_with(std::str::from_utf8(&body).unwrap()));
    }

    #[test]
    fn rewrite_tags_empty_list() {
        let chunk = sample_chunk();
        let (full, _) = compose_chunk_file(&chunk);
        let rewritten = rewrite_tags(&full, &[]).unwrap();
        let s = std::str::from_utf8(&rewritten).unwrap();
        assert!(s.contains("tags: []"));
        assert!(!s.contains("  - person/"));
    }

    #[test]
    fn yaml_scalar_quotes_special_characters() {
        assert_eq!(yaml_scalar("slack:#eng"), "\"slack:#eng\"");
        assert_eq!(yaml_scalar("hello world"), "hello world");
        assert_eq!(yaml_scalar(""), "\"\"");
    }

    fn sample_email_chunk() -> Chunk {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: "emailchunk1".into(),
            content: "---\nFrom: alice@example.com\nSubject: Hello\n\nHello there.".into(),
            metadata: Metadata {
                source_kind: SourceKind::Email,
                source_id: "gmail:alice@example.com|bob@example.com".into(),
                owner: "owner@example.com".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["gmail".into()],
                source_ref: None,
                path_scope: None,
            },
            token_count: 15,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        }
    }

    #[test]
    fn email_chunk_has_participants_list_and_alias() {
        let chunk = sample_email_chunk();
        let (full, _body) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        // participants block must be a YAML list
        assert!(
            full_str.contains("participants:"),
            "email chunk must have participants field; got:\n{full_str}"
        );
        assert!(
            full_str.contains("  - alice@example.com"),
            "alice must appear as list item; got:\n{full_str}"
        );
        assert!(
            full_str.contains("  - bob@example.com"),
            "bob must appear as list item; got:\n{full_str}"
        );
        // aliases block must be present
        assert!(
            full_str.contains("aliases:"),
            "email chunk must have aliases field; got:\n{full_str}"
        );
        assert!(
            full_str.contains("alice@example.com <-> bob@example.com: chunk 0"),
            "alias must encode participants; got:\n{full_str}"
        );
        // sender and thread_id must NOT appear
        assert!(
            !full_str.contains("sender:"),
            "email chunk must NOT have sender field; got:\n{full_str}"
        );
        assert!(
            !full_str.contains("thread_id:"),
            "email chunk must NOT have thread_id field; got:\n{full_str}"
        );
    }

    #[test]
    fn email_chunk_many_participants_alias_summarises() {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: "em2".into(),
            content: "body".into(),
            metadata: Metadata {
                source_kind: SourceKind::Email,
                source_id: "gmail:alice@x.com|bob@y.com|carol@z.com".into(),
                owner: "owner".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: None,
                path_scope: None,
            },
            token_count: 1,
            seq_in_source: 3,
            created_at: ts,
            partial_message: false,
        };
        let (full, _) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        assert!(
            full_str.contains("participants:"),
            "three-party chunk needs participants list; got:\n{full_str}"
        );
        // With 3 participants: first + "2 others"
        assert!(
            full_str.contains("alice@x.com <-> 2 others: chunk 3"),
            "alias with 3 participants must summarise; got:\n{full_str}"
        );
    }

    #[test]
    fn email_chunk_body_bytes_unchanged_by_extra_fields() {
        // Adding participants/aliases to front-matter must not affect body_bytes
        // (SHA-256 invariant: the hash is over body only, not front-matter).
        let chunk = sample_email_chunk();
        let (full, body) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        // Body must still appear at the end unmodified.
        assert!(
            full_str.ends_with(std::str::from_utf8(&body).unwrap()),
            "body bytes must appear unmodified after front-matter"
        );
        // body must equal chunk.content bytes
        assert_eq!(body, chunk.content.as_bytes());
    }

    #[test]
    fn chat_chunk_has_no_email_specific_fields() {
        let chunk = sample_chunk(); // source_kind = Chat
        let (full, _) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        assert!(
            !full_str.contains("aliases:"),
            "chat chunk must not have aliases field"
        );
        assert!(
            !full_str.contains("participants:"),
            "chat chunk must not have participants field"
        );
        assert!(
            !full_str.contains("sender:"),
            "chat chunk must not have sender field"
        );
        assert!(
            !full_str.contains("thread_id:"),
            "chat chunk must not have thread_id field"
        );
    }

    #[test]
    fn email_chunk_with_malformed_source_id_omits_extra_fields() {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: "xyz".into(),
            content: "body".into(),
            metadata: Metadata {
                source_kind: SourceKind::Email,
                source_id: "legacysourceid".into(), // no `gmail:` prefix → parse fails
                owner: "owner".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: None,
                path_scope: None,
            },
            token_count: 1,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        let (full, _) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        // Malformed source_id → no email extras, no panic.
        assert!(!full_str.contains("aliases:"));
        assert!(!full_str.contains("participants:"));
        assert!(!full_str.contains("sender:"));
    }

    // ─── summary compose tests ────────────────────────────────────────────────

    fn sample_summary_input(
        tree_kind: SummaryTreeKind,
        scope: &str,
        level: u32,
    ) -> SummaryComposeInput<'static> {
        let ts_start = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let ts_end = chrono::Utc.timestamp_millis_opt(1_700_086_400_000).unwrap();
        let sealed = chrono::Utc.timestamp_millis_opt(1_700_090_000_000).unwrap();
        // Leak the strings so they have 'static lifetime for this test helper.
        // Only used in tests, not production code.
        let scope: &'static str = Box::leak(scope.to_string().into_boxed_str());
        SummaryComposeInput {
            summary_id: "summary:L1:abc",
            tree_kind,
            tree_id: "tree-id-001",
            tree_scope: scope,
            level,
            child_ids: Box::leak(
                vec!["child-1".to_string(), "child-2".to_string()].into_boxed_slice(),
            ),
            child_basenames: None,
            child_count: 2,
            time_range_start: ts_start,
            time_range_end: ts_end,
            sealed_at: sealed,
            body: "This is the summariser output.\n",
        }
    }

    #[test]
    fn compose_source_summary_has_required_front_matter() {
        let input = sample_summary_input(SummaryTreeKind::Source, "gmail:alice@x.com|bob@y.com", 1);
        let composed = compose_summary_md(&input);
        let fm = &composed.front_matter;
        assert!(fm.starts_with("---\n"), "front-matter must start with ---");
        assert!(fm.ends_with("---\n"), "front-matter must end with ---\\n");
        assert!(fm.contains("kind: summary"), "must have kind: summary");
        assert!(
            fm.contains("tree_kind: source"),
            "must have tree_kind: source"
        );
        assert!(fm.contains("level: 1"), "must have level");
        assert!(fm.contains("child_count: 2"), "must have child_count");
        assert!(
            fm.contains(&format!(
                "openhuman_core_version: {}",
                OPENHUMAN_CORE_VERSION
            )),
            "must stamp the core version"
        );
        assert!(
            fm.contains(&format!(
                "memory_artifact_format: {}",
                MEMORY_ARTIFACT_FORMAT
            )),
            "must stamp the artifact format epoch"
        );
        assert!(
            fm.contains("  - \"[[child-1]]\""),
            "must list child ids as Obsidian wikilinks; got:\n{fm}"
        );
        assert!(
            fm.contains("  - \"[[child-2]]\""),
            "must list child ids as Obsidian wikilinks; got:\n{fm}"
        );
        assert!(
            fm.contains("  - source/"),
            "source-tree summary must seed source tag; got:\n{fm}"
        );
        // aliases must mention the scope
        assert!(fm.contains("aliases:"), "must have aliases");
        assert!(
            composed.body == "This is the summariser output.\n",
            "body must be the summariser text"
        );
        assert!(composed.full.ends_with("This is the summariser output.\n"));
    }

    #[test]
    fn children_are_emitted_as_obsidian_wikilinks() {
        // Contract: every entry in `children:` must be wrapped in `[[…]]` so
        // Obsidian's graph view draws a summary→child edge. The YAML scalar is
        // quoted because of the leading `[` — both forms below are required.
        let input = sample_summary_input(SummaryTreeKind::Source, "gmail:alice@x.com", 1);
        let composed = compose_summary_md(&input);
        let fm = &composed.front_matter;
        for id in ["child-1", "child-2"] {
            let expected = format!("  - \"[[{id}]]\"");
            assert!(
                fm.contains(&expected),
                "child id {id} must be emitted as a quoted wikilink ({expected}); got:\n{fm}"
            );
            // Belt-and-braces: the bare id must NOT appear as a plain scalar
            // (i.e. unwrapped). The wikilink form contains the id, so we
            // search for the bare list-item form.
            let plain = format!("  - {id}\n");
            assert!(
                !fm.contains(&plain),
                "child id {id} must not be emitted as a plain scalar; got:\n{fm}"
            );
        }
    }

    #[test]
    fn child_basename_overrides_replace_chunk_id_in_wikilink() {
        // L1 seals: each child's wikilink should point at the
        // raw archive file basename, not the chunk_id hash. Without
        // this override the link would be `[[<32-char hex>]]` and
        // Obsidian wouldn't find a matching file (the chunk-store
        // copy under `email/<scope>/...` is gone after the
        // raw_refs migration).
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let child_ids = vec!["abc123hash".to_string(), "def456hash".to_string()];
        let overrides: Vec<Option<String>> = vec![
            Some("1700000000000_msg-id-1".into()),
            None, // second child has no override → falls back to sanitize_filename
        ];
        let input = SummaryComposeInput {
            summary_id: "summary:L1:test",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 1,
            child_ids: &child_ids,
            child_basenames: Some(&overrides),
            child_count: 2,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body: "L1 body",
        };
        let composed = compose_summary_md(&input);
        let fm = &composed.front_matter;
        // First child uses the override (raw archive basename).
        assert!(
            fm.contains(r#"  - "[[1700000000000_msg-id-1]]""#),
            "first child must use override basename; got:\n{fm}"
        );
        // Second child has None override — fall back to chunk_id.
        assert!(
            fm.contains(r#"  - "[[def456hash]]""#),
            "None override must fall back to sanitize_filename; got:\n{fm}"
        );
    }

    #[test]
    fn structured_child_summary_id_is_sanitised_in_wikilink() {
        // Real-world case: an L2 summary lists child L1 summaries by their
        // structured id (e.g. `summary:L1:UUID`). Colons are illegal in
        // Windows NTFS filenames, so `summary_rel_path` writes the file as
        // `summary-L1-UUID.md`. The wikilink target must match that basename
        // — i.e. colons must be converted to dashes — otherwise Obsidian
        // cannot resolve the link and the graph stays disconnected.
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let child_id = "summary:L1:b9fa5f08-bf79-41a7-a5c8-2d87883d5c01";
        let expected_basename = "summary-L1-b9fa5f08-bf79-41a7-a5c8-2d87883d5c01";
        let input = SummaryComposeInput {
            summary_id: "summary:L2:cc9a1224",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 2,
            child_ids: &[child_id.to_string()],
            child_basenames: None,
            child_count: 1,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body: "L2 body",
        };
        let composed = compose_summary_md(&input);
        let fm = &composed.front_matter;
        let expected = format!("  - \"[[{expected_basename}]]\"");
        assert!(
            fm.contains(&expected),
            "structured child id must be sanitised to filename basename in wikilink; \
             expected line: {expected}; got:\n{fm}"
        );
        // Raw colon-bearing id must NOT appear inside `[[…]]` — that wikilink
        // would not resolve in Obsidian.
        assert!(
            !fm.contains(&format!("[[{child_id}]]")),
            "raw structured id with colons must not appear inside wikilink; got:\n{fm}"
        );
    }

    #[test]
    fn compose_global_summary_alias_format() {
        let input = sample_summary_input(SummaryTreeKind::Global, "global", 0);
        let composed = compose_summary_md(&input);
        assert!(
            composed.front_matter.contains("tree_kind: global"),
            "must have tree_kind: global"
        );
        assert!(
            composed.front_matter.contains("global digest"),
            "alias must mention 'global digest'"
        );
    }

    #[test]
    fn compose_topic_summary_alias_format() {
        let input = sample_summary_input(SummaryTreeKind::Topic, "person:alex-johnson", 1);
        let composed = compose_summary_md(&input);
        assert!(
            composed.front_matter.contains("tree_kind: topic"),
            "must have tree_kind: topic"
        );
        assert!(
            composed.front_matter.contains("topic"),
            "alias must mention topic entity"
        );
    }

    #[test]
    fn compose_summary_with_zero_children() {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let input = SummaryComposeInput {
            summary_id: "summary:L0:empty",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 0,
            child_ids: &[],
            child_basenames: None,
            child_count: 0,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body: "empty",
        };
        let composed = compose_summary_md(&input);
        assert!(composed.front_matter.contains("children: []"));
        assert!(composed.front_matter.contains("child_count: 0"));
    }

    #[test]
    fn compose_summary_same_start_end_date_single_date_alias() {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let input = SummaryComposeInput {
            summary_id: "summary:L1:sameday",
            tree_kind: SummaryTreeKind::Global,
            tree_id: "t1",
            tree_scope: "global",
            level: 1,
            child_ids: &["child-a".to_string()],
            child_basenames: None,
            child_count: 1,
            time_range_start: ts,
            time_range_end: ts, // same as start
            sealed_at: ts,
            body: "day recap",
        };
        let composed = compose_summary_md(&input);
        // Alias must contain just one date, not "date–date"
        let alias_line = composed
            .front_matter
            .lines()
            .find(|l| l.contains("L1") && l.contains("global digest"))
            .expect("alias line must be present");
        // The date should appear exactly once (no en-dash range)
        let date_str = ts.format("%Y-%m-%d").to_string();
        assert!(
            alias_line.contains(&date_str),
            "alias must contain the date; got: {alias_line}"
        );
        // Must not contain an en-dash (range indicator)
        assert!(
            !alias_line.contains('\u{2013}'),
            "same-day alias must not have en-dash range; got: {alias_line}"
        );
    }

    #[test]
    fn scope_short_label_two_participants() {
        let label = scope_short_label("gmail:alice@x.com|bob@y.com");
        assert_eq!(label, "alice@x.com \u{2194} bob@y.com");
    }

    #[test]
    fn scope_short_label_many_participants() {
        let label = scope_short_label("gmail:alice@x.com|bob@y.com|carol@z.com");
        assert_eq!(label, "alice@x.com + 2 others");
    }

    #[test]
    fn scope_short_label_non_gmail_returns_raw() {
        let label = scope_short_label("slack:#general");
        assert_eq!(label, "slack:#general");
    }

    #[test]
    fn rewrite_summary_tags_delegates_to_rewrite_tags() {
        // compose a summary, then rewrite its tags — body must stay unchanged.
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let input = SummaryComposeInput {
            summary_id: "sum:L1:rwttest",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 1,
            child_ids: &["c1".to_string()],
            child_basenames: None,
            child_count: 1,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body: "summary body text",
        };
        let composed = compose_summary_md(&input);
        let file_bytes = composed.full.as_bytes();
        let new_tags = vec!["person/Alice-Smith".to_string(), "topic/Memory".to_string()];
        let rewritten = rewrite_summary_tags(file_bytes, &new_tags).unwrap();
        let rewritten_str = std::str::from_utf8(&rewritten).unwrap();
        assert!(rewritten_str.contains("  - person/Alice-Smith"));
        assert!(rewritten_str.contains("  - topic/Memory"));
        assert!(!rewritten_str.contains("tags: []"));
        assert!(rewritten_str.contains(&format!(
            "openhuman_core_version: {}",
            OPENHUMAN_CORE_VERSION
        )));
        assert!(rewritten_str.contains(&format!(
            "memory_artifact_format: {}",
            MEMORY_ARTIFACT_FORMAT
        )));
        // Body must be unchanged
        assert!(rewritten_str.ends_with("summary body text"));
    }

    #[test]
    fn rewrite_summary_tags_backfills_missing_provenance() {
        let file =
            b"---\nid: legacy\nkind: summary\ntags: []\naliases:\n  - legacy\n---\nlegacy body";
        let rewritten = rewrite_summary_tags(file, &["person/Alice".to_string()]).unwrap();
        let rewritten_str = std::str::from_utf8(&rewritten).unwrap();
        assert!(rewritten_str.contains(&format!(
            "openhuman_core_version: {}",
            OPENHUMAN_CORE_VERSION
        )));
        assert!(rewritten_str.contains(&format!(
            "memory_artifact_format: {}",
            MEMORY_ARTIFACT_FORMAT
        )));
        assert!(rewritten_str.ends_with("legacy body"));
    }
}
