use super::*;
use serde_json::json;

fn config_in(dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

fn decode<T: DeserializeOwned>(value: Value) -> HostResult<T> {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn an_index_saved_through_the_host_loads_back_per_kind() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir.path()),
    );

    let missing = dispatch(
        &session,
        host_methods::LOAD_INDEX,
        json!([{ "kind": "slang" }]),
    )
    .await
    .unwrap();
    assert_eq!(decode::<Option<String>>(missing), Ok(None));

    for (kind, body) in [
        ("slang", "{\"terms\":1}"),
        ("memes", "{\"memes\":2}"),
        ("remixed", "[\"m1\"]"),
    ] {
        let saved = dispatch(
            &session,
            host_methods::SAVE_INDEX,
            json!([{ "kind": kind, "json": body }]),
        )
        .await
        .unwrap();
        assert_eq!(decode::<()>(saved), Ok(()));
    }
    for (kind, body) in [
        ("slang", "{\"terms\":1}"),
        ("memes", "{\"memes\":2}"),
        ("remixed", "[\"m1\"]"),
    ] {
        let loaded = dispatch(
            &session,
            host_methods::LOAD_INDEX,
            json!([{ "kind": kind }]),
        )
        .await
        .unwrap();
        assert_eq!(decode::<Option<String>>(loaded), Ok(Some(body.into())));
    }
}

#[tokio::test]
async fn without_managed_jev_evaluate_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir.path()),
    );
    let request = json!([{ "body": { "questions": {}, "messages": [] } }]);
    let result = dispatch(&session, host_methods::EVALUATE, request)
        .await
        .unwrap();
    assert_eq!(
        decode::<EvaluateEnvelope>(result),
        Err(HostFailure::Unsupported)
    );
}

#[tokio::test]
async fn search_switched_off_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config_in(dir.path());
    config.search.enabled = Some(false);
    let session = Session::new(&crate::core::runtime::DispatchCtx::host_default(), &config);
    let result = dispatch(
        &session,
        host_methods::SEARCH,
        json!([{ "query": "q", "max_results": 3 }]),
    )
    .await
    .unwrap();
    assert_eq!(
        decode::<Vec<tinymemes_bus::SearchResult>>(result),
        Err(HostFailure::Unsupported)
    );
}

#[tokio::test]
async fn deliver_is_acknowledged_and_unknown_members_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::new(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir.path()),
    );
    let delivered = dispatch(
        &session,
        host_methods::DELIVER,
        json!([{ "reply": "hi", "remixed": true }]),
    )
    .await
    .unwrap();
    assert_eq!(decode::<()>(delivered), Ok(()));
    assert!(dispatch(&session, "Nope", json!([])).await.is_err());
    assert!(dispatch(&session, host_methods::COMPLETE, json!("bad"))
        .await
        .is_err());
}

#[test]
fn registrations_are_independent_and_forgotten_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path());
    let a = register(&crate::core::runtime::DispatchCtx::host_default(), &config);
    let b = register(&crate::core::runtime::DispatchCtx::host_default(), &config);
    assert_ne!(a.correlation(), b.correlation());
    assert!(session(a.correlation()).is_some() && session(b.correlation()).is_some());
    let a_id = a.correlation().to_owned();
    drop(a);
    assert!(
        session(&a_id).is_none(),
        "a failed call leaves nothing behind"
    );
    assert!(session(b.correlation()).is_some());
    assert!(session("").is_none() && session("unknown").is_none());
}

#[test]
fn a_kept_registration_lives_until_the_module_reports_the_end() {
    let dir = tempfile::tempdir().unwrap();
    let registration = register(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir.path()),
    );
    let id = registration.correlation().to_owned();
    registration.keep_for_background();
    assert!(
        session(&id).is_some(),
        "background research may still call back"
    );
    end(&id);
    assert!(session(&id).is_none());
    end(&id);
}

#[test]
fn the_correlation_id_is_read_from_the_first_argument_only() {
    assert_eq!(
        correlation_of(&json!([{ "correlation": "c1", "kind": "slang" }])),
        Some("c1")
    );
    for bad in [
        json!([{ "correlation": "" }]),
        json!([{ "correlation": 7 }]),
        json!([{}]),
        json!([]),
        json!({ "correlation": "c1" }),
        json!("c1"),
    ] {
        assert_eq!(correlation_of(&bad), None, "{bad}");
    }
}

#[tokio::test]
async fn a_remix_session_answers_under_the_context_that_registered_it() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    let dir = tempfile::tempdir().unwrap();
    let caller = CoreContext::for_test(DomainSet::full(), None);
    let other = CoreContext::for_test(DomainSet::full(), None);
    let session = Session::new(
        &DispatchCtx::new(Arc::clone(&caller)),
        &config_in(dir.path()),
    );
    // A callback arrives on the module bus while some other context is active.
    let seen = CoreContext::scope(Arc::clone(&other), async { session.dispatch_ctx() }).await;
    assert!(Arc::ptr_eq(seen.core().expect("bound"), &caller));
}
