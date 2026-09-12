//! Tests for the host half of the TinyMemory module's callbacks.
//!
//! Two things are worth asserting here and a third is not.
//!
//! Worth asserting: that the interfaces are actually *reachable* — served at
//! the path the module proxies, under the name the module resolves — and that
//! each Composio method reaches the host's own Composio client rather than
//! answering from something local. Both are failures that no compiler catches:
//! a name that is never claimed surfaces as `NameHasNoOwner` at the first sync
//! tick, in the field, and a method that quietly answers "nothing here" is the
//! looks-empty-rather-than-broken outcome this whole seam exists to prevent.
//!
//! Not worth asserting: what Composio replies. Every method below is driven
//! against a workspace with no session and no stored key, so the interesting
//! answer is always the *shape* of the failure — which factory it came from —
//! never a connection list. The round trips themselves are covered where they
//! can be honest, in `tinymemory`'s own module E2E against a real module.

use super::{
    serve_interfaces, ComposioCallbacks, EmbeddingCallbacks, CHAT_NAME, CHAT_PATH, COMPOSIO_NAME,
    COMPOSIO_PATH, EMBEDDING_NAME, EMBEDDING_PATH, RUNTIME_NAME, RUNTIME_PATH,
};
use crate::openhuman::config::Config;
use crate::openhuman::integrations::composio::client::create_composio_client;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;
use tinybus::broker::Broker;
use tinybus::service::Interface;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, MemberName};

/// The host error name every method in this file fails under.
const HOST_ERROR: &str = "ai.tinyhumans.tinymemory.Error.Host";

/// A config scoped to `dir`, so nothing here reads the developer's own install.
///
/// Both paths matter and for different reasons: `config_path` is what the
/// callbacks re-read on every call, and it is also what the credential store
/// resolves against (its *parent*, not the workspace). Setting only one gives a
/// test that looks hermetic and answers from the real keychain.
fn scoped_config(dir: &Path) -> Arc<Config> {
    let mut config = Config::default();
    config.workspace_dir = dir.join("workspace");
    config.config_path = dir.join("config.toml");
    Arc::new(config)
}

/// Dispatch `member` against `callbacks` the way the connection would.
async fn call(
    callbacks: &ComposioCallbacks,
    member: &str,
    args: serde_json::Value,
) -> tinybus::Result<serde_json::Value> {
    callbacks
        .call(&MemberName::new(member).expect("member name"), args)
        .await
}

#[test]
fn the_composio_interface_is_the_engines_contract_methods_in_order() {
    // The engine's `ComposioHost` trait has exactly these, and the module side
    // proxies each by name. A method renamed on this side is not a compile
    // error over there — it is a `MethodFailed: UnknownMethod` the first time a
    // sync run reaches for it.
    //
    // `SessionBearer` joined in tinymemory v1.8.0. It is what lets the proxied
    // ("backend") branch of `composio_config` resolve a credential inside a
    // loaded module, which is what unblocked routing this host's Composio sync
    // through the driver at all.
    let dir = tempfile::tempdir().expect("tempdir");
    let callbacks = ComposioCallbacks(scoped_config(dir.path()));

    assert_eq!(callbacks.name().as_str(), COMPOSIO_NAME);
    let members: Vec<String> = callbacks
        .members()
        .iter()
        .map(|member| member.as_str().to_string())
        .collect();
    assert_eq!(
        members,
        [
            "ListConnections",
            "Execute",
            "ApiKey",
            "SessionBearer",
            "IsAvailable"
        ]
    );
}

#[tokio::test]
async fn install_claims_composios_name_beside_the_other_three() {
    // `install` is latched process-wide, so this drives the function it
    // delegates to. The assertion is name resolution, not behaviour: a bogus
    // member reaching `UnknownMethod` proves the name is owned *and* an object
    // is exported at that path, which together are the whole precondition for
    // the module ever getting a real answer.
    let dir = tempfile::tempdir().expect("tempdir");
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await.expect("service transport"))
        .await
        .expect("service connection");
    serve_interfaces(&service, scoped_config(dir.path()))
        .await
        .expect("serve every host interface");

    let client = Connection::connect(bus.connect().await.expect("client transport"))
        .await
        .expect("client connection");

    for (name, path) in [
        (EMBEDDING_NAME, EMBEDDING_PATH),
        (CHAT_NAME, CHAT_PATH),
        (COMPOSIO_NAME, COMPOSIO_PATH),
        (RUNTIME_NAME, RUNTIME_PATH),
    ] {
        let proxy = client.proxy(name, path, name).expect("proxy");
        let error = proxy
            .call::<serde_json::Value>("NoSuchMember", ())
            .await
            .expect_err("a member that does not exist cannot succeed");
        assert_eq!(
            error.wire_name(),
            tinybus::Error::UNKNOWN_METHOD,
            "{name} is not served at {path}: {error}"
        );
    }
}

