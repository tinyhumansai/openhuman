use super::*;
use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection};
use tinycomputer_bus::browser_egress::{
    methods, BrowserEgressConsumeRequest, BrowserEgressTarget, BrowserEgressValidateRequest,
};

fn target(policy_id: &str, generation: u64, hostname: &str, port: u16) -> BrowserEgressTarget {
    BrowserEgressTarget {
        policy_id: policy_id.to_owned(),
        expected_generation: generation,
        session_id: "browser-session-test-7f9a".into(),
        scheme: "https".into(),
        hostname: hostname.to_owned(),
        port,
    }
}

// This linked module claims TinyComputer's real interface name through
// ModuleHost, which gives its calls an authenticated module sender. A plain
// Connection that merely requests the same name does not have that identity.
struct AuthenticatedComputerPeer(Connection);

#[async_trait::async_trait]
impl tinybus::Interface for AuthenticatedComputerPeer {
    fn name(&self) -> tinybus::InterfaceName {
        tinycomputer_bus::names::INTERFACE.try_into().unwrap()
    }

    fn members(&self) -> Vec<tinybus::MemberName> {
        ["Authorize", "Consume", "Validate"]
            .into_iter()
            .map(|member| member.try_into().unwrap())
            .collect()
    }

    async fn call(
        &self,
        member: &tinybus::MemberName,
        args: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        let callback = self.0.proxy(
            contract::INTERFACE,
            contract::OBJECT_PATH,
            contract::INTERFACE,
        )?;
        match member.as_str() {
            "Authorize" => {
                let (target,): (BrowserEgressTarget,) = serde_json::from_value(args)
                    .map_err(|error| tinybus::Error::failed(error.to_string()))?;
                let grant: contract::BrowserEgressGrant =
                    callback.call(methods::AUTHORIZE, (target,)).await?;
                serde_json::to_value(grant)
                    .map_err(|error| tinybus::Error::failed(error.to_string()))
            }
            "Consume" => {
                let (request,): (BrowserEgressConsumeRequest,) = serde_json::from_value(args)
                    .map_err(|error| tinybus::Error::failed(error.to_string()))?;
                let decision: BrowserEgressDecision =
                    callback.call(methods::CONSUME, (request,)).await?;
                serde_json::to_value(decision)
                    .map_err(|error| tinybus::Error::failed(error.to_string()))
            }
            "Validate" => {
                let (request,): (BrowserEgressValidateRequest,) = serde_json::from_value(args)
                    .map_err(|error| tinybus::Error::failed(error.to_string()))?;
                let decision: BrowserEgressDecision =
                    callback.call(methods::VALIDATE, (request,)).await?;
                serde_json::to_value(decision)
                    .map_err(|error| tinybus::Error::failed(error.to_string()))
            }
            _ => Err(tinybus::Error::failed("unknown test peer method")),
        }
    }
}

async fn authenticated_computer_setup(connection: Connection) -> tinybus::Result<()> {
    connection
        .serve_at(
            tinycomputer_bus::names::OBJECT_PATH.try_into().unwrap(),
            AuthenticatedComputerPeer(connection.clone()),
        )
        .await?;
    connection
        .request_name(tinycomputer_bus::names::INTERFACE)
        .await?;
    Ok(())
}

mod authenticated_computer_export {
    tinybus_module::module_export_static! {
        setup = super::authenticated_computer_setup,
        worker_threads = 1,
        provides = ["ai.tinyhumans.tinycomputer.Desktop"],
        methods = ["Authorize", "Consume", "Validate"],
        signals = [],
        requires = [],
        optional = [],
        lazy = false,
    }
}

async fn callback_bus() -> (
    MemoryBus,
    Connection,
    tokio::task::JoinHandle<tinybus::Result<()>>,
) {
    let transport = MemoryBus::new();
    let broker = Broker::new();
    let task = broker.spawn(transport.clone());
    let host = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    install(&host).await.unwrap();
    (transport, host, task)
}

