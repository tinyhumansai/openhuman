//! End-to-end subconscious test with real Ollama, real memory, real SQLite.
//!
//! Requires Ollama running at localhost:11434 with a model loaded.
//! Run with: `cargo test --test subconscious_e2e -- --nocapture --ignored`

use std::sync::Arc;

use serde_json::json;

fn ci_safe_ingestion_config() -> openhuman_core::openhuman::memory::MemoryIngestionConfig {
    openhuman_core::openhuman::memory::MemoryIngestionConfig::default()
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
                taint: openhuman_core::openhuman::memory::MemoryTaint::Internal,
            },
            config: ci_safe_ingestion_config(),
        })
        .await
        .expect("ingest should succeed");
    result.document_id
}

/// Two-tick E2E test — verifies the structured tick model persists tick
/// state across runs. The first tick has no world baseline, so it
/// establishes one; subsequent ticks diff against it. (Exercising the full
/// diff → decide path additionally requires configured + synced memory
/// sources; this smoke test focuses on the tick lifecycle + state.)
#[tokio::test]
#[ignore] // requires running Ollama
async fn two_tick_e2e_with_real_ollama() {
    use openhuman_core::openhuman::embeddings::NoopEmbedding;
    use openhuman_core::openhuman::memory::UnifiedMemory;
    use openhuman_core::openhuman::subconscious::store;

    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();

    let memory = UnifiedMemory::new(workspace, Arc::new(NoopEmbedding), None).expect("init memory");

    // Ingest test data
    ingest_doc(
        &memory,
        "skill-gmail",
        "urgent-emails-batch1",
        "3 urgent emails in inbox",
        "Email 1: From alice@partner.com — Subject: URGENT: API contract deadline\n\
         Body: The API integration deadline has been moved to tomorrow.\n\n\
         Email 2: From boss@company.com — Subject: Re: Q1 Budget Review\n\
         Body: Need your updated numbers by 3pm today.",
    )
    .await;

    let mut config = openhuman_core::openhuman::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    config.heartbeat.enabled = true;
    config.heartbeat.inference_enabled = true;
    config.heartbeat.interval_minutes = 5;
    config.heartbeat.context_budget_tokens = 40_000;
    config.local_ai.runtime_enabled = true;
    config.local_ai.usage.subconscious = true;

    let engine = openhuman_core::openhuman::subconscious::SubconsciousEngine::new(&config);

    // Tick 1
    println!("\n=== TICK 1 ===");
    let result1 = engine.tick().await.expect("tick 1 should succeed");
    println!("  Duration: {}ms", result1.duration_ms);
    println!("  Response chars: {}", result1.response_chars);

    let last_tick1 =
        store::with_connection(workspace, store::get_last_tick_at).expect("read last tick");
    assert!(last_tick1 >= result1.tick_at);

    // Tick 2 with new data
    println!("\n=== TICK 2 ===");
    ingest_doc(
        &memory,
        "skill-gmail",
        "urgent-deadline-moved",
        "CRITICAL: API deadline moved to TOMORROW",
        "Email from alice@partner.com — Subject: RE: URGENT\n\
         Body: The deadline has been moved UP to tomorrow. This is now #1 priority.",
    )
    .await;

    let result2 = engine.tick().await.expect("tick 2 should succeed");
    println!("  Duration: {}ms", result2.duration_ms);
    println!("  Response chars: {}", result2.response_chars);

    let status = engine.status().await;
    println!("\n--- Status ---");
    println!("  Enabled: {}", status.enabled);
    println!("  Total ticks: {}", status.total_ticks);
    assert_eq!(status.total_ticks, 2);

    println!("\n=== E2E PASSED ===\n");
}
