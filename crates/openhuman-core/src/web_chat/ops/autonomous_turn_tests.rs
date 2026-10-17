use super::*;
use crate::core::runtime::{context::CoreContext, DomainSet};
use crate::web_chat::{RUN_CHAT_TASK_TEST_LOCK, TestRunChatTaskBlock, set_test_run_chat_task_block};
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn admitted_background_turn_is_interruptible_and_stays_stopped_after_reload() {
    let _serial=RUN_CHAT_TASK_TEST_LOCK.lock().await; let dir=tempfile::tempdir().unwrap();
    let mut config=crate::config::Config::default();config.workspace_dir=dir.path().into();
    let context=CoreContext::for_test_with_config(DomainSet::full(),config);
    CoreContext::scope(context, async { let thread="autonomous-cancel-integration";
        let work=store::start(dir.path(),thread,"Implement feature".into(),vec!["Pass regression".into()],None).await.unwrap();
        let block=TestRunChatTaskBlock { started:Arc::new(AtomicBool::new(false)), dropped:Arc::new(AtomicBool::new(false)),release:Arc::new(tokio::sync::Notify::new()), succeed_in:None };
        set_test_run_chat_task_block(Some(block.clone())).await;
        start_autonomous_turn(dir.path().into(),work.clone()).await.unwrap(); tokio::time::timeout(std::time::Duration::from_secs(5),async {while !block.started.load(Ordering::SeqCst) {tokio::task::yield_now().await;} }).await.unwrap();
        let ctx = DispatchCtx::ambient();
        let request=in_flight(&ctx).lock().await.get(&key_for(&ctx, thread)).unwrap().request_id.clone();
        // Duplicate admission cannot replace the live turn.
        start_autonomous_turn(dir.path().into(),work.clone()).await.unwrap(); assert_eq!(in_flight(&ctx).lock().await.get(&key_for(&ctx, thread)).unwrap().request_id,request);
        crate::web_chat::cancel_chat_scoped(&ctx, "test", thread, Some("old-request")).await.unwrap();
        assert!(store::get(dir.path(),thread).await.unwrap().unwrap().runnable());
        crate::web_chat::cancel_chat_scoped(&ctx, "test", thread, None).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5),async {while !block.dropped.load(Ordering::SeqCst) {tokio::task::yield_now().await;} }).await.unwrap();
        let reloaded=store::get(dir.path(),thread).await.unwrap().unwrap();
        assert_eq!(reloaded.phase,crate::agent::autonomous::types::Phase::Paused);
        start_autonomous_turn(dir.path().into(),work).await.unwrap();
        assert!(!in_flight(&ctx).lock().await.contains_key(&key_for(&ctx, thread)));
        set_test_run_chat_task_block(None).await;
    }).await;
}
