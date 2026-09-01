//! Tests for the connector module shim.

use super::{is_unsupported_by_route, methods};

#[test]
fn classified_errors_start_with_the_frontend_marker() {
    assert_eq!(
        super::normalize_error(
            methods::EXECUTE,
            "Execute: [composio:error:rate_limited] Please retry later".to_string()
        ),
        "[composio:error:rate_limited] Please retry later"
    );
}

#[test]
fn classified_errors_stay_at_byte_zero_after_tinybus_failure_wrapping() {
    assert_eq!(
        super::normalize_error(
            methods::EXECUTE,
            "Execute: ai.tinyhumans.tinybus.Error.Failed: [composio:error:rate_limited] Please retry later".to_string()
        ),
        "[composio:error:rate_limited] Please retry later"
    );
}

#[test]
fn unclassified_errors_keep_the_member_context() {
    assert_eq!(
        super::normalize_error(methods::EXECUTE, "Execute: module unavailable".to_string()),
        "Execute: module unavailable"
    );
}

#[test]
fn embedded_classification_text_is_not_promoted() {
    let error =
        "Execute: provider returned [composio:error:rate_limited] as plain text".to_string();
    assert_eq!(
        super::normalize_error(methods::EXECUTE, error.clone()),
        error
    );
}

#[test]
fn member_names_come_from_the_contract() {
    // Spelled through `tinyconnectors_bus` rather than as string literals, so a
    // renamed member is a compile error here instead of an "unknown method" at
    // runtime on a user's machine. This holds with gates off too: the contract
    // crate is an ordinary dependency, not a gated one.
    assert_eq!(methods::LIST_TOOLKITS, "ListToolkits");
    assert_eq!(methods::AUTHORIZE, "Authorize");
    assert_eq!(methods::EXECUTE, "Execute");
}

#[test]
fn recognises_the_module_refusing_a_member_its_route_cannot_serve() {
    // Direct mode has no per-user allowlist and no webhook endpoint, so the
    // module refuses those members by name rather than returning an empty
    // result that would read like an answer.
    assert!(is_unsupported_by_route(
        "ListToolkits: ListToolkits is not available over the direct route"
    ));
    assert!(is_unsupported_by_route(
        "DeleteConnection is not available over the direct route"
    ));
}

#[test]
fn does_not_mistake_a_real_failure_for_a_route_refusal() {
    // Getting this backwards renders "no curated allowlist" over a genuine
    // outage, and the user never learns their integration is broken.
    for error in [
        "ListToolkits: request to /agent-integrations/composio/toolkits failed: 502 bad gateway",
        "ListToolkits: response from /toolkits did not match the contract",
        "unknown module 'tinyconnectors'",
        "the module runtime is unavailable",
        "this module was loaded without a connector route",
    ] {
        assert!(!is_unsupported_by_route(error), "{error}");
    }
}

#[cfg(not(feature = "modules"))]
#[tokio::test]
async fn a_build_without_the_module_loader_says_so() {
    // Connectors live in the module. A build with gates off has no loader, so
    // it has no connectors — and should say that rather than fail obscurely.
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = crate::openhuman::config::Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Default::default()
    };

    let error = super::call_bare::<serde_json::Value>(&config, methods::LIST_TOOLKITS)
        .await
        .expect_err("no loader, no connectors");
    assert!(error.contains("modules"), "{error}");
    assert!(error.contains(methods::LIST_TOOLKITS), "{error}");
}
