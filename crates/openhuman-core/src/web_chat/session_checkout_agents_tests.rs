//! Per-agent session cache slots for embedded agents that share thread ids.

use std::sync::Arc;

use super::session_checkout_tests::{host_seeded_with, test_config, unique_thread};
use crate::config::Config;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use crate::web_chat::ops::{key_for, thread_sessions};

/// Thread ids are chosen by callers, so two embedded agents can pick the same
/// one. Each must get its own cache slot, or the second agent's turn would
/// check out the first agent's live session and its history.
#[tokio::test]
async fn two_agents_with_the_same_thread_id_get_their_own_cache_slots() {
    let thread_id = unique_thread("agents");
    assert_eq!(
        key_for(&crate::core::runtime::DispatchCtx::ambient(), &thread_id),
        thread_id,
        "no agent scope: bare id"
    );

    let parent = CoreContext::for_test(DomainSet::full(), None);
    let agent_ctx = |agent: &str| {
        parent.derive_with(
            ContextOverlay::new(Config::default(), DomainSet::full(), Default::default())
                .session_agent(agent),
        )
    };
    let asha = agent_ctx("asha");
    let ravi = agent_ctx("ravi");
    let key_a = key_for(
        &crate::core::runtime::DispatchCtx::new(asha.clone()),
        &thread_id,
    );
    let key_b = key_for(
        &crate::core::runtime::DispatchCtx::new(ravi.clone()),
        &thread_id,
    );
    assert_ne!(key_a, key_b);
    assert_ne!(key_a, thread_id);
    // Delimiter-looking input cannot forge another scope's key.
    use crate::web_chat::ops::scoped_key;
    assert_ne!(scoped_key(Some("a"), "b::c"), scoped_key(Some("a::b"), "c"));
    assert_ne!(scoped_key(None, "a::b"), scoped_key(Some("a"), "b"));
    assert_ne!(scoped_key(None, "\u{1f}1:ab"), scoped_key(Some("a"), "b"));
    assert_ne!(scoped_key(Some("a"), "bc"), scoped_key(Some("ab"), "c"));

    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(&tmp);
    let fingerprint =
        super::build_session_fingerprint(&config, None, None, "orchestrator".into(), "chat");
    for (ctx, key) in [(&asha, &key_a), (&ravi, &key_b)] {
        let entry = crate::web_chat::types::SessionEntry {
            agent: host_seeded_with(&config, "x"),
            fingerprint: fingerprint.clone(),
        };
        {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(ctx));

            thread_sessions(&scope_ctx)
                .lock_owned()
                .await
                .insert(key.clone(), entry);
        };
    }
    let cached = |ctx: &Arc<CoreContext>, key: &String| {
        let (ctx, key) = (Arc::clone(ctx), key.clone());
        async move {
            {
                let scope_ctx = crate::core::runtime::DispatchCtx::new(ctx);

                thread_sessions(&scope_ctx)
                    .lock_owned()
                    .await
                    .contains_key(&key)
            }
        }
    };
    // Under one agent's scope only that agent's slot goes.
    CoreContext::scope(asha.clone(), async {
        let ctx = crate::core::runtime::DispatchCtx::new(asha.clone());
        crate::web_chat::ops::invalidate_thread_sessions(&ctx, &thread_id).await;
    })
    .await;
    assert!(!cached(&asha, &key_a).await, "asha's slot is evicted");
    assert!(cached(&ravi, &key_b).await, "ravi's slot survives");
    CoreContext::scope(ravi.clone(), async {
        let ctx = crate::core::runtime::DispatchCtx::new(ravi.clone());
        crate::web_chat::ops::invalidate_thread_sessions(&ctx, &thread_id).await;
    })
    .await;
    assert!(!cached(&ravi, &key_b).await, "ravi's slot is evicted");
}
