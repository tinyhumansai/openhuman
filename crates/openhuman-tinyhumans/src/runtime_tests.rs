//! The builder is the configuration path: it must hand the embed builder the
//! SDK transport, the hosted controllers and the Jev ranker, set the product
//! identity, and forward every preset and knob unchanged.

use super::*;
use crate::transport::BackendTransport;
use openhuman_embed::seams::DomainGroup;
use openhuman_embed::BuilderSummary;

fn connected(builder: RuntimeBuilder) -> BuilderSummary {
    builder.connect().expect("connect").summary()
}

#[test]
fn connect_wires_transport_hosted_controllers_and_ranker() {
    let _registry = crate::registry_test_lock();

    let summary = connected(RuntimeBuilder::new());
    assert_eq!(summary.controller_extensions, vec![DomainGroup::Hosted]);
    #[cfg(feature = "jev")]
    assert_eq!(
        summary.tool_ranker.as_deref(),
        Some(ToolRanker::kind(&crate::jev::TinyHumansJevRanker::new()))
    );
    #[cfg(not(feature = "jev"))]
    assert!(summary.tool_ranker.is_none());
    // The shared transport is built, as `install` leaves it.
    assert!(crate::is_installed());
}

#[test]
fn into_embed_stays_unconnected() {
    let summary = RuntimeBuilder::new().into_embed().summary();
    assert!(summary.controller_extensions.is_empty());
    assert!(summary.tool_ranker.is_none());
}

#[test]
fn deferred_builder_connects_through_tinyhumans_wrapper() {
    let _registry = crate::registry_test_lock();

    let summary = connected(RuntimeBuilder::new().defer_services());
    assert!(
        !summary.controller_extensions.is_empty(),
        "deferring services still connects the hosted controllers"
    );
}

#[test]
fn hosted_controllers_and_jev_can_be_turned_off() {
    let _registry = crate::registry_test_lock();

    let summary = connected(
        RuntimeBuilder::new()
            .hosted_controllers(false)
            .jev_ranker(false),
    );
    assert!(summary.controller_extensions.is_empty());
    assert!(summary.tool_ranker.is_none());
}

#[test]
fn product_identity_is_set_on_connect() {
    let _registry = crate::registry_test_lock();

    let _guard = crate::backend::product::product_identity_test_lock();
    let identity = crate::ProductIdentity::new("opencompany").unwrap();
    let builder = RuntimeBuilder::new().product_identity(identity.clone());
    // Not applied until the builder connects.
    assert_ne!(crate::product_identity(), identity);
    let wired = wiring(&builder.install).expect("wiring");
    assert_eq!(crate::product_identity(), identity);
    assert_eq!(wired.transport.product_identity(), "opencompany");

    // Restore the default identity and a transport carrying it.
    let restored = wiring(
        &InstallOptions::default()
            .hosted_controllers(false)
            .tool_ranker(false)
            .product_identity(crate::ProductIdentity::default()),
    )
    .expect("restore");
    assert_eq!(
        restored.transport.product_identity(),
        crate::backend::DEFAULT_PRODUCT_IDENTITY
    );
}

#[test]
fn presets_forward_to_the_embed_presets() {
    let pairs = [
        (
            RuntimeBuilder::library(),
            openhuman_embed::RuntimeBuilder::library(),
        ),
        (
            RuntimeBuilder::desktop(),
            openhuman_embed::RuntimeBuilder::desktop(),
        ),
        (
            RuntimeBuilder::cli(),
            openhuman_embed::RuntimeBuilder::cli(),
        ),
        (
            RuntimeBuilder::tui(),
            openhuman_embed::RuntimeBuilder::tui(),
        ),
    ];
    for (ours, theirs) in pairs {
        let (ours, theirs) = (ours.into_embed().summary(), theirs.summary());
        assert_eq!(ours.host_kind, theirs.host_kind);
        assert_eq!(ours.domains, theirs.domains);
        assert_eq!(ours.services, theirs.services);
        assert_eq!(ours.config_source, theirs.config_source);
        assert_eq!(
            format!("{:?}", ours.workspace),
            format!("{:?}", theirs.workspace)
        );
    }
}

