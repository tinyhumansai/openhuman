use super::*;
use crate::session::client::ClientHeaders;
use crate::session::test_support::ENV_LOCK;
use crate::session::test_support::{
    me_user, Backend, FakeCore, MeAnswer, StoredCredential, EXPIRED_JWT, LIVE_JWT, LIVE_JWT_NO_SUB,
    LOCAL_TOKEN, OPAQUE_TOKEN,
};
use std::sync::Arc;

#[tokio::test]
async fn bound_session_restores_only_at_issuing_backend_without_forwarding() {
    let _global = ENV_LOCK.lock().await;
    let issuer = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let other = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let core = FakeCore::new(&issuer.url);
    let first = manager(&core);
    first.store_session_token(&LIVE_JWT, None).await.unwrap();
    assert_eq!(
        core.session().unwrap().issuing_backend.as_deref(),
        Some(issuer.url.as_str())
    );
    let restored = manager(&core);
    restored.current_user(true).await.unwrap();
    assert_eq!(issuer.me_calls(), 2);
    *core.api_url.lock().unwrap() = other.url.clone();
    let error = restored.current_user(true).await.unwrap_err();
    assert!(
        matches!(error, SessionError::Backend(ref message) if message.contains("SESSION_BACKEND_MISMATCH"))
    );
    assert_eq!(
        other.me_calls(),
        0,
        "no Authorization may reach the changed backend"
    );
    assert!(other.consume_calls().is_empty());
    assert!(
        core.session().is_some(),
        "restore does not erase a session for another deployment"
    );
}

#[tokio::test]
async fn expected_backend_rejects_changed_configuration_before_any_credential_send() {
    let _global = ENV_LOCK.lock().await;
    let issuer = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let changed = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let core = FakeCore::new(&changed.url);
    let manager = manager(&core);
    for result in [
        manager
            .login_with_token_for_backend("synthetic-one-time", &issuer.url)
            .await,
        manager
            .store_session_token_for_backend(&LIVE_JWT, None, &issuer.url)
            .await,
    ] {
        assert!(
            matches!(result, Err(SessionError::Backend(ref message)) if message.starts_with("SESSION_BACKEND_MISMATCH"))
        );
    }
    assert_eq!(changed.me_calls(), 0);
    assert!(changed.consume_calls().is_empty());
    assert_eq!(issuer.me_calls(), 0);
    assert!(issuer.consume_calls().is_empty());
    assert!(core.session().is_none());
    *core.api_url.lock().unwrap() = issuer.url.clone();
    manager
        .store_session_token_for_backend(
            &LIVE_JWT,
            None,
            &format!("{}/ignored-path?discarded=nonsecret#fragment", issuer.url),
        )
        .await
        .unwrap();
    assert_eq!(
        issuer.me_calls(),
        1,
        "matching canonical origin accepts the callback"
    );
}

fn manager(core: &Arc<FakeCore>) -> Arc<SessionManager<FakeCore>> {
    SessionManager::new(Arc::clone(core), ClientHeaders::new("openhuman"))
}

#[tokio::test]
async fn exchange_and_validation_use_one_backend_when_configuration_changes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct SwitchingLink {
        core: Arc<FakeCore>,
        changed_url: String,
        reads: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl CoreLink for SwitchingLink {
        async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
            if method == link::CONFIG_RESOLVE_API_URL
                && self.reads.fetch_add(1, Ordering::SeqCst) > 0
            {
                return Ok(json!({"api_url":self.changed_url}));
            }
            self.core.invoke(method, params).await
        }
    }
    let _global = ENV_LOCK.lock().await;
    let issuer = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let other = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let core = FakeCore::new(&issuer.url);
    let manager = SessionManager::new(
        Arc::new(SwitchingLink {
            core: core.clone(),
            changed_url: other.url.clone(),
            reads: AtomicUsize::new(0),
        }),
        ClientHeaders::new("openhuman"),
    );
    assert!(matches!(
        manager.login_with_token("synthetic-one-time").await,
        Err(SessionError::Backend(_))
    ));
    assert_eq!(issuer.consume_calls().len(), 1);
    assert_eq!(
        issuer.me_calls(),
        1,
        "validation stays with the exchange client"
    );
    assert_eq!(
        other.me_calls(),
        0,
        "the new JWT never reaches the changed backend"
    );
    assert!(other.consume_calls().is_empty());
    assert_eq!(
        core.session().unwrap().issuing_backend.as_deref(),
        Some(issuer.url.as_str())
    );
}

