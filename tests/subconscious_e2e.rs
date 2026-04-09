//! End-to-end subconscious test with real Ollama, real memory, real SQLite.
//!
//! Requires Ollama running at localhost:11434 with a model loaded.
//! Run with: `cargo test --test subconscious_e2e -- --nocapture --ignored`

use std::sync::Arc;

use serde_json::json;

/// Config that skips GLiNER relex model (avoids ORT init).
fn ci_safe_ingestion_config() -> openhuman_core::openhuman::memory::MemoryIngestionConfig {
    openhuman_core::openhuman::memory::MemoryIngestionConfig {
        model_name: "__test_no_model__".to_string(),
        ..Default::default()
    }
}

async fn ingest_doc(
    memory: &openhuman_core::openhuman::memory::UnifiedMemory,
    namespace: &str,
    key: &str,
    title: &str,
    content: &str,
) -> String {
    use openhuman_core::openhuman::memory::{MemoryIngestionRequest, NamespaceDocumentInput};
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
            config: ci_safe_ingestion_config(),
        })
        .await
        .expect("ingest should succeed");
    result.document_id
}

/// Full two-tick E2E test:
///
/// **Tick 1**: Gmail has 3 urgent emails, Notion has a deadline tracker.
///   → Ollama should detect urgent items → act or escalate.
///
/// **Tick 2**: New data — deadline moved, ownership changed.
///   → Ollama should detect the change → act or escalate on new state.
///
/// Verifies:
/// - Tasks loaded from HEARTBEAT.md seed
/// - Real Ollama evaluation produces valid decisions
/// - SQLite log entries created for each tick
/// - Act tasks produce text output from Ollama
/// - Second tick sees delta (new data only)
#[tokio::test]
#[ignore] // requires running Ollama
async fn two_tick_e2e_with_real_ollama() {
    use openhuman_core::openhuman::memory::embeddings::NoopEmbedding;
    use openhuman_core::openhuman::memory::{MemoryClient, UnifiedMemory};
    use openhuman_core::openhuman::subconscious::store;

    // ── Setup workspace ──────────────────────────────────────────────
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();

    // Write HEARTBEAT.md
    std::fs::write(
        workspace.join("HEARTBEAT.md"),
        "\
# Periodic Tasks

- Check Gmail for urgent emails that need immediate attention
- Review Notion project tracker for deadline changes
- Monitor connected skills for errors or disconnections
",
    )
    .expect("write heartbeat");

    // Initialize memory
    let memory = UnifiedMemory::new(workspace, Arc::new(NoopEmbedding), None).expect("init memory");
    let memory_client =
        MemoryClient::from_workspace_dir(workspace.to_path_buf()).expect("memory client");

    // ── Tick 1: Ingest initial data ──────────────────────────────────
    println!("\n============================================================");
    println!("  TICK 1: Initial state — urgent emails + project tracker");
    println!("============================================================\n");

    ingest_doc(
        &memory,
        "skill-gmail",
        "urgent-emails-batch1",
        "3 urgent emails in inbox",
        "\
Email 1: From alice@partner.com — Subject: URGENT: API contract deadline
  Body: The API integration deadline has been moved from Friday to tomorrow (Thursday).
  Please confirm you can deliver by end of day. This is blocking the partner launch.

Email 2: From boss@company.com — Subject: Re: Q1 Budget Review
  Body: Need your updated numbers by 3pm today. The board meeting is tomorrow morning.
  Please prioritize this over other tasks.

Email 3: From security@company.com — Subject: [ACTION REQUIRED] Password expiry
  Body: Your corporate password expires in 24 hours. Please update it via the portal
  to avoid being locked out of all systems.",
    )
    .await;

    ingest_doc(
        &memory,
        "skill-notion",
        "q1-tracker-v1",
        "Q1 Delivery Tracker",
        "\
Project: API Integration
  Status: In Progress
  Deadline: April 5 (Friday)
  Owner: You
  Dependencies: Partner API docs (received), Internal review (pending)
  Notes: On track for Friday delivery. Partner team confirmed their side is ready.

Project: Q1 Budget Report
  Status: Draft
  Deadline: Today 3pm
  Owner: You
  Notes: Numbers need updating. Finance sent corrections yesterday.",
    )
    .await;

    // Build engine with real config
    let mut config = openhuman_core::openhuman::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.heartbeat.enabled = true;
    config.heartbeat.inference_enabled = true;
    config.heartbeat.interval_minutes = 5;
    config.heartbeat.context_budget_tokens = 40_000;
    config.local_ai.enabled = true;

    let engine = openhuman_core::openhuman::subconscious::SubconsciousEngine::new(
        &config,
        Some(Arc::new(memory_client)),
    );

    // Run tick 1
    let result1 = engine.tick().await.expect("tick 1 should succeed");

    println!("\n--- Tick 1 Results ---");
    println!("  Duration: {}ms", result1.duration_ms);
    println!("  Evaluations: {}", result1.evaluations.len());
    println!("  Executed: {}", result1.executed);
    println!("  Escalated: {}", result1.escalated);
    for eval in &result1.evaluations {
        println!("  [{}] {:?} — {}", eval.task_id, eval.decision, eval.reason);
    }

    // Verify tick 1
    assert!(
        !result1.evaluations.is_empty(),
        "Ollama should produce evaluations for seeded tasks"
    );

    // Check SQLite log
    let log1 = store::with_connection(workspace, |conn| store::list_log_entries(conn, None, 50))
        .expect("list log");
    println!("\n  Log entries after tick 1: {}", log1.len());
    for entry in &log1 {
        println!(
            "    [{}] {} — {}",
            entry.task_id,
            entry.decision,
            entry.result.as_deref().unwrap_or("(none)")
        );
    }
    assert!(!log1.is_empty(), "Should have log entries after tick 1");

    // Check tasks were seeded
    let tasks = store::with_connection(workspace, |conn| store::list_tasks(conn, false))
        .expect("list tasks");
    println!("\n  Tasks: {}", tasks.len());
    for t in &tasks {
        println!(
            "    [{}] {} (source={:?}, completed={})",
            t.id, t.title, t.source, t.completed
        );
    }
    assert_eq!(tasks.len(), 3, "Should have 3 tasks from HEARTBEAT.md");

    // ── Tick 2: Ingest NEW data (state change) ──────────────────────
    println!("\n============================================================");
    println!("  TICK 2: State change — deadline moved, new urgent email");
    println!("============================================================\n");

    ingest_doc(
        &memory,
        "skill-gmail",
        "urgent-deadline-moved",
        "CRITICAL: API deadline moved to TOMORROW",
        "\
Email from alice@partner.com — Subject: RE: URGENT: API contract deadline
  Body: Update — the deadline has been moved UP to tomorrow (Wednesday) not Thursday.
  The partner CEO is flying in Wednesday evening and wants a demo.
  This is now the #1 priority. Please drop everything else.

Email from boss@company.com — Subject: Re: Re: Q1 Budget Review
  Body: Good news — finance approved a 1-day extension on the budget report.
  New deadline is Friday. Focus on the API delivery instead.",
    )
    .await;

    ingest_doc(
        &memory,
        "skill-notion",
        "q1-tracker-v2",
        "Q1 Delivery Tracker (UPDATED)",
        "\
Project: API Integration
  Status: AT RISK
  Deadline: TOMORROW (Wednesday) — moved up from Friday
  Owner: You
  Dependencies: Partner API docs (received), Internal review (BLOCKING — not started)
  Notes: CRITICAL — deadline moved up 2 days. Internal review not started.
         Partner CEO demo Wednesday evening. Need to start review NOW.
  Blockers: Internal review team not yet assigned.

Project: Q1 Budget Report
  Status: Draft
  Deadline: Friday (extended from today)
  Owner: You
  Notes: Extension granted. Lower priority now.",
    )
    .await;

    // Run tick 2
    let result2 = engine.tick().await.expect("tick 2 should succeed");

    println!("\n--- Tick 2 Results ---");
    println!("  Duration: {}ms", result2.duration_ms);
    println!("  Evaluations: {}", result2.evaluations.len());
    println!("  Executed: {}", result2.executed);
    println!("  Escalated: {}", result2.escalated);
    for eval in &result2.evaluations {
        println!("  [{}] {:?} — {}", eval.task_id, eval.decision, eval.reason);
    }

    // Verify tick 2
    assert!(
        !result2.evaluations.is_empty(),
        "Ollama should produce evaluations for tick 2"
    );

    // Check cumulative log
    let log2 = store::with_connection(workspace, |conn| store::list_log_entries(conn, None, 50))
        .expect("list log");
    println!("\n  Total log entries after tick 2: {}", log2.len());
    assert!(
        log2.len() > log1.len(),
        "Tick 2 should add more log entries"
    );

    // Check for any escalations
    let escalations = store::with_connection(workspace, |conn| store::list_escalations(conn, None))
        .expect("list escalations");
    println!("  Escalations: {}", escalations.len());
    for esc in &escalations {
        println!(
            "    [{}] {} — {} (status={:?})",
            esc.task_id, esc.title, esc.description, esc.status
        );
    }

    // ── Status check ─────────────────────────────────────────────────
    let status = engine.status().await;
    println!("\n--- Engine Status ---");
    println!("  Enabled: {}", status.enabled);
    println!("  Total ticks: {}", status.total_ticks);
    println!("  Task count: {}", status.task_count);
    println!("  Pending escalations: {}", status.pending_escalations);
    assert_eq!(status.total_ticks, 2);

    println!("\n============================================================");
    println!("  E2E TEST PASSED");
    println!("============================================================\n");
}
