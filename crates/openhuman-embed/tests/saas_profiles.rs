//! `ProfileRuntime`: two users' profiles in one SaaS process, driven
//! in-process without the gateway. Each user's `t1` is their own thread, a
//! handle keeps its profile from being released or evicted, a relayed
//! platform message lands on the user's `channel:` thread with its reply on
//! that user's events only, and the process stays locked to SaaS.
//!
//! One `#[test]`: a SaaS boot locks the whole process.

mod common;

use std::time::Duration;

use common::{
    chat_requests, echo_inference, last_user_message, runtime_with_keyring, PointedTransport,
};
use openhuman_embed::profiles::ProfileCredentialKind;
use openhuman_embed::{
    OpenError, ProfileError, ProfileRuntime, RelayMessage, Runtime, SaasConfig, Workspace,
};

const TURN: Duration = Duration::from_secs(120);

#[test]
fn profiles_are_isolated_held_and_relayed() {
    let _ = env_logger::builder().is_test(true).try_init();
    // On a worker thread: a turn dispatched from the test thread itself would
    // overflow its default stack.
    let root = tempfile::tempdir().expect("root");
    let rt = runtime_with_keyring(&root.path().join("operator/workspace"));
    rt.block_on(async { tokio::spawn(scenario(root)).await })
        .expect("scenario");
}

