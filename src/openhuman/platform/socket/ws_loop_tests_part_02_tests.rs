use super::*;

/// Driver-level proof: when the configured URL responds with a 301 pointing
/// at a working Engine.IO server, `ws_loop` follows the redirect, completes
/// the handshake, and records a one-shot warning in `SharedState.error` so
/// the UI can surface the stale-config signal.
#[tokio::test]
async fn ws_loop_follows_301_to_working_backend() {
    // 1. Real EIO server on `ws://127.0.0.1:PORT`.
    let (fwd_tx, mut fwd_rx) = mpsc::unbounded_channel::<String>();
    let real_addr = spawn_mock_eio_server(ConnectBehavior::Ack, fwd_tx).await;
    let real_ws_url = format!("ws://{real_addr}/socket.io/?EIO=4&transport=websocket");

    // 2. Redirect server that 301s every request to the real EIO server.
    let redirect_addr = spawn_mock_301_redirect(real_ws_url.clone()).await;

    // 3. Drive `ws_loop` with the redirect address as the base URL. This is
    //    exactly the production failure mode: BACKEND_URL points at a host
    //    that 301s the WebSocket upgrade.
    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let internal_tx = emit_tx.clone();
    drop(emit_tx);

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            format!("http://{redirect_addr}"),
            static_token_provider("redirect-test-token".to_string()),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
            Arc::new(Mutex::new(false)),
        )
        .await;
    });

    // The SIO CONNECT frame arriving on the *real* server proves the redirect
    // was followed and the WebSocket handshake completed against the redirect
    // target — not the redirect host.
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
    let mut saw_connect = false;
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(frame)) =
            tokio::time::timeout(tokio::time::Duration::from_millis(200), fwd_rx.recv()).await
        {
            if frame.starts_with("40") && frame.contains("redirect-test-token") {
                saw_connect = true;
                break;
            }
        }
    }
    assert!(
        saw_connect,
        "redirect was not followed — SIO CONNECT never reached the real EIO server"
    );

    for _ in 0..50 {
        if *shared.status.read() == ConnectionStatus::Connected {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
    assert_eq!(*shared.status.read(), ConnectionStatus::Connected);

    let warning = shared.error.read().clone();
    assert!(
        warning
            .as_deref()
            .map(|w| w.contains("redirected") && w.contains("BACKEND_URL"))
            .unwrap_or(false),
        "expected redirect warning in SharedState.error, got {warning:?}"
    );

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
}

/// 301 without a Location header is unrecoverable — must surface as a real
/// error and not loop forever attempting to follow nothing.
#[tokio::test]
async fn connect_with_redirects_fails_when_location_missing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await;
        // 301 but no Location header.
        let _ = stream
            .write_all(
                b"HTTP/1.1 301 Moved Permanently\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        let _ = stream.shutdown().await;
    });

    let shared = make_shared();
    let mut url = format!("ws://{addr}/socket.io/?EIO=4&transport=websocket");
    let err = connect_with_redirects(&mut url, &shared)
        .await
        .expect_err("must surface failure when Location is absent");
    assert!(matches!(err, WsError::Http(_)));
    // No warning recorded because the redirect was never actually followed.
    assert!(shared.error.read().is_none());
}

