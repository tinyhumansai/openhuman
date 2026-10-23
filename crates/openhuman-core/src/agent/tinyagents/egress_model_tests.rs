use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
#[cfg(feature = "security-module")]
use crate::modules::security::gap::install_port;
#[cfg(feature = "security-module")]
use crate::modules::security::gap::stub::{Asked, StubPort};
use crate::security::live_policy::test_privacy_scope;
use tinyinference_llm::message::Message;
#[cfg(feature = "security-module")]
use tinysecurity_bus::{DenialReason, PrivacyMode as BusMode, PrivacyRiskLevel2, Verdict};

#[cfg(feature = "security-module")]
/// A module that refuses every transfer put to it.
fn refusing() -> Arc<StubPort> {
    Arc::new(StubPort::allowing().privacy(|_| Verdict::Deny {
        reason: DenialReason::Privacy,
    }))
}

fn assessed(level: IdentificationRisk, categories: &[&str]) -> Option<Assessment> {
    Some(Assessment {
        level,
        categories: categories.iter().map(|c| c.to_string()).collect(),
    })
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn non_sensitive_modes_ask_the_module_without_payload_assessment() {
    for (mode, expected) in [
        (PrivacyMode::Standard, BusMode::Standard),
        (PrivacyMode::LocalOnly, BusMode::LocalOnly),
    ] {
        let stub = Arc::new(StubPort::allowing());
        let _port = install_port(stub.clone());
        assert!(refusal_with(
            mode,
            "openai",
            "gpt",
            assessed(IdentificationRisk::High, &["credential_or_identifier"])
        )
        .await
        .is_none());
        let asked = stub.asked();
        let [Asked::Privacy(request)] = asked.as_slice() else {
            panic!("one module question expected");
        };
        assert_eq!(request.privacy, expected);
        assert_eq!(request.risk, None);
    }
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn sensitive_forwards_the_assessment_and_obeys_the_modules_answer() {
    let stub = Arc::new(StubPort::allowing());
    let _port = install_port(stub.clone());
    assert!(refusal_with(
        PrivacyMode::Sensitive,
        "openai",
        "gpt",
        assessed(IdentificationRisk::None, &[])
    )
    .await
    .is_none());
    let asked = stub.asked();
    let [Asked::Privacy(request)] = asked.as_slice() else {
        panic!("one question, got {asked:?}");
    };
    assert_eq!(
        request.risk.as_ref().map(|r| r.level),
        Some(PrivacyRiskLevel2::None)
    );
    assert_eq!(request.destination_label, "openai");
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn sensitive_refuses_whatever_the_module_refuses_with_the_mode_message() {
    let stub = refusing();
    let _port = install_port(stub.clone());
    for assessment in [
        assessed(IdentificationRisk::High, &["credential_or_identifier"]),
        assessed(IdentificationRisk::Unknown, &[]),
        None,
    ] {
        let message = refusal_with(PrivacyMode::Sensitive, "openai", "gpt", assessment)
            .await
            .expect("refused");
        assert!(message.contains("Sensitive privacy mode"), "{message}");
    }
    // An unassessed request (Unknown or missing) is forwarded as "no assessment".
    let asked = stub.asked();
    let risks: Vec<_> = asked
        .iter()
        .map(|asked| match asked {
            Asked::Privacy(request) => request.risk.is_some(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(risks, vec![true, false, false]);
}

struct Counting {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl ChatModel<()> for Counting {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ModelResponse::assistant("ok"))
    }
}

fn request() -> ModelRequest {
    ModelRequest::new(vec![Message::user("hello")])
}

fn route(local: bool) -> (Arc<dyn ChatModel<()>>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let inner: Arc<dyn ChatModel<()>> = Arc::new(Counting {
        calls: calls.clone(),
    });
    (
        EgressAssessedModel::for_route(inner, "openai", "gpt", local),
        calls,
    )
}

#[tokio::test]
#[cfg(feature = "security-module")]
async fn an_external_route_passes_straight_through_outside_sensitive_mode() {
    let stub = Arc::new(StubPort::allowing());
    let _port = install_port(stub.clone());
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    let (model, calls) = route(false);
    model.invoke(&(), request()).await.expect("allowed");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(stub.asked().len(), 1);
}

#[tokio::test]
#[cfg(not(feature = "security-module"))]
async fn an_external_route_fails_closed_without_the_security_module() {
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    let (model, calls) = route(false);
    assert!(model.invoke(&(), request()).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn standard_mode_refuses_an_external_route_when_the_module_cannot_answer() {
    let _port = install_port(Arc::new(StubPort::unavailable()));
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    let (model, calls) = route(false);
    let error = model
        .invoke(&(), request())
        .await
        .expect_err("fails closed");
    assert!(error.to_string().contains("could not confirm"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the route must not run");
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn local_only_refuses_an_external_route_when_the_module_cannot_answer() {
    let _port = install_port(Arc::new(StubPort::unavailable()));
    let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
    let (model, calls) = route(false);
    let error = model
        .invoke(&(), request())
        .await
        .expect_err("fails closed");
    assert!(error.to_string().contains("could not confirm"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "the route must not run");
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn under_sensitive_an_unassessable_request_is_refused_before_the_route_is_called() {
    // No module is serving in a unit test, so the assessment is "unknown" and the
    // module (scripted here) refuses it: the safe answer when it cannot say.
    let _port = install_port(refusing());
    let _mode = test_privacy_scope(PrivacyMode::Sensitive);
    let (model, calls) = route(false);
    let error = model.invoke(&(), request()).await.expect_err("refused");
    assert!(
        error.to_string().contains("Sensitive privacy mode"),
        "{error}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the route must not be called"
    );
    let streamed = model.stream(&(), request()).await.expect_err("refused");
    assert!(
        streamed.to_string().contains("Sensitive privacy mode"),
        "{streamed}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_route_that_stays_on_the_device_is_never_assessed() {
    let _mode = test_privacy_scope(PrivacyMode::Sensitive);
    let (model, calls) = route(true);
    model
        .invoke(&(), request())
        .await
        .expect("local route is not assessed");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "security-module")]
/// A local primary with an external fallback: each route is wrapped on its own,
/// so the fallback hop is assessed even though the primary never leaves.
#[tokio::test]
async fn a_local_primary_does_not_exempt_an_external_fallback_route() {
    let _port = install_port(refusing());
    let _mode = test_privacy_scope(PrivacyMode::Sensitive);
    let (primary, primary_calls) = route(true);
    let (fallback, fallback_calls) = route(false);
    primary
        .invoke(&(), request())
        .await
        .expect("primary is local");
    assert!(fallback.invoke(&(), request()).await.is_err());
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
}
