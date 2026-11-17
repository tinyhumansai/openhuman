use super::*;

#[tokio::test]
async fn disconnect_rpc_reports_stamped_memory_count() {
    let (_tmp, mut config) = isolated_test_config();
    use_linked_channel_module(&mut config).await;
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "discord-token-abc".into(),
        guild_id: Some("guild-1".into()),
        channel_id: Some("channel-2".into()),
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });
    config.save().await.expect("save Discord setup");
    let engine = Arc::new(ReferenceEngine::new());
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    engine
        .store(channel_memory_item("discord-thread", "discord"))
        .await
        .unwrap();
    engine
        .store(channel_memory_item("telegram-thread", "telegram"))
        .await
        .unwrap();

    let outcome = disconnect_channel(&config, "discord", ChannelAuthMode::BotToken, true)
        .await
        .expect("disconnect clears memory");

    assert_eq!(outcome.value["memory_chunks_deleted"], 1);
    assert_eq!(engine.len(), 1, "the other channel memory remains");
}

#[tokio::test]
async fn disconnect_rpc_reports_zero_and_queues_when_memory_is_off() {
    let (_tmp, mut config) = isolated_test_config();
    use_linked_channel_module(&mut config).await;
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "discord-token-abc".into(),
        guild_id: Some("guild-1".into()),
        channel_id: Some("channel-2".into()),
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });
    config.memory.engine.clear();
    config.save().await.expect("save Discord setup");
    let tenant_ctx = account_context(&config, "offline-account");
    let thread_id =
        store_channel_thread(tenant_ctx.clone(), &config, "discord", "offline-message").await;

    let outcome = CoreContext::scope(tenant_ctx.into_core().unwrap(), async {
        disconnect_channel(&config, "discord", ChannelAuthMode::BotToken, true)
            .await
            .expect("offline memory queues deletion")
    })
    .await;

    assert_eq!(outcome.value["memory_chunks_deleted"], 0);
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Thread { thread_id }]
    );
}

#[tokio::test]
async fn disconnect_rpc_surfaces_memory_error_after_queuing_snapshot() {
    let (_tmp, mut config) = isolated_test_config();
    use_linked_channel_module(&mut config).await;
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "discord-token-abc".into(),
        guild_id: Some("guild-1".into()),
        channel_id: Some("channel-2".into()),
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });
    config.save().await.expect("save Discord setup");
    let thread_id =
        store_channel_thread(DispatchCtx::ambient(), &config, "discord", "failed-message").await;
    crate::memory::test_fixtures::RefusingEngine::with(tinymemory_api::Error::Engine(
        "forget failed".into(),
    ))
    .bind(&config);

    let error = disconnect_channel(&config, "discord", ChannelAuthMode::BotToken, true)
        .await
        .expect_err("disconnect must report the failed memory clear");

    assert!(error.contains("channel disconnected, but failed to clear memory chunks"));
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Thread { thread_id }]
    );
}