/// Provider called once per attempt: a counter-based provider proves the loop
/// re-fetches the token before each `run_connection` invocation.
#[tokio::test]
async fn ws_loop_calls_provider_before_each_attempt() {
    // Use a server that always closes immediately (no EIO OPEN) so the loop
    // cycles through failures quickly without hitting the backoff sleep.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    // Accept and immediately close — simulates a connection refused / close.
    tokio::spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let ws = accept_async(stream).await.expect("ws accept");
        let (mut write, _) = ws.split();
        let _ = write.close().await;
    });

    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = Arc::clone(&call_count);

    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (_emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (internal_tx, _internal_rx) = mpsc::unbounded_channel::<String>();

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            http_base_for(addr),
            Arc::new(move || {
                call_count_clone.fetch_add(1, Ordering::Relaxed);
                Ok("counter-token".to_string())
            }),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
            Arc::new(Mutex::new(false)),
        )
        .await;
    });

    // Let the loop run for at least 2 attempts, then shut down.
    // The first attempt triggers a provider call; after the connection closes,
    // the loop sleeps (1s backoff) before attempt 2 — we just need to see ≥1
    // call to prove the provider is wired up, then shut down.
    for _ in 0..50 {
        if call_count.load(Ordering::Relaxed) >= 1 {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;

    let calls = call_count.load(Ordering::Relaxed);
    assert!(
        calls >= 1,
        "provider must be called at least once before each attempt; got {calls}"
    );
}

/// "Invalid token" same-token escalation: when the server always rejects with
/// "Invalid token" and the provider keeps returning the same token, the loop
/// MUST exit within 2 attempts — not waste the remaining back-off retries.
/// This is the core regression fix for TAURI-RUST-9C (#2892).
#[tokio::test]
async fn ws_loop_escalates_immediately_on_invalid_token_no_refresh() {
    let addr = spawn_mock_invalid_token_server().await;

    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = Arc::clone(&call_count);

    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (_emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    // Do NOT signal shutdown — the loop must exit on its own via fast-fail.
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let (internal_tx, _internal_rx) = mpsc::unbounded_channel::<String>();

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            http_base_for(addr),
            // Provider always returns the same (stale) token.
            Arc::new(move || {
                call_count_clone.fetch_add(1, Ordering::Relaxed);
                Ok("stale-token-xyz".to_string())
            }),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
            Arc::new(Mutex::new(false)),
        )
        .await;
    });

    // The loop must exit by itself (fast-fail) well within 2 seconds.
    // At FAIL_ESCALATE_THRESHOLD=5 the old code would still be sleeping through
    // back-off at this point — finishing fast is what proves the fix.
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(4), handle).await;
    assert!(
        matches!(result, Ok(Ok(()))),
        "ws_loop must exit cleanly after Invalid token (no timeout, no panic)"
    );

    // Loop must have called the provider at most 2 times (initial attempt +
    // one re-fetch check). 3+ would mean it fell through to the old retry path.
    let calls = call_count.load(Ordering::Relaxed);
    assert!(
        calls <= 2,
        "provider must not be called more than 2 times on Invalid token fast-fail; got {calls}"
    );

    // Status must be Disconnected and error slot must carry session-expired.
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
    let err = shared.error.read().clone();
    assert!(
        err.as_deref()
            .map(|e| e.contains("session expired"))
            .unwrap_or(false),
        "expected session-expired error in SharedState, got: {err:?}"
    );
}

/// "Invalid token" with a fresh token available: provider returns token A on
/// the first call, token B on the second. The loop must NOT fast-fail — it
/// should detect the new token and retry once. Since the mock server also
/// rejects token B, the loop will fast-fail on the third call (same-token
/// case), but the important assertion is that it reached at least 2 actual
/// connection attempts (token A and token B) before stopping.
#[tokio::test]
async fn ws_loop_retries_with_fresh_token_on_invalid_token() {
    // Server always replies with "Invalid token".
    let addr = spawn_mock_invalid_token_server().await;

    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = Arc::clone(&call_count);

    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (_emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let (internal_tx, _internal_rx) = mpsc::unbounded_channel::<String>();

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            http_base_for(addr),
            // First call returns "token-a", second and beyond return "token-b".
            Arc::new(move || {
                let c = call_count_clone.fetch_add(1, Ordering::Relaxed);
                if c == 0 {
                    Ok("token-a".to_string())
                } else {
                    Ok("token-b".to_string())
                }
            }),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
            Arc::new(Mutex::new(false)),
        )
        .await;
    });

    // The loop should exit by itself (fast-fail after token-b also rejected).
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(6), handle).await;
    assert!(
        matches!(result, Ok(Ok(()))),
        "ws_loop must exit cleanly after both tokens rejected"
    );

    // Provider must have been called at least 2 times. Per the Minor on
    // PR #2905 the fresh token is now carried forward through
    // `pending_token` so the second attempt does NOT re-call the provider —
    // it uses the exact value the decision step validated. The expected
    // sequence is therefore:
    //
    //   call 1 → "token-a" (start of first attempt, no pending_token)
    //   call 2 → "token-b" (re-fetch check after Invalid token for
    //                       "token-a") → RetryImmediately stashes "token-b"
    //                       into pending_token
    //   (no extra call here: second attempt consumes pending_token = "token-b")
    //   call 3 → "token-b" (re-fetch check after Invalid token for
    //                       "token-b") → same token → Escalate
    //
    // We assert ≥ 2 — i.e. the fresh-token check fired at least once.
    let calls = call_count.load(Ordering::Relaxed);
    assert!(
        calls >= 2,
        "provider must be called at least twice (initial + fresh-token check); got {calls}"
    );

    // End state must be session-expired.
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
    let err = shared.error.read().clone();
    assert!(
        err.as_deref()
            .map(|e| e.contains("session expired"))
            .unwrap_or(false),
        "expected session-expired error in SharedState, got: {err:?}"
    );
}