async fn scenario(root: tempfile::TempDir) {
    let inference = echo_inference().await;
    PointedTransport::install(&inference.uri());

    let mut config = SaasConfig::new(root.path());
    config.max_profiles_open = 4;
    let profiles = ProfileRuntime::build(config).await.expect("boot");
    assert!(root.path().join("service.token").exists(), "token written");

    // ── provisioning ────────────────────────────────────────────────────
    let alice = profiles.provision("alice").await.expect("alice");
    assert!(alice.created);
    assert!(!profiles.provision("alice").await.unwrap().created);
    let bob = profiles.provision("bob").await.expect("bob");
    let ids: Vec<String> = profiles
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.profile_id.to_string())
        .collect();
    assert!(ids.contains(&"alice".to_string()) && ids.contains(&"bob".to_string()));
    match profiles.open("carol").await {
        Err(ProfileError::Open(OpenError::NotProvisioned(id))) => assert_eq!(id.as_str(), "carol"),
        other => panic!("an unprovisioned profile must not open: {other:?}"),
    }
    for (id, key) in [
        (&alice.profile_id, "alice-key"),
        (&bob.profile_id, "bob-key"),
    ] {
        profiles
            .set_credential(id, ProfileCredentialKind::ApiKey, key)
            .await
            .expect("credential");
    }

    // ── two users, one thread id ────────────────────────────────────────
    let alice_h = profiles.open("alice").await.expect("open alice");
    let bob_h = profiles.open("bob").await.expect("open bob");
    assert_ne!(alice_h.workspace_dir(), bob_h.workspace_dir());

    let a = tokio::time::timeout(TURN, alice_h.chat("t1", "alice's secret plan"))
        .await
        .expect("alice's turn finishes")
        .expect("alice's turn");
    let b = tokio::time::timeout(TURN, bob_h.chat("t1", "bob's grocery list"))
        .await
        .expect("bob's turn finishes")
        .expect("bob's turn");
    assert!(a.text.contains("alice's secret plan"), "{a:?}");
    assert!(!a.text.contains("bob"), "{a:?}");
    assert!(b.text.contains("bob's grocery list"), "{b:?}");
    assert!(!b.text.contains("alice"), "{b:?}");

    // Each user's credential carried their own turn.
    let auths: Vec<String> = chat_requests(&inference)
        .await
        .iter()
        .map(|r| {
            let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap_or_default();
            let auth = r
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            format!("{auth} {}", last_user_message(&body))
        })
        .collect();
    assert!(
        auths
            .iter()
            .any(|l| l.contains("alice-key") && l.contains("alice's secret plan")),
        "{auths:?}"
    );
    assert!(
        !auths
            .iter()
            .any(|l| l.contains("alice-key") && l.contains("bob's grocery list")),
        "{auths:?}"
    );
    assert!(
        auths
            .iter()
            .any(|l| l.contains("bob-key") && l.contains("bob's grocery list")),
        "{auths:?}"
    );

    let alice_t1 = joined(alice_h.messages("t1").await.unwrap());
    let bob_t1 = joined(bob_h.messages("t1").await.unwrap());
    assert!(alice_t1.contains("alice's secret plan"), "{alice_t1}");
    assert!(!alice_t1.contains("bob's grocery list"), "{alice_t1}");
    assert!(bob_t1.contains("bob's grocery list"), "{bob_t1}");
    assert!(!bob_t1.contains("alice's secret plan"), "{bob_t1}");

    // A finished turn leaves nothing running on the profile (its progress
    // bridge and detached work end), so only the handle keeps it in use.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while alice_h.context().tenant_in_use() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "alice's finished turn still pins her profile"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── a handle holds its profile ──────────────────────────────────────
    let held = profiles.release(&alice.profile_id).await;
    assert!(
        matches!(held, Err(ProfileError::Host(ref why)) if why.contains("in use")),
        "a held profile is not released: {held:?}"
    );
    drop(alice_h);
    // The turn's own detached work (title, memory) may still be finishing.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match profiles.release(&alice.profile_id).await {
            Ok(released) => {
                assert!(released, "alice was open here");
                break;
            }
            Err(e) if tokio::time::Instant::now() < deadline => {
                log::debug!("alice still busy: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) => panic!("alice never went idle: {e}"),
        }
    }
    assert!(!profiles
        .list()
        .await
        .unwrap()
        .iter()
        .any(|s| s.open && s.profile_id == alice.profile_id));
    // Reopening picks the same thread back up.
    let alice_h = profiles.open("alice").await.expect("reopen alice");
    assert!(alice_h
        .threads()
        .await
        .unwrap()
        .iter()
        .any(|t| t.id == "t1"));

    // ── a relayed platform message ──────────────────────────────────────
    let mut bob_events = bob_h.events();
    let mut alice_events = alice_h.events();
    let relayed = bob_h
        .relay_inbound(RelayMessage::new(
            "telegram",
            "777",
            "555",
            "tg-1",
            "hello from telegram",
        ))
        .await
        .expect("relay");
    assert!(relayed.accepted && !relayed.duplicate, "{relayed:?}");
    assert_eq!(relayed.thread_id, "channel:telegram/555/777");
    let outbound = tokio::time::timeout(TURN, async {
        loop {
            let event = bob_events.recv().await.expect("bob's events");
            if event.event == "channel_outbound" {
                return event;
            }
        }
    })
    .await
    .expect("bob's relayed reply");
    assert_eq!(outbound.thread_id, relayed.thread_id);
    assert!(
        outbound
            .full_response
            .as_deref()
            .is_some_and(|t| t.contains("hello from telegram")),
        "{outbound:?}"
    );
    let again = bob_h
        .relay_inbound(RelayMessage::new(
            "telegram",
            "777",
            "555",
            "tg-1",
            "hello from telegram",
        ))
        .await
        .unwrap();
    assert!(
        again.duplicate,
        "a retried delivery runs nothing: {again:?}"
    );
    assert!(bob_h
        .threads()
        .await
        .unwrap()
        .iter()
        .any(|t| t.id == relayed.thread_id));
    assert!(!alice_h
        .threads()
        .await
        .unwrap()
        .iter()
        .any(|t| t.id == relayed.thread_id));
    // Nothing of bob's relay reached alice's stream.
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(50), alice_events.recv()).await
    {
        assert_ne!(event.thread_id, relayed.thread_id, "leaked: {event:?}");
    }

    // ── the process is locked to SaaS ───────────────────────────────────
    let second = ProfileRuntime::build(SaasConfig::new(root.path())).await;
    assert!(matches!(second, Err(ProfileError::Boot(_))), "{second:?}");
    let desktop = Runtime::builder()
        .workspace(Workspace::Ephemeral)
        .build()
        .await;
    assert!(desktop.is_err(), "no single-user runtime beside SaaS");

    drop((alice_h, bob_h));
    profiles.shutdown().await;
}

fn joined(messages: Vec<openhuman_embed::profiles::ConversationMessageRecord>) -> String {
    messages
        .into_iter()
        .map(|m| m.content)
        .collect::<Vec<_>>()
        .join("\n")
}
