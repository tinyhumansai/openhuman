//! Tests for the host half of privacy enforcement: which class a descriptor
//! belongs to, what is asked of the module, and how its answer is rendered. What
//! the module decides for each mode is pinned in the module's own tests.

use super::*;
use crate::config::PrivacyMode;

fn external_composio() -> EgressDescriptor {
    EgressDescriptor::composio("GITHUB_CREATE_ISSUE")
}

// ── data_class: the host's knowledge of which routes carry user content ─────

#[test]
fn integration_class_comes_from_the_descriptor_not_backend_route_names() {
    let unclassified = EgressDescriptor::integration("/auth/refresh");
    assert_ne!(
        data_class(&unclassified),
        PrivacyClass::ControlPlane,
        "core must not infer hosted-backend route policy from endpoint names"
    );

    let host_classified = EgressDescriptor::integration("/agent-integrations/composio/execute")
        .with_control_plane(true);
    assert_eq!(data_class(&host_classified), PrivacyClass::ControlPlane);
}

#[test]
fn control_plane_only_applies_to_integration_reason() {
    // A network fetch whose host happens to spell a backend path is still a
    // user-data transfer, never control-plane.
    let net = EgressDescriptor::network_fetch("api.example.com").with_control_plane(true);
    assert_eq!(net.reason, EgressReason::NetworkFetch);
    assert_eq!(data_class(&net), PrivacyClass::UserContent);
}

#[test]
fn a_metadata_only_transfer_is_metadata_and_anything_more_is_content() {
    let routing = EgressDescriptor::integration("/agent-integrations/parallel/usage");
    assert_eq!(routing.data_kinds, vec![DataKind::Metadata]);
    assert_eq!(data_class(&routing), PrivacyClass::Metadata);
    let upload = routing.with_data_kind(DataKind::FileContent);
    assert_eq!(data_class(&upload), PrivacyClass::UserContent);
}

// ── Refusal rendering ──────────────────────────────────────────────────────

#[test]
fn refusal_messages_name_the_service_and_the_mode() {
    let desc = external_composio();
    let local = block_message(PrivacyMode::LocalOnly, &desc, Outcome::Refused);
    assert!(
        local.contains("Local-only privacy mode is active"),
        "{local}"
    );
    assert!(local.contains("GITHUB_CREATE_ISSUE"), "{local}");
    let sensitive = block_message(PrivacyMode::Sensitive, &desc, Outcome::Refused);
    assert!(
        sensitive.contains("Sensitive privacy mode is active"),
        "{sensitive}"
    );
    let unavailable = block_message(PrivacyMode::LocalOnly, &desc, Outcome::Unavailable);
    assert!(unavailable.contains("could not confirm"), "{unavailable}");
}

#[cfg(feature = "security-module")]
mod with_module {
    use super::*;
    use crate::modules::security::gap::{install_privacy_port, PrivacyPort};
    use crate::security::live_policy::test_privacy_scope;
    use std::sync::{Arc, Mutex};
    use tinysecurity_bus::{
        Decision, PrivacyCheckRequest2, PrivacyDataClass2, PrivacyMode as BusMode,
        PrivacyRiskLevel2, Verdict,
    };

    #[derive(Debug)]
    struct StubPort {
        answer: Result<Decision, String>,
        asked: Mutex<Vec<PrivacyCheckRequest2>>,
    }

    impl StubPort {
        fn allowing() -> Arc<Self> {
            Self::answer(Verdict::Allow)
        }

        fn refusing() -> Arc<Self> {
            Self::answer(Verdict::Deny {
                reason: tinysecurity_bus::DenialReason::Privacy,
            })
        }

        fn answer(verdict: Verdict) -> Arc<Self> {
            Arc::new(Self {
                answer: Ok(Decision {
                    generation: 1,
                    verdict,
                    cacheable: false,
                }),
                asked: Mutex::new(Vec::new()),
            })
        }

        fn unavailable() -> Arc<Self> {
            Arc::new(Self {
                answer: Err("module unavailable".into()),
                asked: Mutex::new(Vec::new()),
            })
        }