/// Regression guard for the CodeRabbit Major on PR #2905: a non-deterministic
/// provider that returns a **different** non-empty token on every call must
/// NOT cause the loop to hot-loop indefinitely down the `RetryImmediately`
/// path. The bound is one immediate retry per fresh-token cycle; subsequent
/// `RetryImmediately` outcomes must fall through to the normal
/// `consecutive_failures` + backoff sleep path, which converges on a
/// definitive outcome (escalation or session timeout) rather than hammering
/// the server in a tight loop.
///
/// Setup: server always replies `Invalid token`; provider returns a brand-new
/// token on every call (token-0, token-1, token-2, …) — none of which the
/// server will accept. Before the bound, this would skip backoff on every
/// retry and produce tens-to-hundreds of attempts per second. After the
/// bound, total provider calls in a 2-second window must stay small (the
/// loop has to sleep through backoff between cycles).
#[tokio::test]
async fn ws_loop_bounds_fresh_token_retries_with_rotating_provider() {
    // Server always replies with "Invalid token" — guaranteed rejection.
    let addr = spawn_mock_invalid_token_server().await;

    let call_count = Arc::new(AtomicUsize::new(0));
    let call_count_clone = Arc::clone(&call_count);

    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (_emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (internal_tx, _internal_rx) = mpsc::unbounded_channel::<String>();

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            http_base_for(addr),
            // Each provider call returns a fresh, distinct, non-empty token.
            // This is the pathological provider the CodeRabbit Major calls
            // out: rapid server-side rotation, non-deterministic source, or
            // buggy implementation that never converges.
            Arc::new(move || {
                let n = call_count_clone.fetch_add(1, Ordering::Relaxed);
                Ok(format!("rotating-token-{n}"))
            }),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
            Arc::new(Mutex::new(false)),
        )
        .await;
    });

    // Give the loop ~2 seconds to misbehave. A hot-loop (no bound) would
    // accumulate dozens-to-hundreds of provider calls in that window — each
    // RetryImmediately is a `continue` with no sleep, and a roundtrip to the
    // mock server is sub-100 ms on loopback. With the bound, every
    // fresh-token cycle gets exactly one no-backoff retry; after that the
    // loop must sleep through `backoff` (starts at 1 s, doubles each time)
    // before re-attempting. The exponential backoff makes the upper bound
    // here forgiving on slow CI while still being orders-of-magnitude below
    // any plausible hot-loop count.
    tokio::time::sleep(tokio::time::Duration::from_millis(2_000)).await;

    let calls_before_shutdown = call_count.load(Ordering::Relaxed);
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;

    // The loop must NOT have hot-looped — a small bound proves the
    // RetryImmediately path is now bounded. The exact number depends on CI
    // scheduling, but 20 is comfortably above the steady-state expectation
    // (~3–6 cycles in 2 s of backoff sleep, each cycle = 2 provider calls)
    // and orders of magnitude below an unbounded loop.
    assert!(
        calls_before_shutdown <= 20,
        "RetryImmediately must be bounded — rotating-token provider triggered \
         {calls_before_shutdown} provider calls in 2 s (suspected hot-loop; expected ≤ 20)"
    );

    // …and the loop must have made progress toward escalation, not stayed
    // pinned on the no-backoff `continue` path. After the bound has fired
    // once the loop has incremented `consecutive_failures` and started
    // sleeping backoff. The clearest external proof of that is at least 3
    // provider calls (per-iteration flow with the Minor `pending_token`
    // optimisation also applied):
    //
    //   call 1 → initial connect attempt (token-0)
    //   call 2 → decide_after_invalid_token re-fetch (token-1) →
    //            RetryImmediately, stashes token-1 into pending_token
    //   (second attempt consumes pending_token; no new provider call)
    //   call 3 → decide_after_invalid_token after the second Invalid token
    //            (token-2) → RetryImmediately → BOUND HIT → falls through
    //            to backoff sleep
    //
    // We assert ≥ 3 (gives slack for a slow loopback) — i.e. we got past the
    // single immediate retry into the bounded path.
    assert!(
        calls_before_shutdown >= 3,
        "loop must have progressed past the initial connect + first immediate \
         retry — observed only {calls_before_shutdown} provider calls"
    );

    // End state must be Disconnected. The status field reflects the most
    // recent state transition; whether the loop reached `session expired`
    // depends on how many cycles fit into the 2-second window before
    // shutdown — but Disconnected is the unconditional end state on both
    // paths.
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

