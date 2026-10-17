//! Busy state and delivery slots are per profile. In SaaS two profiles can use
//! the same thread id and the same web-chat session id (`{client_id,
//! thread_id}`); one profile's in-flight turn or delivery must neither hold
//! back nor stand in for the other's. Desktop keys stay the bare ids.

use super::*;
use crate::agent::orchestration::background_completions::{
    pending_for, record_completion, router_for_workspace, TestWorkspace,
};
use crate::core::runtime::DispatchCtx;
use crate::core::runtime::{ContextOverlay, DomainSet};
use crate::tools::toolpacks::ToolGroups;

fn profile(name: &str, workspace: &Path) -> Arc<CoreContext> {
    let config = crate::config::Config {
        workspace_dir: workspace.to_path_buf(),
        ..crate::config::Config::default()
    };
    CoreContext::for_test(DomainSet::full(), None).derive_with(
        ContextOverlay::new(config, DomainSet::kernel(), ToolGroups::none())
            .profile(name)
            .session_agent(name),
    )
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4().simple())
}

fn web_session(thread: &str) -> String {
    format!(r#"{{"client_id":"c","thread_id":"{thread}"}}"#)
}

/// Two profiles, each with one pending result on the same thread id.
struct Pair {
    alice: Arc<CoreContext>,
    bob: Arc<CoreContext>,
    ws_a: TestWorkspace,
    ws_b: TestWorkspace,
    thread: String,
    session: String,
}

async fn pair() -> Pair {
    let (ws_a, ws_b) = (TestWorkspace::new(), TestWorkspace::new());
    let alice = profile(&unique("u-alice"), ws_a.path());
    let bob = profile(&unique("u-bob"), ws_b.path());
    let thread = unique("t");
    let session = web_session(&thread);
    for (ctx, ws, task) in [(&alice, &ws_a, "task-alice"), (&bob, &ws_b, "task-bob")] {
        {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(ctx));
            record_completion(
                &scope_ctx,
                ws.path(),
                &session,
                task,
                "researcher",
                "done",
                Some(thread.clone()),
            )
            .await
        };
    }
    Pair {
        alice,
        bob,
        ws_a,
        ws_b,
        thread,
        session,
    }
}

fn in_scope<R>(ctx: &Arc<CoreContext>, f: impl FnOnce() -> R) -> R {
    CoreContext::sync_scope(Arc::clone(ctx), f)
}

#[tokio::test]
async fn one_profiles_busy_turn_does_not_block_anothers_thread() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let p = pair().await;
    let turn = in_scope(&p.alice, || {
        TurnBusy::start(&DispatchCtx::ambient(), &p.session)
    });

    assert!(in_scope(&p.alice, || is_busy(
        &DispatchCtx::ambient(),
        &p.thread
    )));
    assert!(!in_scope(&p.bob, || is_busy(
        &DispatchCtx::ambient(),
        &p.thread
    )));
    let router_b = router_for_workspace(p.ws_b.path());
    let batch = in_scope(&p.bob, || {
        claim_ready(&DispatchCtx::ambient(), &router_b, &p.thread)
    })
    .expect("bob's thread is idle while alice's turn runs");
    assert_eq!(batch[0].task_id, "task-bob");
    router_b.release(&["task-bob".to_string()]);

    // Bob's Stop clears only Bob's turns; Alice stays busy.
    assert_eq!(
        in_scope(&p.bob, || clear_busy_for_thread(
            &DispatchCtx::ambient(),
            &p.thread
        )),
        0
    );
    assert!(in_scope(&p.alice, || is_busy(
        &DispatchCtx::ambient(),
        &p.thread
    )));

    drop(turn);
    assert!(!in_scope(&p.alice, || is_busy(
        &DispatchCtx::ambient(),
        &p.thread
    )));
}

