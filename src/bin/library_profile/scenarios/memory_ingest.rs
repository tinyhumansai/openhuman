//! `memory-ingest`: canonicalise and ingest 100 chat messages, then drain the
//! real extraction/admission/tree queue.
//!
//! # Why this still names the engine crate (#5560)
//!
//! It is measuring the in-process engine on purpose, and routing it through
//! `memory::binding` would not be a migration — it would be a different
//! measurement. `MemoryIngest::ingest_chat` on the contract hands items to
//! whatever driver the workspace bound, which in this binary is the **null**
//! driver (no module is loaded), so the number would be the cost of doing
//! nothing. And `queue::drain_until_idle` has no contract member at all: the
//! queue is not a capability family, and the point of this scenario is that the
//! extraction/admission/tree jobs actually run before the timer stops.
//!
//! So the honest fix here is a **feature gate, not a migration** — this binary
//! carries `required-features = ["rss-bench"]` and is local/dev only, and
//! `rss-bench` is deliberately absent from `scripts/ci/product-features.txt`,
//! so nothing here is in the shipped product graph. What it needs from the
//! manifest is for `tinymemory-core` to become `optional = true` with
//! `rss-bench` enabling it; a bin target cannot use dev-dependencies, so
//! demoting the crate to dev-only without that would break this build with the
//! gate on, in a configuration no CI lane compiles.

use anyhow::Result;
use chrono::{TimeZone, Utc};
use tinycortex::memory::ingest::canonicalize::chat::{ChatBatch, ChatMessage};
use tinymemory_core::ingest_pipeline::ingest_chat;
use tinymemory_core::queue::drain_until_idle;

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
    openhuman_core::core::bus::init().await.expect("bus init");
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