async fn drain(rx: &mut tokio::sync::broadcast::Receiver<SessionEvent>) -> Vec<SessionEvent> {
    let mut out = Vec::new();
    while let Ok(event) = rx.try_recv() {
        out.push(event);
    }
    out
}

#[tokio::test]
async fn login_with_token_exchanges_validates_and_stores() {
    let _global = ENV_LOCK.lock().await;
    let backend = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    let mut rx = m.subscribe();

    let state = m.login_with_token("tok").await.unwrap();
    assert!(state.core.is_authenticated);
    assert_eq!(state.core.user_id.as_deref(), Some("user-123"));
    assert_eq!(
        state.current_user.as_ref().unwrap()["email"],
        "u@example.com"
    );
    assert!(!state.current_user_stale);

    let stored = core.session().unwrap();
    assert_eq!(stored.kind, "session");
    assert_eq!(stored.token, *LIVE_JWT);
    assert_eq!(stored.user_id.as_deref(), Some("user-123"));
    assert_eq!(stored.user.unwrap()["_id"], "user-123");
    assert_eq!(identity::peek_user_id().as_deref(), Some("user-123"));
    assert!(matches!(
        drain(&mut rx).await.as_slice(),
        [SessionEvent::Changed(_)]
    ));
}

#[tokio::test]
async fn rejected_jwt_is_never_stored() {
    let backend = Backend::start(vec![MeAnswer::Status(401)]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    assert!(matches!(
        m.store_session_token(&LIVE_JWT, None).await,
        Err(SessionError::Rejected(_))
    ));
    assert_eq!(core.session(), None);
    assert!(!core
        .methods()
        .iter()
        .any(|m| m == link::AUTH_SET_CREDENTIAL));
}

#[tokio::test]
async fn expired_jwt_is_refused_locally_without_a_request() {
    let backend = Backend::start(vec![]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    assert_eq!(
        m.store_session_token(&EXPIRED_JWT, None).await.unwrap_err(),
        SessionError::Expired
    );
    assert_eq!(backend.me_calls(), 0);
    assert_eq!(core.session(), None);
}

#[tokio::test]
async fn unreachable_backend_stores_a_pending_session_when_the_jwt_can_be_trusted() {
    let core = FakeCore::new("http://127.0.0.1:9");
    let m = manager(&core);
    let state = m.store_session_token(&LIVE_JWT, None).await.unwrap();
    assert!(state.core.is_authenticated);
    let stored = core.session().unwrap();
    assert_eq!(stored.user_id.as_deref(), Some("user-123"));
    assert_eq!(stored.user.unwrap()[PENDING_BACKEND_VALIDATION_FIELD], true);
    // The current user is served from the stored payload, flagged stale.
    assert!(state.current_user_stale);
    m.cancel_revalidation();
}

#[tokio::test]
async fn unreachable_backend_refuses_tokens_without_exp_or_subject() {
    let core = FakeCore::new("http://127.0.0.1:9");
    let m = manager(&core);
    assert!(matches!(
        m.store_session_token(OPAQUE_TOKEN, None).await,
        Err(SessionError::Transient(_))
    ));
    assert_eq!(
        m.store_session_token(&LIVE_JWT_NO_SUB, None)
            .await
            .unwrap_err(),
        SessionError::UserIdUnavailable
    );
    assert_eq!(core.session(), None);
}

#[tokio::test]
async fn rejected_replacement_keeps_pending_session_revalidation_running() {
    let core = FakeCore::new("http://127.0.0.1:9");
    let m = manager(&core);
    m.store_session_token(&LIVE_JWT, None).await.unwrap();
    assert!(m.revalidation.lock().unwrap().is_some());

    assert_eq!(
        m.store_session_token(&EXPIRED_JWT, None).await.unwrap_err(),
        SessionError::Expired
    );
    assert!(
        m.revalidation.lock().unwrap().is_some(),
        "a rejected replacement must not abandon the stored pending session"
    );
    m.cancel_revalidation();
}

// #6318 (review follow-up) — the successful-validation arm used to cancel
// the prior pending credential's revalidation loop *before* the fallible
// core handoff (`push`). If that handoff fails (or the core restarts
// mid-call), the prior credential must not be left provisional with no
// revalidation task running.
#[tokio::test]
async fn failed_core_handoff_keeps_the_prior_pending_session_revalidation_running() {
    let _global = ENV_LOCK.lock().await;

    // A is stored pending because the backend is unreachable; its own
    // background revalidation loop starts.
    let backend = Backend::start(vec![MeAnswer::Status(503)]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    m.store_session_token(&LIVE_JWT, None).await.unwrap();
    assert!(m.revalidation.lock().unwrap().is_some());

    // The backend becomes reachable and confirms the token via /auth/me
    // (the successful-validation arm), but the core handoff itself fails.
    *backend.state.me.lock().unwrap() = vec![MeAnswer::Ok(me_user())].into();
    *core.fail_method.lock().unwrap() = Some(link::AUTH_SET_CREDENTIAL.to_string());

    assert!(matches!(
        m.store_session_token(&LIVE_JWT, None).await,
        Err(SessionError::Core(_))
    ));

    assert!(
        m.revalidation.lock().unwrap().is_some(),
        "a failed core handoff must not cancel the prior pending session's revalidation loop"
    );
    m.cancel_revalidation();
}

// #6318 (review follow-up) — the background revalidation loop's confirmation
// arm used to exit on any outcome of a confirmed `/auth/me`, even when the
// subsequent core handoff (`push`) failed. That left `pendingBackendValidation`
// stuck forever despite a confirmed backend answer, with no loop left to
// retry it. Real time: the loop's first attempt fires after
// `REVALIDATION_INITIAL_DELAY` (5s), so this test waits past that.
#[tokio::test]
async fn a_failed_confirmation_handoff_keeps_the_revalidation_loop_retrying() {
    let _global = ENV_LOCK.lock().await;

    let backend = Backend::start(vec![MeAnswer::Status(503)]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    m.store_session_token(&LIVE_JWT, None).await.unwrap();
    assert!(m.revalidation.lock().unwrap().is_some());

    // The backend becomes reachable and will confirm the token, but the
    // core handoff itself fails.
    *backend.state.me.lock().unwrap() = vec![MeAnswer::Ok(me_user())].into();
    *core.fail_method.lock().unwrap() = Some(link::AUTH_SET_CREDENTIAL.to_string());

    // Let the loop's first attempt (after REVALIDATION_INITIAL_DELAY) run.
    tokio::time::sleep(REVALIDATION_INITIAL_DELAY + Duration::from_millis(500)).await;

    // `revalidation` only clears on an explicit `cancel_revalidation()`, so
    // `is_some()` alone would stay true even after the spawned task itself
    // returned; check `is_finished()` on the stored handle to know whether
    // the loop is still actually running.
    let still_running = m
        .revalidation
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|handle| !handle.is_finished());
    assert!(
        still_running,
        "a failed confirmation handoff must not exit the revalidation loop"
    );
    assert_eq!(
        core.session()
            .unwrap()
            .user
            .unwrap()
            .get(PENDING_BACKEND_VALIDATION_FIELD),
        Some(&serde_json::Value::Bool(true)),
        "the session must remain pending — the confirmed profile was never pushed"
    );
    m.cancel_revalidation();
}

#[tokio::test]
async fn caller_supplied_user_id_rescues_a_subjectless_jwt_offline() {
    let core = FakeCore::new("http://127.0.0.1:9");
    let m = manager(&core);
    let user = serde_json::json!({ "id": "from-caller" });
    m.store_session_token(&LIVE_JWT_NO_SUB, Some(user))
        .await
        .unwrap();
    assert_eq!(
        core.session().unwrap().user_id.as_deref(),
        Some("from-caller")
    );
    m.cancel_revalidation();
}

#[tokio::test]
async fn local_session_is_stored_without_touching_the_backend() {
    let backend = Backend::start(vec![]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    assert!(matches!(
        m.store_session_token(&LOCAL_TOKEN, None).await,
        Err(SessionError::Invalid(_))
    ));
    let state = m
        .store_session_token(&LOCAL_TOKEN, Some(serde_json::json!({ "name": "Me" })))
        .await
        .unwrap();
    assert_eq!(backend.me_calls(), 0);
    assert_eq!(core.session().unwrap().kind, "local");
    assert_eq!(state.current_user.unwrap()["name"], "Me");
}

#[tokio::test]
async fn api_key_store_and_clear() {
    let core = FakeCore::new("http://127.0.0.1:9");
    let m = manager(&core);
    let state = m.store_api_key(" sk-1 ").await.unwrap();
    assert_eq!(state.core.credential.as_deref(), Some("api-key"));
    assert_eq!(core.api_key.lock().unwrap().as_deref(), Some("sk-1"));
    let state = m.clear_api_key().await.unwrap();
    assert!(!state.core.is_authenticated);
    assert!(matches!(
        m.store_api_key("").await,
        Err(SessionError::Invalid(_))
    ));
}

#[tokio::test]
async fn logout_clears_the_session_and_identity() {
    let _global = ENV_LOCK.lock().await;
    let backend = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    m.login_with_token("tok").await.unwrap();
    let state = m.logout().await.unwrap();
    assert!(!state.core.is_authenticated);
    assert_eq!(core.session(), None);
    assert_eq!(identity::peek_user_id(), None);
    assert_eq!(m.cache().peek(), None);
    let (method, params) = core
        .calls()
        .into_iter()
        .find(|(m, _)| m == link::AUTH_CLEAR_CREDENTIAL)
        .unwrap();
    assert_eq!(method, link::AUTH_CLEAR_CREDENTIAL);
    assert_eq!(params["kind"], "session");
}

#[tokio::test]
async fn current_user_rejection_signs_out_and_emits_expired() {
    let backend = Backend::start(vec![MeAnswer::Ok(me_user()), MeAnswer::Status(401)]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    m.login_with_token("tok").await.unwrap();
    let mut rx = m.subscribe();
    assert!(matches!(
        m.current_user(true).await,
        Err(SessionError::Rejected(_))
    ));
    assert_eq!(core.session(), None);
    let events = drain(&mut rx).await;
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Expired { source } if source == "auth/me")));
    assert!(events
        .iter()
        .any(|e| matches!(e, SessionEvent::Changed(s) if !s.core.is_authenticated)));
    let state = m.state().await.unwrap();
    assert!(!state.core.is_authenticated);
}

/// Only a 401 expires the session: a 402/403/404 from `/auth/me` is a refusal
/// (plan, WAF, proxy) and the stored session survives it.
#[tokio::test]
async fn current_user_refusal_keeps_the_session() {
    for status in [402, 403, 404] {
        let backend = Backend::start(vec![MeAnswer::Ok(me_user()), MeAnswer::Status(status)]).await;
        let core = FakeCore::new(&backend.url);
        let m = manager(&core);
        m.login_with_token("tok").await.unwrap();
        let mut rx = m.subscribe();
        let cached = m.current_user(true).await;
        assert!(
            matches!(&cached, Ok(c) if c.stale),
            "{status}: expected the stored user served stale, got {cached:?}"
        );
        assert!(core.session().is_some(), "{status} cleared the session");
        assert!(
            !drain(&mut rx)
                .await
                .iter()
                .any(|e| matches!(e, SessionEvent::Expired { .. })),
            "{status} emitted Expired"
        );
    }
}

/// Linking a second provider to the same account is a session refresh, not a
/// different user (matrix 1.2.2).
///
/// Nothing the client can see says *which* provider logged you in: `/auth/me`
/// returns no provider field and this crate models none, so "signing in with
/// GitHub when you already signed in with Google" arrives here as nothing more
/// than a second login token that redeems to a **different JWT for the same
/// backend user id**. That is the whole of multi-provider linking as far as the
/// desktop app is concerned, and it is the only part of matrix 1.2.2 that is
/// testable client-side.
///
/// It matters because `store_session_token` signs the previous user out when
/// the user id changes. If a same-user re-login were ever treated as a user
/// change, linking a second provider would wipe the first provider's session
/// state on every login.
#[tokio::test]
async fn relinking_the_same_user_with_a_new_token_refreshes_rather_than_switches_user() {
    let _global = ENV_LOCK.lock().await;
    let backend = Backend::start(vec![MeAnswer::Ok(me_user()), MeAnswer::Ok(me_user())]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);

    m.login_with_token("via-google").await.unwrap();
    assert_eq!(core.session().unwrap().token, *LIVE_JWT);
    assert_eq!(identity::peek_user_id().as_deref(), Some("user-123"));

    // The second provider's login redeems to a different JWT. Same claims —
    // same `sub`, same `exp` — so the same backend user; only the signature
    // differs, exactly as a freshly issued session would.
    let second_jwt = format!("{}x", *LIVE_JWT);
    *backend.state.consume_jwt.lock().unwrap() = Some(second_jwt.clone());

    let mut rx = m.subscribe();
    let state = m.login_with_token("via-github").await.unwrap();

    assert!(state.core.is_authenticated);
    assert_eq!(
        state.core.user_id.as_deref(),
        Some("user-123"),
        "a second provider for the same account must not change the user id"
    );
    assert_eq!(
        core.session().unwrap().token,
        second_jwt,
        "the newly issued session token should have replaced the old one"
    );
    assert_eq!(
        identity::peek_user_id().as_deref(),
        Some("user-123"),
        "the identity slot must survive a same-user re-login"
    );

    // No sign-out anywhere in the transition. `SessionEvent::Expired` or a
    // `Changed` carrying `!is_authenticated` would mean the app bounced the
    // user to Welcome midway through linking a provider.
    let events = drain(&mut rx).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::Expired { .. })),
        "same-user re-login emitted an Expired event: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::Changed(s) if !s.core.is_authenticated)),
        "same-user re-login emitted a signed-out state: {events:?}"
    );
}