#[tokio::test]
async fn browser_egress_callback_rejects_unauthenticated_peers() {
    let (transport, _host, _task) = callback_bus().await;
    let caller = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = caller
        .proxy(
            contract::INTERFACE,
            contract::OBJECT_PATH,
            contract::INTERFACE,
        )
        .unwrap();
    let result: tinybus::Result<contract::BrowserEgressGrant> = proxy
        .call(
            methods::AUTHORIZE,
            (target("host-policy", 1, "8.8.8.8", 443),),
        )
        .await;
    assert!(result.is_err(), "an unclaimed bus peer must be refused");
}

#[tokio::test]
async fn browser_egress_callback_fails_closed_without_a_host_registered_scope() {
    let (transport, _host, _task) = callback_bus().await;
    let caller = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    caller
        .request_name(tinycomputer_bus::names::INTERFACE)
        .await
        .unwrap();
    let proxy = caller
        .proxy(
            contract::INTERFACE,
            contract::OBJECT_PATH,
            contract::INTERFACE,
        )
        .unwrap();
    let result: tinybus::Result<contract::BrowserEgressGrant> = proxy
        .call(
            methods::AUTHORIZE,
            (target("unregistered", 1, "8.8.8.8", 443),),
        )
        .await;
    assert!(result.is_err(), "unknown host policy IDs must be refused");
}

