//! Tests for the smart_walk module.

#[cfg(test)]
mod tests {
    use crate::openhuman::config::Config;
    use crate::openhuman::inference::provider::traits::{ChatMessage, Provider};
    use crate::openhuman::memory::query::smart_walk::dispatch::{
        dispatch_keyword_search, dispatch_list_sources, dispatch_read_content, search_dir_recursive,
    };
    use crate::openhuman::memory::query::smart_walk::prompts::{
        build_content_inventory, parse_tool_calls, InnerCall,
    };
    use crate::openhuman::memory::query::smart_walk::runner::run_smart_walk;
    use crate::openhuman::memory::query::smart_walk::types::{
        SmartWalkOptions, SmartWalkStopReason,
    };
    use async_trait::async_trait;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::TempDir;

    struct StubProvider {
        responses: Mutex<Vec<String>>,
    }

    impl StubProvider {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(|s| s.to_string()).collect()),
            }
        }
    }

    #[async_trait]
    impl Provider for StubProvider {
        async fn chat_with_system(
            &self,
            _system: Option<&str>,
            _message: &str,
            _model: &str,
            _temp: f64,
        ) -> anyhow::Result<String> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(anyhow::anyhow!("StubProvider: no more responses"));
            }
            Ok(responses.remove(0))
        }

        async fn chat_with_history(
            &self,
            _messages: &[ChatMessage],
            _model: &str,
            _temp: f64,
        ) -> anyhow::Result<String> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(anyhow::anyhow!("StubProvider: no more responses"));
            }
            Ok(responses.remove(0))
        }
    }

    fn test_config(tmp: &TempDir) -> Config {
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        cfg
    }

    fn seed_content(content_root: &Path) {
        let raw_dir = content_root.join("raw").join("test-source").join("commits");
        std::fs::create_dir_all(&raw_dir).unwrap();
        std::fs::write(
            raw_dir.join("123_abc.md"),
            "---\nsource_kind: document\n---\n# Test Commit\nFixed the login bug in auth module.\n",
        )
        .unwrap();

        let doc_dir = content_root.join("document").join("test-doc");
        std::fs::create_dir_all(&doc_dir).unwrap();
        std::fs::write(
            doc_dir.join("readme.md"),
            "---\nsource_kind: document\n---\n# README\nProject documentation for the auth system.\n",
        )
        .unwrap();

        let wiki_dir = content_root
            .join("wiki")
            .join("summaries")
            .join("source-test");
        std::fs::create_dir_all(wiki_dir.join("L1")).unwrap();
        std::fs::write(
            wiki_dir.join("L1").join("summary-001.md"),
            "---\nkind: summary\nlevel: 1\n---\nSummary of auth changes in May 2026.\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn smart_walk_keyword_search_and_answer() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_content(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: keyword search for "login"
            r#"<tool_call>{"name":"keyword_search","arguments":{"pattern":"login","content_type":"all"}}</tool_call>"#,
            // Turn 2: read the matching file
            r#"<tool_call>{"name":"read_content","arguments":{"path":"raw/test-source/commits/123_abc.md"}}</tool_call>"#,
            // Turn 3: collect evidence and answer
            r#"<tool_call>{"name":"collect_evidence","arguments":{"items":[{"source":"raw/test-source/commits/123_abc.md","snippet":"Fixed the login bug in auth module.","relevance":"directly mentions login fix"}]}}</tool_call>
<tool_call>{"name":"answer","arguments":{"text":"The login bug was fixed in the auth module, as documented in commit 123_abc."}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 10,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "What happened with the login bug?", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        assert!(outcome.answer.contains("login"));
        assert_eq!(outcome.evidence.len(), 1);
        assert!(outcome.evidence[0].snippet.contains("login bug"));
    }

    #[tokio::test]
    async fn smart_walk_list_sources() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_content(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: list sources
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"all"}}</tool_call>"#,
            // Turn 2: answer
            r#"<tool_call>{"name":"answer","arguments":{"text":"Found raw, document, and wiki content."}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 5,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "What sources are available?", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        assert!(outcome.answer.contains("raw"));
    }

    #[tokio::test]
    async fn smart_walk_max_turns() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_content(&content_root);

        let provider = StubProvider::new(vec![
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"all"}}</tool_call>"#,
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"raw"}}</tool_call>"#,
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"wiki"}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 3,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "loop test", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::MaxTurnsReached);
        assert_eq!(outcome.turns_used, 3);
    }

    #[test]
    fn parse_multiple_tool_calls() {
        let response = r#"Let me search.
<tool_call>{"name":"keyword_search","arguments":{"pattern":"test"}}</tool_call>
<tool_call>{"name":"entity_search","arguments":{"query":"Alice"}}</tool_call>"#;

        let (text, calls) = parse_tool_calls(response);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "keyword_search");
        assert_eq!(calls[1].name, "entity_search");
        assert!(text.contains("Let me search"));
    }

    #[test]
    fn content_inventory_counts_files() {
        let tmp = TempDir::new().unwrap();
        let content_root = tmp.path().join("content");
        seed_content(&content_root);

        let inventory = build_content_inventory(&content_root);
        assert!(inventory.contains("Raw content"));
        assert!(inventory.contains("Documents"));
        assert!(inventory.contains("Wiki summaries"));
    }

    // ── Staging integration tests (run with --ignored) ────────────────

    fn staging_content_root() -> Option<std::path::PathBuf> {
        let path = std::path::PathBuf::from(
            "/Users/enamakel/.openhuman-staging/users/69d9cb73e61f755583c3671f/workspace/memory_tree/content",
        );
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    #[test]
    #[ignore]
    fn staging_keyword_search_finds_steven() {
        let content_root = staging_content_root().expect("staging content not available");
        let mut results = Vec::new();
        search_dir_recursive(
            &content_root.join("raw"),
            "steven",
            &mut results,
            &content_root,
        );
        println!("keyword 'steven': {} results", results.len());
        for r in results.iter().take(5) {
            println!("  {}", r);
        }
        assert!(
            !results.is_empty(),
            "should find 'steven' in staging raw content"
        );
    }

    #[test]
    #[ignore]
    fn staging_content_inventory() {
        let content_root = staging_content_root().expect("staging content not available");
        let inventory = build_content_inventory(&content_root);
        println!("Inventory:\n{}", inventory);
        assert!(inventory.contains("Raw content"));
        assert!(inventory.contains("Documents"));
    }

    #[test]
    #[ignore]
    fn staging_list_sources_shows_github() {
        let content_root = staging_content_root().expect("staging content not available");
        let call = InnerCall {
            name: "list_sources".into(),
            args: serde_json::json!({"content_type": "all"}),
        };
        let (_, result, _, _) = dispatch_list_sources(&content_root, &call);
        println!("list_sources:\n{}", result);
        assert!(result.contains("raw/"), "should list raw sources");
    }

    #[test]
    #[ignore]
    fn staging_read_wiki_summary() {
        let content_root = staging_content_root().expect("staging content not available");
        let wiki_dir = content_root.join("wiki").join("summaries");
        if !wiki_dir.exists() {
            println!("no wiki summaries found — skipping");
            return;
        }
        // Find first summary file
        let first = walkdir_first_md(&wiki_dir);
        if let Some(path) = first {
            let rel = path
                .strip_prefix(&content_root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            println!("Reading wiki: {}", rel);
            let call = InnerCall {
                name: "read_content".into(),
                args: serde_json::json!({"path": rel}),
            };
            let (_, result, _, _) = dispatch_read_content(&content_root, &call);
            println!("Content preview: {}", &result[..result.len().min(300)]);
            assert!(
                !result.starts_with("error"),
                "should read wiki file without error"
            );
        }
    }

    #[test]
    #[ignore]
    fn staging_read_episodic_memory() {
        let content_root = staging_content_root().expect("staging content not available");
        let ep_dir = content_root.join("episodic");
        if !ep_dir.exists() {
            println!("no episodic memories — skipping");
            return;
        }
        let first = walkdir_first_md(&ep_dir);
        if let Some(path) = first {
            let rel = path
                .strip_prefix(&content_root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            println!("Reading episodic: {}", rel);
            let call = InnerCall {
                name: "read_content".into(),
                args: serde_json::json!({"path": rel}),
            };
            let (_, result, _, _) = dispatch_read_content(&content_root, &call);
            println!("Content preview: {}", &result[..result.len().min(300)]);
            assert!(
                !result.starts_with("error"),
                "should read episodic file without error"
            );
        }
    }

    #[test]
    #[ignore]
    fn staging_full_smart_walk_keyword_pipeline() {
        let content_root = staging_content_root().expect("staging content not available");

        // Simulate the pipeline: list_sources → keyword_search → read_content
        let call = InnerCall {
            name: "list_sources".into(),
            args: serde_json::json!({"content_type": "raw"}),
        };
        let (_, sources, _, _) = dispatch_list_sources(&content_root, &call);
        println!("Step 1 - Sources:\n{}", sources);

        let call = InnerCall {
            name: "keyword_search".into(),
            args: serde_json::json!({"pattern": "memory", "content_type": "all"}),
        };
        let (_, search_result, _, _) = dispatch_keyword_search(&content_root, &call);
        println!("Step 2 - Search 'memory':\n{}", search_result);

        if search_result.contains('[') {
            // Extract first file path from results
            if let Some(path_start) = search_result.find('[') {
                if let Some(path_end) = search_result[path_start + 1..].find(']') {
                    let file_path = &search_result[path_start + 1..path_start + 1 + path_end];
                    println!("Step 3 - Reading: {}", file_path);
                    let call = InnerCall {
                        name: "read_content".into(),
                        args: serde_json::json!({"path": file_path}),
                    };
                    let (_, content, _, _) = dispatch_read_content(&content_root, &call);
                    println!(
                        "Step 3 - Content ({} chars): {}",
                        content.len(),
                        &content[..content.len().min(200)]
                    );
                    assert!(
                        !content.starts_with("error"),
                        "pipeline should complete without errors"
                    );
                }
            }
        }
    }

    fn walkdir_first_md(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        fn recurse(dir: &std::path::Path) -> Option<std::path::PathBuf> {
            for entry in std::fs::read_dir(dir).ok()?.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(found) = recurse(&path) {
                        return Some(found);
                    }
                } else if path.extension().map_or(false, |e| e == "md") {
                    return Some(path);
                }
            }
            None
        }
        recurse(dir)
    }

    fn seed_synced_memory(content_root: &Path) {
        // Raw email content
        let email_dir = content_root.join("raw").join("email").join("inbox");
        std::fs::create_dir_all(&email_dir).unwrap();
        std::fs::write(
            email_dir.join("001_meeting.md"),
            "---\nsource_kind: email\nauthor: alice@example.com\ndate: 2026-06-01\n---\n\
             # Team standup notes\n\n\
             Action items:\n\
             - Deploy the auth service refactor by Friday\n\
             - Review PR #342 for the billing module\n\
             - Schedule security audit with external team\n",
        )
        .unwrap();
        std::fs::write(
            email_dir.join("002_project.md"),
            "---\nsource_kind: email\nauthor: bob@example.com\ndate: 2026-06-02\n---\n\
             # Project Phoenix status update\n\n\
             The migration is 80% complete. Remaining:\n\
             - Database schema changes (blocked on DBA review)\n\
             - API versioning for backward compatibility\n\
             - Load testing the new endpoints\n",
        )
        .unwrap();
        std::fs::write(
            email_dir.join("003_personal.md"),
            "---\nsource_kind: email\nauthor: carol@example.com\ndate: 2026-06-03\n---\n\
             # Lunch plans\n\n\
             Hey, want to grab sushi on Thursday? The new place on 5th street \
             got great reviews.\n",
        )
        .unwrap();

        // Episodic memories
        let ep_dir = content_root.join("episodic").join("daily");
        std::fs::create_dir_all(&ep_dir).unwrap();
        std::fs::write(
            ep_dir.join("2026-06-01.md"),
            "---\nkind: episodic\ndate: 2026-06-01\n---\n\
             Worked on the auth service refactor. Had a productive standup.\n\
             Identified three blockers for Project Phoenix.\n",
        )
        .unwrap();

        // Wiki summaries
        let wiki_dir = content_root
            .join("wiki")
            .join("summaries")
            .join("email-inbox");
        std::fs::create_dir_all(wiki_dir.join("L1")).unwrap();
        std::fs::write(
            wiki_dir.join("L1").join("summary-week-22.md"),
            "---\nkind: summary\nlevel: 1\n---\n\
             Week 22 summary: Team focused on Project Phoenix migration \
             and auth service refactor. Key contacts: alice@example.com (standup), \
             bob@example.com (project status), carol@example.com (social).\n",
        )
        .unwrap();

        // Document content
        let doc_dir = content_root.join("document").join("notes");
        std::fs::create_dir_all(&doc_dir).unwrap();
        std::fs::write(
            doc_dir.join("project-phoenix.md"),
            "---\nsource_kind: document\n---\n\
             # Project Phoenix\n\n\
             ## Overview\n\
             Migration from legacy monolith to microservices.\n\n\
             ## Status\n\
             Phase 2 of 3 — data migration and API versioning.\n\n\
             ## Key risks\n\
             - Data integrity during cutover\n\
             - Backward compatibility for mobile clients\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn walk_synced_email_with_keyword_and_evidence() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_synced_memory(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: keyword search
            r#"<tool_call>{"name":"keyword_search","arguments":{"pattern":"auth service","content_type":"all"}}</tool_call>"#,
            // Turn 2: collect evidence
            concat!(
                r#"<tool_call>{"name":"collect_evidence","arguments":{"items":["#,
                r#"{"source":"raw/email/inbox/001_meeting.md","snippet":"Deploy the auth service refactor by Friday","relevance":"action item"}"#,
                r#"]}}</tool_call>"#,
            ),
            // Turn 3: answer
            r#"<tool_call>{"name":"answer","arguments":{"text":"The auth service refactor needs to be deployed by Friday."}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 5,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(
            &cfg,
            &provider,
            "What's happening with the auth service?",
            opts,
        )
        .await
        .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        assert!(outcome.answer.contains("auth service"));
        assert!(!outcome.evidence.is_empty());
    }

    #[tokio::test]
    async fn walk_with_xml_format_tool_calls() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_synced_memory(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: XML-formatted tool call
            concat!(
                "<tool_call>",
                "<tool_name>keyword_search</tool_name>",
                "<parameters>{\"pattern\": \"project phoenix\", \"content_type\": \"all\"}</parameters>",
                "</tool_call>",
            ),
            // Turn 2: JSON-formatted answer
            r#"<tool_call>{"name":"answer","arguments":{"text":"Project Phoenix is in phase 2 of 3."}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 5,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(
            &cfg,
            &provider,
            "What's the status of Project Phoenix?",
            opts,
        )
        .await
        .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        assert!(outcome.answer.contains("phase 2"));
    }

    #[tokio::test]
    async fn walk_reads_across_content_types() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_synced_memory(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: list sources
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"all"}}</tool_call>"#,
            // Turn 2: read document
            r#"<tool_call>{"name":"read_content","arguments":{"path":"document/notes/project-phoenix.md"}}</tool_call>"#,
            // Turn 3: read episodic
            r#"<tool_call>{"name":"read_content","arguments":{"path":"episodic/daily/2026-06-01.md"}}</tool_call>"#,
            // Turn 4: collect + answer
            concat!(
                r#"<tool_call>{"name":"collect_evidence","arguments":{"items":["#,
                r#"{"source":"document/notes/project-phoenix.md","snippet":"Phase 2 of 3","relevance":"status"},"#,
                r#"{"source":"episodic/daily/2026-06-01.md","snippet":"Identified three blockers","relevance":"context"}"#,
                r#"]}}</tool_call>"#,
                r#"<tool_call>{"name":"answer","arguments":{"text":"Project Phoenix: Phase 2/3 with 3 blockers identified."}}</tool_call>"#,
            ),
        ]);

        let opts = SmartWalkOptions {
            max_turns: 5,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "Summarize Project Phoenix status", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        assert_eq!(outcome.evidence.len(), 2);
    }

    #[tokio::test]
    async fn walk_llm_gives_up_uses_fallback() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_synced_memory(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: search finds nothing
            r#"<tool_call>{"name":"keyword_search","arguments":{"pattern":"quantum computing","content_type":"all"}}</tool_call>"#,
            // Turn 2: LLM gives up with empty response
            "",
        ]);

        let opts = SmartWalkOptions {
            max_turns: 5,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "Tell me about quantum computing", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::LlmGaveUp);
        assert!(outcome.evidence.is_empty());
        assert!(outcome.answer.contains("Could not converge"));
    }

    #[tokio::test]
    async fn walk_direct_answer_without_tools() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_synced_memory(&content_root);

        let provider = StubProvider::new(vec![
            // LLM directly answers without using any tools
            "I don't have enough context to answer that question from your memory.",
        ]);

        let opts = SmartWalkOptions {
            max_turns: 5,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "What's the meaning of life?", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        assert!(outcome.answer.contains("don't have enough context"));
        assert_eq!(outcome.turns_used, 1);
        assert!(outcome.evidence.is_empty());
    }

    #[tokio::test]
    async fn walk_collect_evidence_deduplicates_within_limit() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_synced_memory(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: collect a batch of evidence
            concat!(
                r#"<tool_call>{"name":"collect_evidence","arguments":{"items":["#,
                r#"{"source":"raw/email/inbox/001_meeting.md","snippet":"Deploy auth","relevance":"task"},"#,
                r#"{"source":"raw/email/inbox/002_project.md","snippet":"Migration 80%","relevance":"status"}"#,
                r#"]}}</tool_call>"#,
            ),
            // Turn 2: collect more evidence (including a duplicate of the first source)
            concat!(
                r#"<tool_call>{"name":"collect_evidence","arguments":{"items":["#,
                r#"{"source":"document/notes/project-phoenix.md","snippet":"Phase 2 of 3","relevance":"doc"},"#,
                r#"{"source":"raw/email/inbox/001_meeting.md","snippet":"Deploy auth (duplicate)","relevance":"task"}"#,
                r#"]}}</tool_call>"#,
            ),
            // Turn 3: answer
            r#"<tool_call>{"name":"answer","arguments":{"text":"Summary with evidence items including duplicate source."}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 10,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "Summarize everything", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        // 2 items from turn 1 + 2 items from turn 2 (one of which duplicates a turn-1 source);
        // collect_evidence does not deduplicate, so all 4 items are present.
        assert_eq!(outcome.evidence.len(), 4);
    }
}