#[tokio::test]
async fn a_delivery_in_flight_for_one_profile_does_not_take_anothers_slot() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let p = pair().await;
    let (router_a, router_b) = (
        router_for_workspace(p.ws_a.path()),
        router_for_workspace(p.ws_b.path()),
    );

    let held = in_scope(&p.alice, || {
        DeliverySlot::claim(&DispatchCtx::ambient(), &p.thread, router_a.clone())
    })
    .expect("alice claims her slot");
    assert!(
        in_scope(&p.alice, || DeliverySlot::claim(
            &DispatchCtx::ambient(),
            &p.thread,
            router_a.clone()
        ))
        .is_none(),
        "one delivery per thread within a profile"
    );
    let bob_slot = in_scope(&p.bob, || {
        DeliverySlot::claim(&DispatchCtx::ambient(), &p.thread, router_b.clone())
    });
    assert!(
        bob_slot.is_some(),
        "alice's delivery does not hold bob's slot"
    );
    drop((held, bob_slot));

    // End to end: while Alice's delivery turn is in flight, Bob's runs and
    // lands in Bob's thread; then Alice's lands in hers.
    let delivered: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let log = |who: &'static str| {
        let delivered = Arc::clone(&delivered);
        move |thread: String, notice: String| {
            delivered
                .lock()
                .unwrap()
                .push((format!("{who}:{thread}"), notice));
            async { Ok::<_, String>("ok".to_string()) }
        }
    };
    let bob = Arc::clone(&p.bob);
    let (thread, router_bob, bob_log) = (p.thread.clone(), router_b.clone(), log("bob"));
    let alice_deliver = move |thread_a: String, notice: String| {
        let bob = Arc::clone(&bob);
        let (thread, router_bob, bob_log) = (thread.clone(), router_bob.clone(), bob_log.clone());
        let alice_log = log("alice");
        async move {
            let retry = {
                let scope_ctx = crate::core::runtime::DispatchCtx::new(bob);
                try_deliver_with(&scope_ctx, thread, router_bob, bob_log, |_t, _n| async {
                    unreachable!("bob's delivery succeeds")
                })
                .await
            };
            assert_eq!(retry, None);
            alice_log(thread_a, notice).await
        }
    };
    let retry = {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(Arc::clone(&p.alice));
        try_deliver_with(
            &scope_ctx,
            p.thread.clone(),
            router_a,
            alice_deliver,
            |_t, _n| async { unreachable!("alice's delivery succeeds") },
        )
        .await
    };
    assert_eq!(retry, None);

    let delivered = delivered.lock().unwrap().clone();
    assert_eq!(delivered.len(), 2, "both profiles got their result");
    assert_eq!(delivered[0].0, format!("bob:{}", p.thread));
    assert!(delivered[0].1.contains("task-bob") && !delivered[0].1.contains("task-alice"));
    assert_eq!(delivered[1].0, format!("alice:{}", p.thread));
    assert!(delivered[1].1.contains("task-alice") && !delivered[1].1.contains("task-bob"));
    assert!(pending_for(p.ws_a.path(), &p.thread).is_empty());
    assert!(pending_for(p.ws_b.path(), &p.thread).is_empty());
}

#[tokio::test]
async fn desktop_keys_are_the_bare_ids() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let desktop = CoreContext::for_test(DomainSet::full(), None);
    let ws = TestWorkspace::new();
    let thread = unique("t-desk");
    let session = unique("desk-session");
    in_scope(&desktop, || {
        background_completions::note_session_thread(&DispatchCtx::ambient(), &session, &thread)
    });

    let turn = in_scope(&desktop, || {
        TurnBusy::start(&DispatchCtx::ambient(), &session)
    });
    assert_eq!(turn.key, session);
    assert!(busy().lock().unwrap().contains_key(&session));
    assert!(in_scope(&desktop, || is_busy(
        &DispatchCtx::ambient(),
        &thread
    )));
    drop(turn);
    assert!(!busy().lock().unwrap().contains_key(&session));

    let router = router_for_workspace(ws.path());
    let slot = in_scope(&desktop, || {
        DeliverySlot::claim(&DispatchCtx::ambient(), &thread, router)
    })
    .expect("free");
    assert_eq!(slot.key, thread);
    assert!(delivering().lock().unwrap().contains(&thread));
    drop(slot);
    assert!(!delivering().lock().unwrap().contains(&thread));
}

#[tokio::test]
async fn an_older_guard_dropping_keeps_a_replacement_turn_busy() {
    let _guard = crate::config::TEST_ENV_LOCK.lock().await;
    let desktop = CoreContext::for_test(DomainSet::full(), None);
    let thread = unique("t-interrupt");
    let session = web_session(&thread);

    let old = in_scope(&desktop, || {
        TurnBusy::start(&DispatchCtx::ambient(), &session)
    });
    let new = in_scope(&desktop, || {
        TurnBusy::start(&DispatchCtx::ambient(), &session)
    });
    drop(old);
    assert!(
        in_scope(&desktop, || is_busy(&DispatchCtx::ambient(), &thread)),
        "replacement still running"
    );
    drop(new);
    assert!(!in_scope(&desktop, || is_busy(
        &DispatchCtx::ambient(),
        &thread
    )));
}