/// Exercises the authenticated callback against a real TinySecurity cdylib.
/// The fixture is supplied by `scripts/ci/security-browser-egress-fixture.sh`.
#[tokio::test(flavor = "multi_thread")]
#[cfg(feature = "native-security-tests")]
async fn browser_egress_callbacks_use_tinysecurity_one_use_grants_and_leases() {
    use crate::modules::security::native_tests::native_fixture_configured;

    let fixture = native_fixture_configured(false, |_| {}).await;
    install(&fixture.client).await.unwrap();
    let owner = tinysecurity_bus::SubjectScope {
        user: "native-test-user".into(),
        workspace: "native-test-workspace".into(),
        agent: "native-test-agent".into(),
    };
    let policy = tinysecurity_bus::EgressPolicy {
        owner,
        ceiling: tinysecurity_bus::AccessTier::Full,
        privacy: tinysecurity_bus::PrivacyMode::Standard,
        destinations: Vec::new(),
        data_categories: vec![tinysecurity_bus::DataCategory::OpaqueBrowserTraffic],
        local_capabilities: Vec::new(),
        validity_ms: 30_000,
        open_public: true,
        preflight_transports: vec![tinysecurity_bus::EgressTransport::Browser],
    };
    let policy_id: tinysecurity_bus::EgressPolicyId = fixture
        .proxy
        .call_confidential(
            tinysecurity_bus::names::methods::REGISTER_EGRESS_POLICY,
            (policy,),
        )
        .await
        .expect("trusted host registers browser policy");
    let info: tinysecurity_bus::PolicyInfo = fixture
        .proxy
        .call(tinysecurity_bus::names::methods::POLICY_INFO, ())
        .await
        .expect("read actual module generation");
    crate::modules::security::egress::remember_browser_dial_binding_for_test(
        policy_id.clone(),
        info.generation,
        fixture.proxy.clone(),
    );

    let host_callback = fixture
        .client
        .proxy(
            contract::INTERFACE,
            contract::OBJECT_PATH,
            contract::INTERFACE,
        )
        .unwrap();
    assert!(host_callback.is_available().await.unwrap());
    fixture
        .host
        .attach_linked_with_config(
            authenticated_computer_export::linked_module().unwrap(),
            serde_json::json!({}),
        )
        .unwrap();
    let attacker = Connection::connect(fixture.transport.connect().await.unwrap())
        .await
        .unwrap();
    attacker
        .request_name("ai.tinyhumans.tinycomputer.Desktop")
        .await
        .unwrap_err();
    let attacker_callback = attacker
        .proxy(
            contract::INTERFACE,
            contract::OBJECT_PATH,
            contract::INTERFACE,
        )
        .unwrap();
    let spoofed: tinybus::Result<contract::BrowserEgressGrant> = attacker_callback
        .call(
            methods::AUTHORIZE,
            (target(&policy_id.0, info.generation, "8.8.8.8", 443),),
        )
        .await;
    assert!(
        spoofed.is_err(),
        "a plain peer cannot impersonate TinyComputer"
    );
    let proxy = fixture
        .client
        .proxy(
            tinycomputer_bus::names::INTERFACE,
            tinycomputer_bus::names::OBJECT_PATH,
            tinycomputer_bus::names::INTERFACE,
        )
        .unwrap();

    let direct = crate::modules::security::egress::authorize_browser_dial(target(
        &policy_id.0,
        info.generation,
        "8.8.8.8",
        443,
    ))
    .await
    .expect("direct host adapter authorization should expose authority decoding errors");
    assert!(!direct.authorization.is_empty());
    let target = target(&policy_id.0, info.generation, "8.8.8.8", 443);
    let grant: contract::BrowserEgressGrant = proxy
        .call("Authorize", (target.clone(),))
        .await
        .expect("TinySecurity authorizes a public IP without a network dial");
    assert_eq!(grant.generation, info.generation);
    assert_eq!(grant.addresses, ["8.8.8.8:443"]);
    assert!(!grant.authorization.is_empty());

    let consumed: BrowserEgressDecision = proxy
        .call(
            methods::CONSUME,
            (BrowserEgressConsumeRequest {
                target: target.clone(),
                authorization: grant.authorization.clone(),
                address: grant.addresses[0].clone(),
            },),
        )
        .await
        .expect("TinySecurity consumes the exact one-use grant");
    assert!(consumed.allowed);

    let valid: BrowserEgressDecision = proxy
        .call(
            methods::VALIDATE,
            (BrowserEgressValidateRequest {
                authorization: grant.authorization.clone(),
            },),
        )
        .await
        .expect("TinySecurity validates the live tunnel lease");
    assert!(valid.allowed);

    let second_consume: BrowserEgressDecision = proxy
        .call(
            methods::CONSUME,
            (BrowserEgressConsumeRequest {
                target,
                authorization: grant.authorization,
                address: grant.addresses[0].clone(),
            },),
        )
        .await
        .expect("replay is a denial, not a transport error");
    assert!(!second_consume.allowed, "one-use grants cannot be replayed");
}

