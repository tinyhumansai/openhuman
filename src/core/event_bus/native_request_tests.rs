use super::*;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn register_and_dispatch_owned_payload() {
    let registry = NativeRegistry::new();
    registry.register::<String, usize, _, _>("echo.len", |s| async move { Ok(s.len()) });

    let n: usize = registry
        .request("echo.len", "hello".to_string())
        .await
        .expect("dispatch should succeed");
    assert_eq!(n, 5);
}

#[tokio::test]
async fn dispatches_trait_object_payload() {
    // The whole point of native_request: pass trait objects without
    // serialization.
    trait Greeter: Send + Sync {
        fn greet(&self, name: &str) -> String;
    }
    struct EnglishGreeter;
    impl Greeter for EnglishGreeter {
        fn greet(&self, name: &str) -> String {
            format!("Hello, {name}!")
        }
    }

    struct Req {
        greeter: Arc<dyn Greeter>,
        name: String,
    }
    struct Resp(String);

    let registry = NativeRegistry::new();
    registry.register::<Req, Resp, _, _>("greeter.greet", |req| async move {
        Ok(Resp(req.greeter.greet(&req.name)))
    });

    let resp: Resp = registry
        .request(
            "greeter.greet",
            Req {
                greeter: Arc::new(EnglishGreeter),
                name: "world".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(resp.0, "Hello, world!");
}

#[tokio::test]
async fn dispatches_mpsc_sender_payload() {
    // Streaming deltas: caller passes a sender, handler writes to it.
    struct Req {
        delta_tx: mpsc::Sender<String>,
        prompt: String,
    }
    struct Resp {
        final_text: String,
    }

    let registry = NativeRegistry::new();
    registry.register::<Req, Resp, _, _>("llm.stream", |req| async move {
        // Simulated streaming.
        req.delta_tx.send("tok1".into()).await.unwrap();
        req.delta_tx.send("tok2".into()).await.unwrap();
        Ok(Resp {
            final_text: format!("{}:done", req.prompt),
        })
    });

    let (tx, mut rx) = mpsc::channel::<String>(4);
    let handle = tokio::spawn(async move {
        let mut buf = Vec::new();
        while let Some(d) = rx.recv().await {
            buf.push(d);
        }
        buf
    });

    let resp: Resp = registry
        .request(
            "llm.stream",
            Req {
                delta_tx: tx,
                prompt: "hi".into(),
            },
        )
        .await
        .unwrap();

    let deltas = handle.await.unwrap();
    assert_eq!(deltas, vec!["tok1".to_string(), "tok2".to_string()]);
    assert_eq!(resp.final_text, "hi:done");
}

#[tokio::test]
async fn dispatches_oneshot_sender_for_async_resolution() {
    // Approval-style pattern: handler stashes a oneshot sender for
    // later resolution by some other component (here, simulated
    // by resolving in the handler itself after a tiny delay).
    struct Req {
        prompt: String,
        tx: oneshot::Sender<bool>,
    }
    struct Resp;

    let registry = NativeRegistry::new();
    registry.register::<Req, Resp, _, _>("approval.prompt", |req| async move {
        // Simulate async resolution by a different task/actor.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            let decision = req.prompt.starts_with("safe:");
            let _ = req.tx.send(decision);
        });
        Ok(Resp)
    });

    let (tx, rx) = oneshot::channel();
    let _resp: Resp = registry
        .request(
            "approval.prompt",
            Req {
                prompt: "safe:read_file".into(),
                tx,
            },
        )
        .await
        .unwrap();

    let decision = rx.await.unwrap();
    assert!(decision);
}

#[tokio::test]
async fn unregistered_method_returns_error() {
    let registry = NativeRegistry::new();
    let err = registry
        .request::<String, usize>("missing", "x".into())
        .await
        .expect_err("expected UnregisteredHandler");

    match err {
        NativeRequestError::UnregisteredHandler { method } => assert_eq!(method, "missing"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn type_mismatch_on_request_type_returns_error() {
    let registry = NativeRegistry::new();
    registry.register::<String, usize, _, _>("m", |s| async move { Ok(s.len()) });

    // Call with wrong Req type (u32 instead of String)
    let err = registry
        .request::<u32, usize>("m", 42)
        .await
        .expect_err("expected TypeMismatch on request");

    match err {
        NativeRequestError::TypeMismatch {
            method,
            expected,
            actual,
        } => {
            assert_eq!(method, "m");
            assert!(expected.contains("String"), "expected {expected}");
            assert!(actual.contains("u32"), "actual {actual}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn type_mismatch_on_response_type_returns_error() {
    let registry = NativeRegistry::new();
    registry.register::<String, usize, _, _>("m", |s| async move { Ok(s.len()) });

    // Call with wrong Resp type (String instead of usize)
    let err = registry
        .request::<String, String>("m", "x".into())
        .await
        .expect_err("expected TypeMismatch on response");

    assert!(matches!(err, NativeRequestError::TypeMismatch { .. }));
}

#[tokio::test]
async fn handler_error_propagates_as_handler_failed() {
    let registry = NativeRegistry::new();
    registry.register::<(), (), _, _>("boom", |_| async move { Err("kapow".to_string()) });

    let err = registry
        .request::<(), ()>("boom", ())
        .await
        .expect_err("expected HandlerFailed");

    match err {
        NativeRequestError::HandlerFailed { method, message } => {
            assert_eq!(method, "boom");
            assert_eq!(message, "kapow");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn second_registration_replaces_handler() {
    let registry = NativeRegistry::new();
    registry.register::<u32, u32, _, _>("double", |n| async move { Ok(n * 2) });

    let v: u32 = registry.request("double", 5u32).await.unwrap();
    assert_eq!(v, 10);

    // Tests rely on this: register again with a different impl.
    registry.register::<u32, u32, _, _>("double", |n| async move { Ok(n + 100) });

    let v: u32 = registry.request("double", 5u32).await.unwrap();
    assert_eq!(v, 105);
}

#[tokio::test]
async fn concurrent_dispatches_do_not_deadlock() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let registry = Arc::new(NativeRegistry::new());
    let counter = Arc::new(AtomicUsize::new(0));

    {
        let counter = Arc::clone(&counter);
        registry.register::<u32, u32, _, _>("count", move |n| {
            let counter = Arc::clone(&counter);
            async move {
                // Simulate some work so overlapping dispatches interleave.
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(n)
            }
        });
    }

    let mut handles = Vec::new();
    for i in 0..32u32 {
        let registry = Arc::clone(&registry);
        handles.push(tokio::spawn(async move {
            registry.request::<u32, u32>("count", i).await.unwrap()
        }));
    }

    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(counter.load(Ordering::SeqCst), 32);
}

#[tokio::test]
async fn is_registered_and_len_reflect_state() {
    let registry = NativeRegistry::new();
    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(!registry.is_registered("a"));

    registry.register::<(), (), _, _>("a", |_| async move { Ok(()) });
    registry.register::<(), (), _, _>("b", |_| async move { Ok(()) });

    assert!(registry.is_registered("a"));
    assert!(registry.is_registered("b"));
    assert!(!registry.is_registered("c"));
    assert_eq!(registry.len(), 2);
}

#[tokio::test]
async fn clear_removes_all_handlers() {
    let registry = NativeRegistry::new();
    registry.register::<(), (), _, _>("a", |_| async move { Ok(()) });
    registry.clear();
    assert!(registry.is_empty());
}
