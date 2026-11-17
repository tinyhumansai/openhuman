use super::*;

#[test]
fn native_prefix_claims_only_oh_slugs() {
    assert!(NativeToolBackend.claims("oh:web_search"));
    assert!(!NativeToolBackend.claims("GMAIL_SEND_EMAIL"));
}

#[test]
fn composio_is_the_catch_all_and_is_registered_last() {
    // The empty prefix claims everything, so anything registered after it
    // would never be reached. Pinning the position turns that ordering
    // mistake into a test failure rather than a silently dead backend.
    assert_eq!(ComposioToolBackend.prefix(), "");
    assert!(ComposioToolBackend.claims("ANYTHING_AT_ALL"));
    assert_eq!(
        BACKENDS.last().map(|b| b.name()),
        Some("composio"),
        "a backend registered after the empty-prefix catch-all is unreachable"
    );
    assert!(
        BACKENDS[..BACKENDS.len() - 1]
            .iter()
            .all(|b| !b.prefix().is_empty()),
        "only the last backend may declare an empty prefix"
    );
}

#[test]
fn dispatch_routes_each_namespace_to_its_owner() {
    assert_eq!(
        backend_for("oh:web_search").map(|b| b.name()),
        Some("native")
    );
    assert_eq!(
        backend_for("GMAIL_SEND_EMAIL").map(|b| b.name()),
        Some("composio")
    );
}

#[test]
fn unknown_slug_names_registered_backends() {
    // The message has to say what IS available; "unknown tool" alone leaves
    // an author guessing whether they typo'd the slug or the prefix.
    let err = unclaimed_slug_error("bogus:thing").to_string();
    assert!(err.contains("bogus:thing"), "names the slug: {err}");
    for b in BACKENDS {
        assert!(
            err.contains(b.name()),
            "names backend `{}`: {err}",
            b.name()
        );
    }
}

#[tokio::test]
async fn preflight_runs_through_backend_list() {
    // The dry-run invoker asks whichever backend owns the slug, rather than
    // testing the prefix itself and calling one namespace's preflight
    // directly. A native slug must therefore reach the native backend's
    // no-op and pass without a Composio catalog lookup.
    let config = Config::default();
    let backend = backend_for("oh:web_search").expect("native backend claims oh: slugs");
    assert_eq!(backend.name(), "native");
    backend
        .preflight(&config, "oh:web_search", &serde_json::json!({}))
        .await
        .expect("native preflight is a no-op and cannot fail");
}