/// Exercises private-address rejection through the real TinyComputer module,
/// Chromium, the host callback, and a real TinySecurity cdylib. The fixture is
/// supplied by `scripts/ci/security-browser-egress-fixture.sh` on Linux.
#[tokio::test(flavor = "multi_thread")]
#[cfg(all(feature = "native-security-tests", target_os = "linux"))]
#[ignore = "requires local TinyComputer cdylib and Chromium; run the browser-egress fixture script"]
async fn loaded_tinycomputer_uses_tinysecurity_to_block_loopback_dials() {
    use crate::modules::security::native_tests::native_fixture_configured;
    use std::os::unix::fs::PermissionsExt;
    use tinycomputer_bus::DesktopResponse;

    let computer_artifact = std::path::PathBuf::from(
        std::env::var_os("OPENHUMAN_TEST_TINYCOMPUTER_MODULE")
            .expect("fixture script builds the TinyComputer module"),
    );
    let computer_fixtures = tempfile::tempdir().expect("private TinyComputer fixture directory");
    std::fs::set_permissions(
        computer_fixtures.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .expect("restrict TinyComputer fixture directory");
    let file_name = computer_artifact.file_name().unwrap();
    let computer_module = computer_fixtures.path().join(file_name);
    std::fs::copy(&computer_artifact, &computer_module).expect("stage TinyComputer module");
    let digest = tinybus::module::sha256_file(&computer_module).expect("TinyComputer digest");
    std::fs::write(
        computer_fixtures.path().join("modules.toml"),
        format!("{file_name:?} = {digest:?}\n"),
    )
    .expect("pin staged TinyComputer fixture");
    let fixture = native_fixture_configured(false, |_| {}).await;
    install(&fixture.client).await.unwrap();
    let owner = tinysecurity_bus::SubjectScope {
        user: "native-test-user".into(),
        workspace: "native-test-workspace".into(),
        agent: "native-test-agent".into(),
    };
    let policy = tinysecurity_bus::EgressPolicy {
        owner,
        ceiling: tinysecurity_bus::AccessTier::Full,
        privacy: tinysecurity_bus::PrivacyMode::Standard,
        destinations: Vec::new(),
        data_categories: vec![tinysecurity_bus::DataCategory::OpaqueBrowserTraffic],
        local_capabilities: Vec::new(),
        validity_ms: 30_000,
        open_public: true,
        preflight_transports: vec![tinysecurity_bus::EgressTransport::Browser],
    };
    let policy_id: tinysecurity_bus::EgressPolicyId = fixture
        .proxy
        .call_confidential(
            tinysecurity_bus::names::methods::REGISTER_EGRESS_POLICY,
            (policy,),
        )
        .await
        .expect("trusted host registers browser policy");
    let info: tinysecurity_bus::PolicyInfo = fixture
        .proxy
        .call(tinysecurity_bus::names::methods::POLICY_INFO, ())
        .await
        .expect("read actual module generation");
    crate::modules::security::egress::remember_browser_dial_binding_for_test(
        policy_id.clone(),
        info.generation,
        fixture.proxy.clone(),
    );

    fixture
        .host
        .load_file_with_config(
            computer_module,
            serde_json::json!({
                "browser": {
                    "egress": {
                        "policy_id": policy_id.0,
                        "generation": info.generation
                    }
                }
            }),
        )
        .expect("load the actual TinyComputer cdylib with the TinySecurity policy");
    let proxy = fixture
        .client
        .proxy(
            tinycomputer_bus::names::INTERFACE,
            tinycomputer_bus::names::OBJECT_PATH,
            tinycomputer_bus::names::INTERFACE,
        )
        .unwrap()
        .with_timeout(std::time::Duration::from_secs(30));

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind local denial fixture");
    let port = listener.local_addr().unwrap().port();
    let mut private_target = target(&policy_id.0, info.generation, "127.0.0.1", port);
    private_target.scheme = "http".into();
    let direct = crate::modules::security::egress::authorize_browser_dial(private_target)
        .await
        .expect_err("TinySecurity must reject loopback before a browser dial");
    assert!(
        direct.contains("TinySecurity denied browser dial"),
        "{direct}"
    );

    let opened: DesktopResponse = proxy
        .call(
            tinycomputer_bus::browser::names::methods::OPEN_SESSION,
            (serde_json::json!({}),),
        )
        .await
        .expect("real TinyComputer opens a Chromium session");
    assert!(opened.ok, "browser session failed: {:?}", opened.error);
    let session = opened.data.as_ref().unwrap()["id"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    let navigated: DesktopResponse = proxy
        .call(
            tinycomputer_bus::browser::names::methods::NAVIGATE,
            (serde_json::json!({
                "session": session,
                "url": format!("http://127.0.0.1:{port}/private")
            }),),
        )
        .await
        .expect("TinyComputer reports the denied browser navigation");
    assert!(
        !navigated.ok,
        "navigation to the private listener must be denied: {:?}",
        navigated.data
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), listener.accept())
            .await
            .is_err(),
        "the denied browser request must never reach the loopback listener"
    );
    let closed: DesktopResponse = proxy
        .call(
            tinycomputer_bus::browser::names::methods::CLOSE_SESSION,
            (serde_json::json!({ "session": session }),),
        )
        .await
        .expect("close the Chromium session");
    assert!(closed.ok, "browser cleanup failed: {:?}", closed.error);
}