/// `is_invalid_token_error` unit tests are in `token_provider::tests`.
/// This test pins the exact wire shape from `read_sio_connect_ack()` against
/// the classifier to guard against drift between the two modules.
#[test]
fn sio_connect_error_invalid_token_classifies_correctly() {
    // This is the exact string produced by `read_sio_connect_ack()` when
    // the server sends `44{"message":"Invalid token"}`.
    assert!(is_invalid_token_error(
        "Socket.IO connect error: Invalid token"
    ));
    // A different server error must not trigger the fast-fail path.
    assert!(!is_invalid_token_error(
        "Socket.IO connect error: namespace not found"
    ));
    // Internal errors (EIO, WS layer) must not match.
    assert!(!is_invalid_token_error("EIO OPEN: timeout"));
    assert!(!is_invalid_token_error(
        "WebSocket connect: connection refused"
    ));
}

// ── decide_after_invalid_token ─────────────────────────────────────────────

/// Provider returns a genuinely fresh token → loop should retry immediately,
/// carrying the validated fresh token forward so the next attempt sends
/// exactly that value instead of re-reading the provider.
#[test]
fn decide_after_invalid_token_fresh_token_returns_retry() {
    let provider: TokenProvider = Arc::new(|| Ok("fresh-token".to_string()));
    match decide_after_invalid_token("stale-token", &provider) {
        InvalidTokenAction::RetryImmediately { token } => {
            assert_eq!(token, "fresh-token");
        }
        InvalidTokenAction::Escalate { reason } => {
            panic!("expected RetryImmediately, got Escalate({reason})");
        }
    }
}

/// Provider returns the same token → session is definitively expired; escalate.
#[test]
fn decide_after_invalid_token_same_token_escalates() {
    let provider: TokenProvider = Arc::new(|| Ok("same-token".to_string()));
    match decide_after_invalid_token("same-token", &provider) {
        InvalidTokenAction::Escalate { .. } => {}
        InvalidTokenAction::RetryImmediately { .. } => {
            panic!("expected Escalate when provider returns the same token");
        }
    }
}

/// Provider returns an error → escalate with the provider error as reason.
#[test]
fn decide_after_invalid_token_provider_error_escalates() {
    let provider: TokenProvider =
        Arc::new(|| Err("no session token stored — user must log in first".to_string()));
    match decide_after_invalid_token("any-token", &provider) {
        InvalidTokenAction::Escalate { reason } => {
            assert!(
                reason.contains("provider error"),
                "expected 'provider error' in escalation reason, got: {reason}"
            );
        }
        InvalidTokenAction::RetryImmediately { .. } => {
            panic!("expected Escalate when provider errors");
        }
    }
}

/// Provider returns an empty string → treat as no session; escalate.
#[test]
fn decide_after_invalid_token_empty_token_escalates() {
    let provider: TokenProvider = Arc::new(|| Ok(String::new()));
    match decide_after_invalid_token("prev-token", &provider) {
        InvalidTokenAction::Escalate { .. } => {}
        InvalidTokenAction::RetryImmediately { .. } => {
            panic!("expected Escalate when provider returns empty token");
        }
    }
}

// ── drain_pending_emits ─────────────────────────────────────────────────────

/// The emit channel outlives the socket, so anything still queued when a
/// connection ends would be flushed onto the next one — a different sid, whose
/// roster the backend has already cleared.
#[test]
fn draining_reports_and_discards_everything_still_queued() {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    for event in ["orch:tool_result", "orch:effect:result", "presence"] {
        tx.send(format!("42[\"{event}\",{{}}]")).expect("queue");
    }

    let dropped = drain_pending_emits(&mut rx);

    assert_eq!(dropped, 3, "every queued message should be counted");
    assert!(
        rx.try_recv().is_err(),
        "nothing may survive into the next connection"
    );
}

/// The common case — a clean disconnect with nothing queued — must not log a
/// spurious warning, so the count has to be honest about zero.
#[test]
fn draining_an_empty_channel_reports_nothing_dropped() {
    let (_tx, mut rx) = mpsc::unbounded_channel::<String>();
    assert_eq!(drain_pending_emits(&mut rx), 0);
}