#[tokio::test]
async fn composio_answers_over_the_bus_and_not_only_in_process() {
    // The narrow version of the test above: one real method, end to end,
    // through a proxy addressed exactly as the module addresses it.
    let dir = tempfile::tempdir().expect("tempdir");
    let config = scoped_config(dir.path());
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await.expect("service transport"))
        .await
        .expect("service connection");
    serve_interfaces(&service, Arc::clone(&config))
        .await
        .expect("serve every host interface");

    let client = Connection::connect(bus.connect().await.expect("client transport"))
        .await
        .expect("client connection");
    let proxy = client
        .proxy(COMPOSIO_NAME, COMPOSIO_PATH, COMPOSIO_NAME)
        .expect("proxy");

    let available: bool = proxy.call("IsAvailable", ()).await.expect("IsAvailable");
    assert_eq!(available, create_composio_client(config.as_ref()).is_ok());
}

#[tokio::test]
async fn is_available_is_the_hosts_own_factory_probe() {
    // Equality with the factory rather than a hardcoded `false`: the point is
    // delegation, and a literal would pass just as well against a stub that
    // always said no — which is exactly the bug worth catching.
    let dir = tempfile::tempdir().expect("tempdir");
    let config = scoped_config(dir.path());
    let callbacks = ComposioCallbacks(Arc::clone(&config));

    let answer = call(&callbacks, "IsAvailable", json!([]))
        .await
        .expect("IsAvailable never fails");
    assert_eq!(
        answer,
        json!(create_composio_client(config.as_ref()).is_ok())
    );
}

#[tokio::test]
async fn api_key_is_the_hosts_own_credential_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = scoped_config(dir.path());
    let callbacks = ComposioCallbacks(Arc::clone(&config));

    let expected = crate::openhuman::security::credentials::get_composio_api_key(config.as_ref())
        .ok()
        .flatten();
    let answer = call(&callbacks, "ApiKey", json!([]))
        .await
        .expect("ApiKey never fails");
    assert_eq!(answer, json!(expected));
}

/// `api_key` falls back to `config.composio.api_key` when the credential store
/// holds nothing, keeping parity with `create_composio_client`'s fallback so
/// `is_available` and `api_key` cannot disagree for a direct-mode user.
#[tokio::test]
async fn api_key_falls_back_to_config_key_when_credential_store_is_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = dir.path().join("workspace");
    config.config_path = dir.path().join("config.toml");
    config.composio.api_key = Some("direct-key-from-config".to_owned());

    // Persisted, not only constructed. `api_key` answers from a LIVE re-read so
    // that a key the user changes mid-session is followed, and falls back to the
    // install-time snapshot only when that read fails. A config that exists only
    // in memory is not the case this test means to cover — the re-read would
    // succeed against an empty file and quietly answer `None`, which is the
    // fallback never firing rather than the fallback working.
    std::fs::create_dir_all(dir.path()).expect("config dir");
    std::fs::write(
        &config.config_path,
        toml::to_string(&config).expect("serialise config"),
    )
    .expect("write config");

    let callbacks = ComposioCallbacks(Arc::new(config));

    let answer = call(&callbacks, "ApiKey", json!([]))
        .await
        .expect("ApiKey never fails");
    assert_eq!(answer, json!("direct-key-from-config"));
}

#[tokio::test]
async fn a_probe_survives_a_config_it_cannot_read() {
    // The two probes have nowhere to put "could not tell", and answering "no"
    // on an unreadable config would stop a sync run that was working a minute
    // ago. Point them at a path that cannot resolve and they must still answer.
    let callbacks = ComposioCallbacks(scoped_config(Path::new("/nonexistent/openhuman")));

    assert!(call(&callbacks, "IsAvailable", json!([])).await.is_ok());
    assert!(call(&callbacks, "ApiKey", json!([])).await.is_ok());
}

#[tokio::test]
async fn list_connections_stops_inside_the_hosts_composio_stack() {
    // With no session and no key there is no client to build, so the failure
    // must be the host's — carrying this file's error name, not a bus-level
    // one. A stub that answered `Ok(vec![])` would be indistinguishable from a
    // user with no connections, which is the failure mode being designed out.
    let dir = tempfile::tempdir().expect("tempdir");
    let callbacks = ComposioCallbacks(scoped_config(dir.path()));

    let error = call(&callbacks, "ListConnections", json!([]))
        .await
        .expect_err("no client can be built for an empty workspace");
    assert_eq!(error.wire_name(), HOST_ERROR);
}

