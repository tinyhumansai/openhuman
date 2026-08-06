//! `memory-ingest`: canonicalise and ingest 100 chat messages, then drain the
//! real extraction/admission/tree queue.

use anyhow::Result;
use chrono::{TimeZone, Utc};
use openhuman_core::core::event_bus::init_global;
use openhuman_core::openhuman::memory::ingest_pipeline::ingest_chat;
use openhuman_core::openhuman::memory::queue::drain_until_idle;
use tinycortex::memory::ingest::canonicalize::chat::{ChatBatch, ChatMessage};

use crate::harness::{fixture, measure, ProfileResult};

const INGEST_MESSAGE_COUNT: usize = 100;

fn ingestion_batch() -> ChatBatch {
    let messages = (0..INGEST_MESSAGE_COUNT)
        .map(|index| ChatMessage {
            author: if index % 2 == 0 { "alice" } else { "bob" }.into(),
            timestamp: Utc
                .timestamp_millis_opt(1_700_000_000_000 + index as i64 * 60_000)
                .single()
                .expect("valid profile timestamp"),
            text: format!(
                "Phoenix migration update {index}: staging p99 is 12ms and error rate is 0.001%. \
                 Alice owns the rollback runbook, Bob owns on-call coordination, and the \
                 phoenix_v2_enabled flag ramps Friday after billing-ledger verification."
            ),
            source_ref: Some(format!("profile://message/{index}")),
        })
        .collect();
    ChatBatch {
        platform: "profile".into(),
        channel_label: "library-benchmark".into(),
        messages,
    }
}

pub async fn run() -> Result<ProfileResult> {
    let fixture = fixture()?;
    let _ = init_global(256);
    eprintln!("[library-profile] memory-ingest: fixture + event bus ready");
    measure("memory-ingest", INGEST_MESSAGE_COUNT, None, |_rec| async {
        let result = ingest_chat(
            &fixture.config,
            "profile:chat:100",
            "profile-user",
            vec!["profile".into()],
            ingestion_batch(),
        )
        .await?;
        anyhow::ensure!(result.chunks_written > 0, "ingestion wrote no chunks");
        drain_until_idle(&fixture.config).await?;
        Ok(())
    })
    .await
}