#[test]
fn knobs_and_seams_forward_unchanged() {
    let _registry = crate::registry_test_lock();

    struct Ranker;
    #[async_trait::async_trait]
    impl ToolRanker for Ranker {
        fn kind(&self) -> &'static str {
            "test-ranker"
        }
        async fn rank(
            &self,
            _intent: &str,
            _context: &tinytools::RankContext,
            _candidates: &[tinytools::RankCandidate],
            _limit: usize,
        ) -> Result<Vec<tinytools::RankHit>, tinytools::RankError> {
            Ok(Vec::new())
        }
    }

    let summary = connected(
        RuntimeBuilder::desktop()
            .token(TokenSource::Fixed(Arc::new("bearer".to_string())))
            .listen("127.0.0.1", 7791)
            .action_dir("/srv/work")
            .workspace_dir("/srv/state")
            .session_store(Arc::new(openhuman_embed::InMemorySessionStores::new()))
            .tool_ranker(Arc::new(Ranker)),
    );
    assert!(summary.fixed_token);
    assert_eq!(summary.listen_host.as_deref(), Some("127.0.0.1"));
    assert_eq!(summary.listen_port, Some(7791));
    assert_eq!(summary.action_dir, Some(PathBuf::from("/srv/work")));
    assert_eq!(summary.workspace_dir, Some(PathBuf::from("/srv/state")));
    assert!(summary.has_session_store);
    // A host ranker replaces the Jev one rather than being overwritten by it.
    assert_eq!(summary.tool_ranker.as_deref(), Some("test-ranker"));
    assert_eq!(summary.controller_extensions, vec![DomainGroup::Hosted]);
}

struct HostRanker;

#[async_trait::async_trait]
impl ToolRanker for HostRanker {
    fn kind(&self) -> &'static str {
        "host-ranker"
    }
    async fn rank(
        &self,
        _intent: &str,
        _context: &tinytools::RankContext,
        _candidates: &[tinytools::RankCandidate],
        _limit: usize,
    ) -> Result<Vec<tinytools::RankHit>, tinytools::RankError> {
        Ok(Vec::new())
    }
}

#[test]
fn a_ranker_on_a_wrapped_embed_builder_is_kept() {
    let _registry = crate::registry_test_lock();

    let embed = openhuman_embed::RuntimeBuilder::new().tool_ranker(Arc::new(HostRanker));
    let summary = connected(RuntimeBuilder::from_embed(embed));
    assert_eq!(summary.tool_ranker.as_deref(), Some("host-ranker"));
}

#[test]
fn turning_jev_back_on_does_not_replace_a_host_ranker() {
    let _registry = crate::registry_test_lock();

    let summary = connected(
        RuntimeBuilder::new()
            .tool_ranker(Arc::new(HostRanker))
            .jev_ranker(true),
    );
    assert_eq!(summary.tool_ranker.as_deref(), Some("host-ranker"));
}

#[test]
fn connect_keeps_cli_knobs_for_the_host_boot() {
    let _registry = crate::registry_test_lock();

    use openhuman_embed::seams::HostBoot;
    use openhuman_embed::{DomainSet, ServiceSet, TokenSource};

    let mut domains = DomainSet::full();
    domains.mcp = false;
    // `run_from_args` is `connect()` followed by the embed builder's
    // `run_from_args`, which wraps the connected builder in a `HostBoot` for
    // the core's `run` / `serve` launcher. This covers the connect step and
    // the HostBoot extraction; the dispatch itself needs a live CLI process
    // and is covered by the cli/saas e2e suites.
    let connected = RuntimeBuilder::cli()
        .domains(domains)
        .services(ServiceSet::headless_api())
        .token(TokenSource::Fixed(Arc::new("bearer".into())))
        .listen("0.0.0.0", 7812)
        .connect()
        .expect("connect");
    let boot = HostBoot::new(connected);
    let handed = openhuman_embed::RuntimeBuilder::from_host_boot(&boot).expect("the builder");
    let summary = handed.summary();
    assert_eq!(summary.domains.map(|d| d.mcp), Some(false));
    assert_eq!(summary.services, Some(ServiceSet::headless_api()));
    assert!(summary.fixed_token);
    assert_eq!(summary.listen_host.as_deref(), Some("0.0.0.0"));
    assert_eq!(summary.listen_port, Some(7812));
    assert_eq!(summary.controller_extensions, vec![DomainGroup::Hosted]);
}