#[tokio::test]
async fn execute_takes_its_four_arguments_in_the_order_the_engine_sends_them() {
    // Positional arguments are the part of a bus contract that has no compiler
    // behind it. The pair below is the cheapest honest check: the right arity
    // gets far enough to fail inside the host, the wrong one is refused before
    // any host code runs.
    let dir = tempfile::tempdir().expect("tempdir");
    let callbacks = ComposioCallbacks(scoped_config(dir.path()));

    let error = call(
        &callbacks,
        "Execute",
        json!(["GMAIL_FETCH_EMAILS", { "max_results": 1 }, "default", null]),
    )
    .await
    .expect_err("no client can be built for an empty workspace");
    assert_eq!(
        error.wire_name(),
        HOST_ERROR,
        "a well-formed call must reach the host's client factory: {error}"
    );

    let error = call(&callbacks, "Execute", json!([]))
        .await
        .expect_err("Execute takes four arguments");
    assert_eq!(
        error.wire_name(),
        "ai.tinyhumans.tinybus.Error.BadArguments",
        "a malformed call must be refused at decode, not inside the host: {error}"
    );
}

#[tokio::test]
async fn execute_forwards_an_absent_connection_pin_as_absent() {
    // `connection_id` is `Option`, and JSON `null` is how the module spells
    // "unpinned". Decoding it as a missing argument instead would make every
    // unpinned execute fail arity rather than run.
    let dir = tempfile::tempdir().expect("tempdir");
    let callbacks = ComposioCallbacks(scoped_config(dir.path()));

    for pin in [json!(null), json!("ca_abc123")] {
        let error = call(
            &callbacks,
            "Execute",
            json!(["GMAIL_FETCH_EMAILS", null, "default", pin]),
        )
        .await
        .expect_err("no client can be built for an empty workspace");
        assert_eq!(error.wire_name(), HOST_ERROR, "{error}");
    }
}

#[tokio::test]
async fn embed_takes_its_four_arguments_in_the_order_the_module_sends_them() {
    // `Embed` is `(provider, model, dimensions, texts)`. The module shipped
    // sending three — `(model, dimensions, texts)` — so `dimensions` landed in
    // `model` and every call was refused at decode with "invalid type:
    // integer, expected a string"; nothing ingested in module mode got a vector
    // (#5820). Same cheapest-honest-check as `Execute` above: the right arity
    // gets as far as the host's provider factory, the wrong one never does.
    let dir = tempfile::tempdir().expect("tempdir");
    let callbacks = EmbeddingCallbacks(scoped_config(dir.path()));

    let error = callbacks
        .call(
            &MemberName::new("Embed").expect("member name"),
            json!(["no-such-provider", "some-model", 4, ["alpha"]]),
        )
        .await
        .expect_err("an unknown provider slug fails inside the host's factory");
    assert_eq!(
        error.wire_name(),
        HOST_ERROR,
        "a well-formed call must reach the host's provider factory: {error}"
    );

    let error = callbacks
        .call(
            &MemberName::new("Embed").expect("member name"),
            json!(["some-model", 4, ["alpha"]]),
        )
        .await
        .expect_err("the module's old three-argument form is malformed");
    assert_eq!(
        error.wire_name(),
        "ai.tinyhumans.tinybus.Error.BadArguments",
        "a malformed call must be refused at decode, not inside the host: {error}"
    );
}

// ── The summarization role resolves through the consent ladder ───────────────

/// A model the role factory can hand back through the test override, so the
/// tests below prove the ladder refuses *despite* a resolvable factory.
struct StubModel;

#[async_trait::async_trait]
impl tinyinference::model::ChatModel<()> for StubModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: tinyinference::model::ModelRequest,
    ) -> tinyinference::Result<tinyinference::model::ModelResponse> {
        Ok(tinyinference::model::ModelResponse::assistant("stub"))
    }
}

/// With local AI off and no cloud opt-in, a module-side `"summarization"`
/// chat call must be refused by the ladder — even while the role factory
/// demonstrably CAN hand back a model (the override is installed). This is
/// the consent hole the ladder plugs: before it, the blind role factory
/// resolved `"summarization"` to the configured cloud provider regardless of
/// `memory_tree.cloud_summarization_opt_in`.
#[test]
fn summarization_role_is_refused_without_local_ai_or_cloud_opt_in() {
    let _guard =
        crate::openhuman::inference::provider::factory::test_provider_override::install_model(
            std::sync::Arc::new(StubModel),
        );
    let mut config = Config::default();
    config.local_ai.runtime_enabled = false;
    config.memory_tree.cloud_summarization_opt_in = false;

    let error = match super::resolve_chat_model("summarization", &config) {
        Err(error) => error,
        Ok(_) => panic!("no local AI and no cloud opt-in must refuse the fold"),
    };
    assert!(
        error.to_string().contains("cloud_summarization_opt_in"),
        "the refusal must name the setting that unlocks it: {error}"
    );
}

/// Any other role keeps the role factory unchanged — the ladder is scoped to
/// the summarization role, not to the seam.
#[test]
fn non_summarization_roles_keep_the_role_factory() {
    let _guard =
        crate::openhuman::inference::provider::factory::test_provider_override::install_model(
            std::sync::Arc::new(StubModel),
        );
    let config = Config::default();
    super::resolve_chat_model("chat", &config)
        .expect("a non-summarization role resolves through the factory (test override)");
}
