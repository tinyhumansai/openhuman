use super::*;
use tinybus::broker::Broker;
use tinybus::module::ModuleHost;

/// Every registry id this crate can link, in registration order.
const ALL_IDS: [&str; 13] = [
    "tinybox",
    "tinychannels",
    "tinycomputer",
    "tinyconnectors",
    "tinydocs",
    "tinyeventqueues",
    "tinyflows",
    "tinyhosts",
    "tinyjuice",
    "tinymcp",
    "tinysearch",
    "tinyvoice",
    "tinywallet",
];

fn enabled_ids() -> Vec<&'static str> {
    [
        (cfg!(feature = "tinybox"), "tinybox"),
        (cfg!(feature = "tinychannels"), "tinychannels"),
        (cfg!(feature = "tinycomputer"), "tinycomputer"),
        (cfg!(feature = "tinyconnectors"), "tinyconnectors"),
        (cfg!(feature = "tinydocs"), "tinydocs"),
        (cfg!(feature = "tinyeventqueues"), "tinyeventqueues"),
        (cfg!(feature = "tinyflows"), "tinyflows"),
        (cfg!(feature = "tinyhosts"), "tinyhosts"),
        (cfg!(feature = "tinyjuice"), "tinyjuice"),
        (cfg!(feature = "tinymcp"), "tinymcp"),
        (cfg!(feature = "tinysearch"), "tinysearch"),
        (cfg!(feature = "tinyvoice"), "tinyvoice"),
        (cfg!(feature = "tinywallet"), "tinywallet"),
    ]
    .into_iter()
    .filter_map(|(on, id)| on.then_some(id))
    .collect()
}

#[test]
fn every_enabled_feature_registers_exactly_its_own_id() {
    let ids: Vec<&str> = linked_modules().iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, enabled_ids());
    assert!(ids.iter().all(|id| ALL_IDS.contains(id)));
}

#[test]
fn install_is_idempotent() {
    install();
    install();
}

#[test]
fn install_makes_every_enabled_factory_visible_to_the_core() {
    install();
    for id in enabled_ids() {
        assert!(
            openhuman_rpc::embed::modules::is_linked_registered(id),
            "{id} was not registered"
        );
    }
}

/// Each factory yields a module whose manifest names its own registry id and
/// whose descriptor passes admission: the same check a released library gets,
/// with the executable's digest as attestation.
#[test]
fn every_enabled_factory_builds_a_module_that_names_its_own_id() {
    for (id, factory) in linked_modules() {
        let module = factory().unwrap_or_else(|error| panic!("{id}: factory failed: {error}"));
        assert!(
            module.manifest.module.name.starts_with(id),
            "{id}: manifest names {}",
            module.manifest.module.name
        );
        assert!(
            module
                .manifest
                .bus_name
                .as_str()
                .starts_with("ai.tinyhumans"),
            "{id}: bus name {}",
            module.manifest.bus_name.as_str()
        );
    }
}

/// Linking several modules must not make them share state: each is admitted by
/// its own host, and a module whose required interfaces the bare host does not
/// serve is refused for exactly that reason, never for a duplicate symbol or a
/// descriptor fault.
#[tokio::test]
async fn every_enabled_module_is_admitted_or_refused_only_for_its_host_interfaces() {
    for (id, factory) in linked_modules() {
        let module = factory().unwrap();
        let host = ModuleHost::new(Broker::new());
        match host.attach_linked_with_config(module, serde_json::json!({})) {
            Ok(info) => assert!(info.manifest.module.name.starts_with(id)),
            Err(error) => {
                let message = error.to_string();
                assert!(
                    message.contains("required interface has no provider"),
                    "{id}: unexpected admission refusal: {message}"
                );
            }
        }
    }
}

/// Two modules can sit in one host: the descriptors and manifests of different
/// modules do not collide on a bus name or object path.
#[test]
fn enabled_modules_claim_distinct_bus_names_and_object_paths() {
    let mut bus_names = std::collections::BTreeSet::new();
    let mut paths = std::collections::BTreeSet::new();
    for (id, factory) in linked_modules() {
        let module = factory().unwrap();
        assert!(
            bus_names.insert(module.manifest.bus_name.as_str().to_string()),
            "{id}: duplicate bus name"
        );
        assert!(
            paths.insert(module.manifest.object_path.as_str().to_string()),
            "{id}: duplicate object path"
        );
    }
}

/// A linked module is attested by the executable's digest. Every module this
/// build links that the bare host admits must be attested, or its confidential
/// members could never be delivered in a linked build.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_linked_modules_are_attested_as_host_code() {
    use tinybus::module::ModuleHost;
    use tinybus::transport::memory::MemoryBus;
    use tinybus::Connection;

    for (id, factory) in linked_modules() {
        let bus = MemoryBus::new();
        let broker = Broker::new();
        let broker_task = broker.spawn(bus.clone());
        let host = ModuleHost::new(broker);
        let Ok(info) = host.attach_linked_with_config(factory().unwrap(), serde_json::json!({}))
        else {
            broker_task.abort();
            continue;
        };
        let client = Connection::connect(bus.connect().await.unwrap())
            .await
            .unwrap();
        let attestation = client
            .attestation(info.manifest.bus_name.clone())
            .await
            .unwrap();
        assert!(attestation.is_some(), "{id}: not attested as host code");
        broker_task.abort();
    }
}

/// tinywallet's attested members work linked: the module signs a transaction
/// confidentially, which an unattested module would refuse. This is why
/// tinywallet is linkable rather than `source`-only.
#[cfg(feature = "tinywallet")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linked_tinywallet_accepts_confidential_delivery() {
    use tinybus::module::ModuleHost;
    use tinybus::transport::memory::MemoryBus;
    use tinybus::Connection;

    let bus = MemoryBus::new();
    let broker = Broker::new();
    let broker_task = broker.spawn(bus.clone());
    let host = ModuleHost::new(broker);
    let module = tinywallet_module::linked_module().unwrap();
    let info = host
        .attach_linked_with_config(module, serde_json::json!({}))
        .expect("linked tinywallet is admitted");
    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = client
        .proxy(
            tinywallet_module::BUS_NAME,
            tinywallet_module::OBJECT_PATH,
            tinywallet_module::BUS_NAME,
        )
        .unwrap();
    assert_eq!(info.manifest.bus_name.as_str(), tinywallet_module::BUS_NAME);
    let request = serde_json::json!({
        "secret": {
            "mnemonic": "abandon abandon abandon abandon abandon abandon \
                         abandon abandon abandon abandon abandon about",
            "derivation_path": "m/44'/60'/0'/0/0",
            "chain": "evm",
        },
        "transaction": {
            "kind": "evm",
            "to": "0x3535353535353535353535353535353535353535",
            "value_wei": "1000000000000000000",
            "data_hex": "0x",
            "nonce": 9,
            "gas_limit": 21_000,
            "gas_price_wei": "20000000000",
            "chain_id": 1,
        },
    });
    let mut delivered = false;
    for _ in 0..50 {
        if proxy
            .call_confidential::<serde_json::Value>("SignTransaction", (request.clone(),))
            .await
            .is_ok()
        {
            delivered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    broker_task.abort();
    assert!(
        delivered,
        "linked tinywallet must accept confidential delivery"
    );
}