        fn asked(&self) -> Vec<PrivacyCheckRequest2> {
            self.asked.lock().expect("privacy ask lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl PrivacyPort for StubPort {
        async fn privacy(&self, request: PrivacyCheckRequest2) -> Result<Decision, String> {
            self.asked
                .lock()
                .map_err(|_| "privacy ask lock poisoned".to_owned())?
                .push(request);
            self.answer.clone()
        }
    }

    #[tokio::test]
    async fn standard_mode_uses_the_module_permission() {
        let stub = StubPort::allowing();
        let _port = install_privacy_port(stub.clone());
        assert!(
            egress_blocks_message(PrivacyMode::Standard, &external_composio())
                .await
                .is_none()
        );
        let asked = stub.asked();
        let [request] = asked.as_slice() else {
            panic!("Standard mode must ask the privacy module exactly once");
        };
        assert_eq!(request.privacy, BusMode::Standard);
    }

    #[tokio::test]
    async fn a_restrictive_mode_asks_the_module_with_the_descriptor_facts() {
        let stub = StubPort::refusing();
        let _port = install_privacy_port(stub.clone());
        let desc = external_composio().with_risk(IdentificationRisk::Low, vec!["email".into()]);
        let message = egress_blocks_message(PrivacyMode::Sensitive, &desc)
            .await
            .expect("the module refused");
        assert!(
            message.contains("Sensitive privacy mode is active"),
            "{message}"
        );
        let asked = stub.asked();
        let [request] = asked.as_slice() else {
            panic!("exactly one privacy question, got {asked:?}");
        };
        assert_eq!(request.privacy, BusMode::Sensitive);
        assert!(request.external);
        assert_eq!(request.class, PrivacyDataClass2::UserContent);
        assert_eq!(request.destination_label, "composio");
        let risk = request.risk.as_ref().expect("assessed risk is forwarded");
        assert_eq!(risk.level, PrivacyRiskLevel2::Low);
        assert_eq!(risk.categories, vec!["email".to_string()]);
    }

    #[tokio::test]
    async fn an_unassessed_payload_is_forwarded_as_no_risk() {
        let stub = StubPort::refusing();
        let _port = install_privacy_port(stub.clone());
        let _ = egress_blocks_message(PrivacyMode::Sensitive, &external_composio()).await;
        let asked = stub.asked();
        let [request] = asked.as_slice() else {
            panic!("exactly one privacy question");
        };
        assert!(request.risk.is_none(), "Unknown means not assessed");
    }

    #[tokio::test]
    async fn control_plane_and_local_runtime_facts_are_forwarded() {
        let stub = StubPort::allowing();
        let _port = install_privacy_port(stub.clone());
        let pricing = EgressDescriptor::integration("pricing").with_control_plane(true);
        assert!(egress_blocks_message(PrivacyMode::LocalOnly, &pricing)
            .await
            .is_none());
        let local = EgressDescriptor::inference("ollama", "llama3", false);
        assert!(egress_blocks_message(PrivacyMode::LocalOnly, &local)
            .await
            .is_none());
        let asked = stub.asked();
        let classes: Vec<_> = asked
            .iter()
            .map(|request| (request.class, request.external))
            .collect();
        assert_eq!(
            classes,
            vec![
                (PrivacyDataClass2::ControlPlane, true),
                (PrivacyDataClass2::UserContent, false)
            ]
        );
    }

    #[tokio::test]
    async fn an_unavailable_module_refuses_under_a_restrictive_mode() {
        let _port = install_privacy_port(StubPort::unavailable());
        let message = egress_blocks_message(PrivacyMode::LocalOnly, &external_composio())
            .await
            .expect("fails closed");
        assert!(message.contains("could not confirm"), "{message}");
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        assert!(enforce_egress(&external_composio()).await.is_err());
        assert!(enforce_egress_blocking(&external_composio()).is_err());
    }

    #[tokio::test]
    async fn standard_mode_refuses_when_the_module_cannot_answer() {
        let _port = install_privacy_port(StubPort::unavailable());
        let message = egress_blocks_message(PrivacyMode::Standard, &external_composio())
            .await
            .expect("module failure fails closed in every privacy mode");
        assert!(message.contains("could not confirm"), "{message}");
        let _mode = test_privacy_scope(PrivacyMode::Standard);
        assert!(enforce_egress(&external_composio()).await.is_err());
        assert!(enforce_egress_blocking(&external_composio()).is_err());
    }

    #[tokio::test]
    async fn the_tool_block_carries_the_policy_marker_and_clears_when_allowed() {
        let mut desc = EgressDescriptor::network_fetch("api.example.com");
        desc = desc.with_data_kind(DataKind::ToolArguments);
        {
            let _port = install_privacy_port(StubPort::refusing());
            let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
            let message = local_only_tool_block(&desc).await.expect("blocked");
            assert!(message.starts_with(POLICY_BLOCKED_MARKER), "{message}");
            assert!(message.contains("api.example.com"), "{message}");
            // The synchronous network-tool trait has no async admission path;
            // its bounded bridge must refuse when no module runtime is armed.
            assert!(local_only_tool_block_blocking(&desc).is_some());
        }
        let _port = install_privacy_port(StubPort::allowing());
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        assert!(local_only_tool_block(&desc).await.is_none());
    }
}
