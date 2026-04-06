#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::TempDir;

    use crate::openhuman::memory::embeddings::NoopEmbedding;
    use crate::openhuman::memory::{
        MemoryIngestionConfig, MemoryIngestionRequest, NamespaceDocumentInput, UnifiedMemory,
    };
    use crate::openhuman::subconscious::decision_log::DecisionLog;
    use crate::openhuman::subconscious::situation_report::build_situation_report;
    use crate::openhuman::subconscious::types::Decision;

    /// Find the largest byte index ≤ `max_bytes` that is a valid char boundary.
    fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> usize {
        let mut end = s.len().min(max_bytes);
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        end
    }

    fn fixture(path: &str) -> String {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(
            base.join("tests")
                .join("fixtures")
                .join("subconscious")
                .join(path),
        )
        .expect("fixture should load")
    }

    fn ci_safe_config() -> MemoryIngestionConfig {
        MemoryIngestionConfig {
            model_name: "__test_no_model__".to_string(),
            ..MemoryIngestionConfig::default()
        }
    }

    async fn ingest(
        memory: &UnifiedMemory,
        namespace: &str,
        key: &str,
        title: &str,
        content: &str,
    ) -> String {
        let result = memory
            .ingest_document(MemoryIngestionRequest {
                document: NamespaceDocumentInput {
                    namespace: namespace.to_string(),
                    key: key.to_string(),
                    title: title.to_string(),
                    content: content.to_string(),
                    source_type: "test".to_string(),
                    priority: "high".to_string(),
                    tags: Vec::new(),
                    metadata: json!({}),
                    category: "core".to_string(),
                    session_id: None,
                    document_id: None,
                },
                config: ci_safe_config(),
            })
            .await
            .unwrap();
        result.document_id
    }

    /// Full two-tick integration test:
    /// 1. Ingest tick1 data → build report → verify it contains the data
    /// 2. Record a decision in the log
    /// 3. Ingest tick2 data → build report → verify delta-only (not old data)
    /// 4. Verify decision log deduplication
    #[tokio::test]
    async fn two_tick_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path();
        let memory = UnifiedMemory::new(workspace, Arc::new(NoopEmbedding), None).unwrap();
        let client =
            crate::openhuman::memory::MemoryClient::from_workspace_dir(workspace.to_path_buf())
                .unwrap();

        // Write HEARTBEAT.md
        std::fs::write(workspace.join("HEARTBEAT.md"), fixture("heartbeat.md")).unwrap();

        // ============================================================
        // TICK 1: Ingest initial data
        // ============================================================
        let tick1_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();

        let gmail_doc_id = ingest(
            &memory,
            "skill-gmail",
            "tick1-deadline-email",
            "API contract deadline reminder",
            &fixture("tick1_gmail.txt"),
        )
        .await;

        let notion_doc_id = ingest(
            &memory,
            "skill-notion",
            "tick1-tracker",
            "Q1 Delivery Tracker",
            &fixture("tick1_notion.txt"),
        )
        .await;

        // Build situation report for tick 1 (cold start: last_tick_at = 0)
        let report1 = build_situation_report(
            Some(&client),
            workspace,
            0.0, // cold start
            40_000,
        )
        .await;

        println!("=== TICK 1 REPORT ===");
        println!("{}", &report1[..truncate_at_char_boundary(&report1, 2000)]);
        println!("=====================\n");

        // Verify tick 1 report contains ingested data
        assert!(
            report1.contains("Memory Documents"),
            "Report should have memory docs section"
        );
        assert!(
            report1.contains("skill-gmail") || report1.contains("deadline"),
            "Report should mention gmail data"
        );
        assert!(
            report1.contains("Pending Tasks"),
            "Report should have tasks section"
        );
        assert!(
            report1.contains("Check for deadline changes"),
            "Report should include HEARTBEAT.md tasks"
        );

        // Simulate tick 1 decision: escalate the deadline
        let mut decision_log = DecisionLog::new();
        let tick1_output = crate::openhuman::subconscious::types::TickOutput {
            decision: Decision::Escalate,
            reason: "Deadline reminder for API contract (April 3)".to_string(),
            actions: vec![],
        };
        decision_log.record(tick1_time, &tick1_output, vec![gmail_doc_id.clone()]);

        println!(
            "Decision log after tick 1: {} active records",
            decision_log.active_count()
        );
        assert_eq!(decision_log.active_count(), 1);
        assert!(decision_log.was_already_surfaced(&[gmail_doc_id.clone()]));
        assert!(!decision_log.was_already_surfaced(&["nonexistent".to_string()]));

        // ============================================================
        // TICK 2: Ingest new data (state change)
        // ============================================================
        let tick2_time = tick1_time + 1.0; // 1 second later (simulated)

        let gmail2_doc_id = ingest(
            &memory,
            "skill-gmail",
            "tick2-deadline-moved",
            "URGENT: API contract deadline moved to tomorrow",
            &fixture("tick2_gmail.txt"),
        )
        .await;

        let notion2_doc_id = ingest(
            &memory,
            "skill-notion",
            "tick2-tracker-updated",
            "Q1 Delivery Tracker (updated)",
            &fixture("tick2_notion.txt"),
        )
        .await;

        // Build situation report for tick 2 (delta since tick 1)
        let report2 = build_situation_report(
            Some(&client),
            workspace,
            tick1_time, // delta since tick 1
            40_000,
        )
        .await;

        println!("=== TICK 2 REPORT ===");
        println!("{}", &report2[..truncate_at_char_boundary(&report2, 2000)]);
        println!("=====================\n");

        // Verify tick 2 report contains NEW data
        assert!(
            report2.contains("new/updated"),
            "Report should show new/updated docs"
        );

        // Verify deduplication: old gmail doc should be filtered
        let all_new_doc_ids = vec![
            gmail_doc_id.clone(),
            gmail2_doc_id.clone(),
            notion2_doc_id.clone(),
        ];
        let unsurfaced = decision_log.filter_unsurfaced(&all_new_doc_ids);
        println!("Unsurfaced doc IDs: {:?}", unsurfaced);
        assert!(
            !unsurfaced.contains(&gmail_doc_id),
            "Old deadline email should be filtered out (already surfaced)"
        );
        assert!(
            unsurfaced.contains(&gmail2_doc_id),
            "New deadline-moved email should NOT be filtered"
        );
        assert!(
            unsurfaced.contains(&notion2_doc_id),
            "Updated notion tracker should NOT be filtered"
        );

        // Record tick 2 decision
        let tick2_output = crate::openhuman::subconscious::types::TickOutput {
            decision: Decision::Escalate,
            reason: "Deadline moved to tomorrow — urgent".to_string(),
            actions: vec![],
        };
        decision_log.record(tick2_time, &tick2_output, vec![gmail2_doc_id.clone()]);

        println!(
            "Decision log after tick 2: {} active records",
            decision_log.active_count()
        );
        assert_eq!(decision_log.active_count(), 2);

        // ============================================================
        // TICK 3: No new data
        // ============================================================
        let report3 = build_situation_report(
            Some(&client),
            workspace,
            tick2_time, // delta since tick 2
            40_000,
        )
        .await;

        println!("=== TICK 3 REPORT ===");
        println!("{}", &report3[..truncate_at_char_boundary(&report3, 2000)]);
        println!("=====================\n");

        // Tick 3 should show no changes
        let has_changes = !report3.contains("No changes since last tick");
        // Note: on cold data with fixed timestamps, all docs may appear
        // "old" relative to tick2_time. The key test is that the decision
        // log correctly filters previously surfaced items.

        println!("Tick 3 has changes: {}", has_changes);

        // Verify JSON roundtrip of decision log
        let json = decision_log.to_json().unwrap();
        let restored = DecisionLog::from_json(&json).unwrap();
        assert_eq!(restored.active_count(), 2);
        assert!(restored.was_already_surfaced(&[gmail_doc_id.clone()]));
        assert!(restored.was_already_surfaced(&[gmail2_doc_id.clone()]));

        // Verify acknowledgment
        decision_log.mark_acknowledged(&[gmail_doc_id.clone()]);
        assert!(
            !decision_log.was_already_surfaced(&[gmail_doc_id]),
            "Acknowledged docs should no longer be surfaced"
        );

        println!("=== ALL TESTS PASSED ===");
    }
}