/// Server-side revocation must clear the identity slot and the user cache, not
/// just the core credential (matrix 1.4.3).
///
/// `current_user_rejection_signs_out_and_emits_expired` above covers the
/// credential and the events; it does not look at `identity::peek_user_id()`
/// or `cache().peek()`. `logout_clears_the_session_and_identity` checks both,
/// but only for an *explicit* logout — so a revocation path that forgot either
/// one would pass the whole suite.
///
/// Both matter, for different reasons:
///   * the identity slot is the process-global `Sentry before_send` reads
///     (`session/identity.rs` — *"Only the id is kept"*), so a stale id there
///     attributes every later event to a user who was signed out, silently and
///     for the life of the process.
///   * a surviving `CurrentUserCache` entry lets `current_user(false)` serve
///     the revoked user's profile from memory after the backend already
///     refused it.
#[tokio::test]
async fn current_user_rejection_also_clears_identity_and_cache() {
    let backend = Backend::start(vec![MeAnswer::Ok(me_user()), MeAnswer::Status(401)]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    m.login_with_token("tok").await.unwrap();

    // Precondition, asserted rather than assumed: a rejection test that starts
    // from an already-empty identity slot proves nothing about clearing it.
    assert!(
        identity::peek_user_id().is_some(),
        "login should have populated the identity slot; without that this test \
         cannot show the rejection cleared it"
    );
    assert!(
        m.cache().peek().is_some(),
        "login should have populated the user cache; without that this test \
         cannot show the rejection cleared it"
    );

    assert!(matches!(
        m.current_user(true).await,
        Err(SessionError::Rejected(_))
    ));

    assert_eq!(
        identity::peek_user_id(),
        None,
        "a revoked session left its user id in the process-global identity \
         slot; Sentry would keep attributing events to a signed-out user"
    );
    assert_eq!(
        m.cache().peek(),
        None,
        "a revoked session left its profile in the current-user cache; a later \
         current_user(false) would serve it from memory"
    );
}

#[tokio::test]
async fn clearing_a_rejected_session_reports_a_surviving_api_key() {
    let backend = Backend::start(vec![]).await;
    let core = FakeCore::new(&backend.url);
    *core.session.lock().unwrap() = Some(StoredCredential {
        kind: "session".to_string(),
        token: LIVE_JWT.clone(),
        user_id: Some("user-123".to_string()),
        user: Some(me_user()),
        issuing_backend: None,
    });
    *core.api_key.lock().unwrap() = Some("sk-fallback".to_string());
    let m = manager(&core);
    let mut rx = m.subscribe();

    assert!(m.clear_session_credential("test").await);
    let events = drain(&mut rx).await;
    assert!(events.iter().any(|event| matches!(
        event,
        SessionEvent::Changed(state) if state.core.credential.as_deref() == Some("api-key")
    )));
}

#[tokio::test]
async fn current_user_serves_the_stored_user_stale_while_the_backend_is_down() {
    let backend = Backend::start(vec![MeAnswer::Ok(me_user()), MeAnswer::Status(503)]).await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    m.login_with_token("tok").await.unwrap();
    m.cache().forget();
    let current = m.current_user(true).await.unwrap();
    assert!(current.stale);
    assert_eq!(current.user.unwrap()["_id"], "user-123");
    assert!(
        core.session().is_some(),
        "an outage must not sign the user out"
    );
}

#[tokio::test]
async fn current_user_confirms_a_pending_session_once_the_backend_answers() {
    let backend = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let core = FakeCore::new(&backend.url);
    *core.session.lock().unwrap() = Some(crate::session::test_support::StoredCredential {
        kind: "session".into(),
        token: LIVE_JWT.clone(),
        user_id: Some("user-123".into()),
        user: Some(serde_json::json!({ PENDING_BACKEND_VALIDATION_FIELD: true })),
        issuing_backend: None,
    });
    let m = manager(&core);
    let current = m.current_user(false).await.unwrap();
    assert_eq!(current.user.as_ref().unwrap()["email"], "u@example.com");
    let stored = core.session().unwrap();
    assert!(stored
        .user
        .unwrap()
        .get(PENDING_BACKEND_VALIDATION_FIELD)
        .is_none());
}

#[tokio::test]
async fn rejection_of_a_superseded_token_leaves_the_new_session_alone() {
    // Token A's refresh is in flight when the user signs in as B; A's 401
    // must not clear B.
    let backend = Backend::start(vec![
        MeAnswer::Ok(me_user()),
        MeAnswer::SlowStatus(300, 401),
        MeAnswer::Ok(me_user()),
    ])
    .await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    m.login_with_token("tok").await.unwrap();
    let mut rx = m.subscribe();
    let refresh = {
        let m = Arc::clone(&m);
        tokio::spawn(async move { m.current_user(true).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let replacement = crate::session::test_support::StoredCredential {
        kind: "session".into(),
        token: crate::session::test_support::LIVE_JWT_NO_SUB.clone(),
        user_id: Some("user-456".into()),
        user: Some(serde_json::json!({ "_id": "user-456" })),
        issuing_backend: None,
    };
    *core.session.lock().unwrap() = Some(replacement.clone());
    let current = refresh.await.unwrap().unwrap();
    assert!(current.stale, "a superseded verdict is served as stale");
    assert_eq!(core.session(), Some(replacement));
    assert!(!core
        .methods()
        .iter()
        .any(|m| m == link::AUTH_CLEAR_CREDENTIAL));
    assert!(drain(&mut rx)
        .await
        .iter()
        .all(|e| !matches!(e, SessionEvent::Expired { .. })));
}

#[tokio::test]
async fn state_retries_its_core_snapshot_after_a_superseded_refresh() {
    let backend = Backend::start(vec![
        MeAnswer::Ok(me_user()),
        MeAnswer::Slow(300),
        MeAnswer::Ok(serde_json::json!({ "_id": "user-456", "email": "b@example.com" })),
    ])
    .await;
    let core = FakeCore::new(&backend.url);
    let m = manager(&core);
    m.login_with_token("tok").await.unwrap();
    m.cache().forget();

    let reading = {
        let m = Arc::clone(&m);
        tokio::spawn(async move { m.state().await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    m.store_session_token(
        &LIVE_JWT_NO_SUB,
        Some(serde_json::json!({ "id": "user-456" })),
    )
    .await
    .unwrap();

    let state = reading.await.unwrap().unwrap();
    assert_eq!(state.core.user_id.as_deref(), Some("user-456"));
    assert_eq!(state.current_user.as_ref().unwrap()["_id"], "user-456");
}

#[tokio::test]
async fn pending_confirmation_after_logout_does_not_restore_the_session() {
    let backend = Backend::start(vec![MeAnswer::Slow(300)]).await;
    let core = FakeCore::new(&backend.url);
    *core.session.lock().unwrap() = Some(crate::session::test_support::StoredCredential {
        kind: "session".into(),
        token: LIVE_JWT.clone(),
        user_id: Some("user-123".into()),
        user: Some(serde_json::json!({ PENDING_BACKEND_VALIDATION_FIELD: true })),
        issuing_backend: None,
    });
    let m = manager(&core);
    let refresh = {
        let m = Arc::clone(&m);
        tokio::spawn(async move { m.current_user(true).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    m.logout().await.unwrap();
    let calls_after_logout = core.calls().len();
    refresh.await.unwrap().unwrap();
    assert_eq!(
        core.session(),
        None,
        "the confirmation must not reinstall the token"
    );
    assert!(!core.calls()[calls_after_logout..]
        .iter()
        .any(|(m, _)| m == link::AUTH_SET_CREDENTIAL));
}

#[tokio::test]
async fn state_for_a_signed_out_core_needs_no_backend() {
    let core = FakeCore::new("http://127.0.0.1:9");
    let m = manager(&core);
    let state = m.state().await.unwrap();
    assert!(!state.core.is_authenticated);
    assert_eq!(state.current_user, None);
    assert_eq!(state.core.kind(), None);
}

#[tokio::test]
async fn client_is_rebuilt_when_the_backend_url_changes() {
    let core = FakeCore::new("http://a.example");
    let m = manager(&core);
    let first = m.client().await.unwrap();
    assert_eq!(first.base_url(), "http://a.example");
    assert!(Arc::ptr_eq(&first, &m.client().await.unwrap()));
    *core.api_url.lock().unwrap() = "http://b.example/".to_string();
    let second = m.client().await.unwrap();
    assert_eq!(second.base_url(), "http://b.example");
    assert!(!Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn core_failures_surface_as_core_errors() {
    let core = FakeCore::new("http://127.0.0.1:9");
    *core.fail_with.lock().unwrap() = Some("boom".to_string());
    let m = manager(&core);
    assert!(matches!(m.state().await, Err(SessionError::Core(_))));
    assert!(matches!(m.logout().await, Err(SessionError::Core(_))));
    assert!(matches!(
        m.store_session_token(&LIVE_JWT, None).await,
        Err(SessionError::Backend(_))
    ));
}

#[test]
fn session_error_display_carries_stable_prefixes() {
    assert!(SessionError::Rejected("x".into())
        .to_string()
        .starts_with("REJECTED:"));
    assert!(SessionError::Expired.to_string().starts_with("EXPIRED:"));
    assert!(SessionError::Transient("x".into())
        .to_string()
        .starts_with("TRANSIENT:"));
    assert!(SessionError::UserIdUnavailable
        .to_string()
        .starts_with("USER_ID_UNAVAILABLE:"));
    assert!(SessionError::ConsumeFailed("x".into())
        .to_string()
        .starts_with("CONSUME_FAILED:"));
    assert!(SessionError::Core("x".into())
        .to_string()
        .starts_with("CORE:"));
}
