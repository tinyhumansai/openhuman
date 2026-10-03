use super::*;
use std::sync::Arc;
use tinymemory::conformance::ReferenceEngine;
use tinymemory::{MemoryEngine, MemoryMeta, Role, SourceKind, StoreItem, Turn};

fn conversation(channel: &str) -> StoreItem {
    let mut meta = MemoryMeta::from_source(SourceKind::Conversation, None);
    meta.tags = vec![crate::memory::conversations::buffer::channel_tag(channel)];
    StoreItem::Conversation {
        turns: vec![Turn::new(Role::User, format!("hello from {channel}"))],
        meta,
    }
}

#[tokio::test]
async fn clears_only_the_channels_conversations() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    let engine = Arc::new(ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    engine.store(conversation("discord")).await.unwrap();
    engine.store(conversation("telegram")).await.unwrap();

    assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 1);
    assert_eq!(engine.len(), 1);
}

#[tokio::test]
async fn memory_off_clears_nothing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace_dir = tmp.path().to_path_buf();
    config.memory.engine = String::new();
    assert_eq!(clear_channel_memory(&config, "discord").await.unwrap(), 0);
}
