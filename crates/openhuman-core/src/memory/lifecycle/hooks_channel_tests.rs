use super::*;

#[test]
fn channel_metadata_does_not_change_stable_ids_for_either_conversation_turn_role() {
    for role in [Role::User, Role::Assistant] {
        let mut tagged = StoreItem::Conversation {
            turns: vec![Turn::new(role, "same turn")],
            meta: MemoryMeta::default(),
        };
        let untagged = tagged.clone();
        tagged.meta_mut().channel = Some("telegram".into());

        assert_eq!(untagged.fingerprint(), tagged.fingerprint());
    }
}

#[tokio::test]
async fn a_channel_reply_is_stamped_with_channel_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let identity = tinymemory_tools::layout::identity::Identity::agent("channel-agent")
        .resolve(&crate::memory::scope::identity_pins(&config));

    post_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &identity,
        "channel-thread".into(),
        1,
        "reply from telegram".into(),
        Vec::new(),
        Utc::now(),
        Some("telegram".into()),
    )
    .await;

    let turns = stored(&engine, MetaFilter::kinds([ItemKind::Conversation])).await;
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].meta.channel.as_deref(), Some("telegram"));
    assert!(turns[0].text.contains("assistant: reply from telegram"));
}