/// Primes the real TinySecurity URL guard with a public DNS answer, then loads
/// Chromium through TinyComputer after the same controlled name changes to
/// loopback. The module must reject the connection-time answer before dial.
#[tokio::test(flavor = "multi_thread")]
#[cfg(all(feature = "native-security-tests", target_os = "linux"))]
#[ignore = "requires local TinySecurity/TinyComputer cdylibs and Chromium; run the browser-egress fixture script"]
async fn loaded_tinycomputer_rejects_public_to_loopback_dns_rebinding() {
    use crate::modules::security::native_tests::native_fixture_configured;
    use std::os::unix::fs::PermissionsExt;
    use tinycomputer_bus::DesktopResponse;
    use tinysecurity_bus::{
        AccessTier, DataCategory, EgressPolicy, EgressTransport, PrivacyMode, ScopedEgressRequest,
        SubjectScope,
    };

    let computer_artifact = std::path::PathBuf::from(
        std::env::var_os("OPENHUMAN_TEST_TINYCOMPUTER_MODULE")
            .expect("fixture script builds the TinyComputer module"),
    );
    let computer_fixtures = tempfile::tempdir().expect("private TinyComputer fixture directory");
    std::fs::set_permissions(
        computer_fixtures.path(),
        std::fs::Permissions::from_mode(0o700),
    )
    .expect("restrict TinyComputer fixture directory");
    let file_name = computer_artifact.file_name().unwrap();
    let computer_module = computer_fixtures.path().join(file_name);
    std::fs::copy(&computer_artifact, &computer_module).expect("stage TinyComputer module");
    let digest = tinybus::module::sha256_file(&computer_module).expect("TinyComputer digest");
    std::fs::write(
        computer_fixtures.path().join("modules.toml"),
        format!("{file_name:?} = {digest:?}\n"),
    )
    .expect("pin staged TinyComputer fixture");

    let fixture = native_fixture_configured(false, |_| {}).await;
    install(&fixture.client).await.unwrap();
    let owner = SubjectScope {
        user: "native-test-user".into(),
        workspace: "native-test-workspace".into(),
        agent: "native-test-agent".into(),
    };
    let policy = EgressPolicy {
        owner: owner.clone(),
        ceiling: AccessTier::Full,
        privacy: PrivacyMode::Standard,
        destinations: Vec::new(),
        data_categories: vec![DataCategory::OpaqueBrowserTraffic],
        local_capabilities: Vec::new(),
        validity_ms: 30_000,
        open_public: true,
        preflight_transports: vec![EgressTransport::Browser],
    };
    let policy_id: tinysecurity_bus::EgressPolicyId = fixture
        .proxy
        .call_confidential(
            tinysecurity_bus::names::methods::REGISTER_EGRESS_POLICY,
            (policy,),
        )
        .await
        .expect("trusted host registers browser policy");
    let info: tinysecurity_bus::PolicyInfo = fixture
        .proxy
        .call(tinysecurity_bus::names::methods::POLICY_INFO, ())
        .await
        .expect("read actual module generation");
    crate::modules::security::egress::remember_browser_dial_binding_for_test(
        policy_id.clone(),
        info.generation,
        fixture.proxy.clone(),
    );

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind loopback rebind target");
    let port = listener.local_addr().unwrap().port();
    let preflight = ScopedEgressRequest {
        policy_id: policy_id.clone(),
        subject: owner,
        tier: AccessTier::Full,
        url: format!("http://rebind.test:{port}/private"),
        method: "GET".into(),
        category: DataCategory::OpaqueBrowserTraffic,
        payload: String::new(),
        headers: Vec::new(),
        transport: EgressTransport::Browser,
        local_capability: None,
        transformation: None,
    };
    let preflight_response: tinysecurity_bus::ScopedEgressResponse = fixture
        .proxy
        .call_confidential(
            tinysecurity_bus::names::methods::SCOPED_URL_GUARD,
            (preflight.clone(),),
        )
        .await
        .expect("TinySecurity answers the public-name preflight");
    assert_eq!(preflight_response.generation, info.generation);
    assert!(
        preflight_response.denial.is_none(),
        "the first controlled DNS answer is public and should pass preflight"
    );
    let mut metadata_preflight = preflight.clone();
    metadata_preflight.url = "http://metadata.test/latest/meta-data/".into();
    let metadata_preflight_response: tinysecurity_bus::ScopedEgressResponse = fixture
        .proxy
        .call_confidential(
            tinysecurity_bus::names::methods::SCOPED_URL_GUARD,
            (metadata_preflight,),
        )
        .await
        .expect("TinySecurity answers the public metadata-name preflight");
    assert_eq!(metadata_preflight_response.generation, info.generation);
    assert!(
        metadata_preflight_response.denial.is_none(),
        "the first controlled metadata DNS answer is public and should pass preflight"
    );
    let mut metadata_target = target(&policy_id.0, info.generation, "metadata.test", 80);
    metadata_target.scheme = "http".into();
    let metadata_dial = crate::modules::security::egress::authorize_browser_dial(metadata_target)
        .await
        .expect_err("TinySecurity must reject a connection-time metadata address");
    assert!(
        metadata_dial.contains("TinySecurity denied browser dial"),
        "{metadata_dial}"
    );

    fixture
        .host
        .load_file_with_config(
            computer_module,
            serde_json::json!({
                "browser": {
                    "egress": {
                        "policy_id": policy_id.0,
                        "generation": info.generation
                    }
                }
            }),
        )
        .expect("load the actual TinyComputer cdylib with the TinySecurity policy");
    let proxy = fixture
        .client
        .proxy(
            tinycomputer_bus::names::INTERFACE,
            tinycomputer_bus::names::OBJECT_PATH,
            tinycomputer_bus::names::INTERFACE,
        )
        .unwrap()
        .with_timeout(std::time::Duration::from_secs(30));
    let opened: DesktopResponse = proxy
        .call(
            tinycomputer_bus::browser::names::methods::OPEN_SESSION,
            (serde_json::json!({}),),
        )
        .await
        .expect("real TinyComputer opens a Chromium session");
    assert!(opened.ok, "browser session failed: {:?}", opened.error);
    let session = opened.data.as_ref().unwrap()["id"]
        .as_str()
        .expect("browser session id")
        .to_owned();
    let navigated: DesktopResponse = proxy
        .call(
            tinycomputer_bus::browser::names::methods::NAVIGATE,
            (serde_json::json!({
                "session": session,
                "url": format!("http://rebind.test:{port}/private")
            }),),
        )
        .await
        .expect("TinyComputer reports the connection-time DNS denial");
    assert!(
        !navigated.ok,
        "connection-time loopback DNS answer must be denied: {:?}",
        navigated.data
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), listener.accept())
            .await
            .is_err(),
        "rebinding request must never reach the loopback listener"
    );
    let metadata_navigation: DesktopResponse = proxy
        .call(
            tinycomputer_bus::browser::names::methods::NAVIGATE,
            (serde_json::json!({
                "session": session,
                "url": "http://metadata.test/latest/meta-data/"
            }),),
        )
        .await
        .expect("TinyComputer reports the connection-time metadata denial");
    assert!(
        !metadata_navigation.ok,
        "connection-time metadata DNS answer must be denied: {:?}",
        metadata_navigation.data
    );
    let closed: DesktopResponse = proxy
        .call(
            tinycomputer_bus::browser::names::methods::CLOSE_SESSION,
            (serde_json::json!({ "session": session }),),
        )
        .await
        .expect("close the Chromium session");
    assert!(closed.ok, "browser cleanup failed: {:?}", closed.error);
}
