use super::*;
use async_trait::async_trait;
use serde_json::json;
use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::{BusName, Connection};
use tinystoragedrivers::secrets::{DerivedKeys, KeyProvider};
use tinystoragedrivers::{MemoryStorage, Scope, StorageError};
use tinystoragedrivers_bus::{SecretNameRequest, SecretValue, SetSecretRequest};
use zeroize::Zeroizing;

use super::super::storage_grants::StorageGrant;
use super::super::storage_host::{module_scope, CallerDirectory, Environment, StorageHost};
use crate::storage::StorageBackend;

use super::TEST_TURN as TURN;

const MODULE: &str = "tinymcp";

struct Directory(BusName);

#[async_trait]
impl CallerDirectory for Directory {
    async fn module_for(&self, sender: &BusName) -> Option<String> {
        (sender == &self.0).then(|| MODULE.to_string())
    }
}

struct Env(Arc<dyn StorageBackend>);

#[async_trait]
impl Environment for Env {
    fn backend(&self) -> Option<Arc<dyn StorageBackend>> {
        Some(Arc::clone(&self.0))
    }
    fn module_scope(&self, module_id: &str) -> Result<Scope, StorageError> {
        module_scope(None, module_id)
    }
    fn grant(&self, module_id: &str) -> Option<StorageGrant> {
        super::super::storage_grants::declared(module_id)
    }
    fn keys(&self) -> Result<Arc<dyn KeyProvider>, StorageError> {
        Ok(Arc::new(DerivedKeys::new(Zeroizing::new([9u8; 32]))))
    }
    async fn admit_write(&self, _scope: &Scope) -> Result<(), StorageError> {
        Ok(())
    }
}

struct Bus {
    caller: Connection,
    _modules: ModuleHost,
}

/// A bus whose storage host is the linked module, over `backend`, with one
/// caller that the directory knows as `tinymcp`.
async fn boot(backend: Arc<dyn StorageBackend>) -> Bus {
    let transport = MemoryBus::new();
    let broker = Broker::new();
    broker.spawn(transport.clone());
    let modules = ModuleHost::new(broker);
    let host_connection = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let directory = Directory(caller.unique_name().unwrap());
    let host = Arc::new(StorageHost::new(
        Arc::new(directory),
        Arc::new(Env(backend)),
        Arc::new(NoAudit),
    ));
    attach(&modules, &host_connection, host).await.unwrap();
    Bus {
        caller,
        _modules: modules,
    }
}

struct NoAudit;
impl super::super::storage_host::AuditSink for NoAudit {
    fn record(&self, _record: &super::super::storage_host::AuditRecord) {}
}

impl Bus {
    fn object(&self, name: &str) -> tinybus::Proxy {
        self.caller
            .proxy(
                STORAGE_INTERFACE,
                format!("/ai/tinyhumans/tinystorage/{name}"),
                STORAGE_INTERFACE,
            )
            .unwrap()
    }

    async fn set_secret(&self, name: &str, value: &[u8]) -> tinybus::Result<()> {
        self.object("Secrets")
            .call_confidential(
                "Set",
                (SetSecretRequest {
                    name: name.into(),
                    value: value.to_vec(),
                },),
            )
            .await
    }

    async fn secret(&self, name: &str) -> tinybus::Result<Option<Vec<u8>>> {
        let reply: SecretValue = self
            .object("Secrets")
            .call_confidential("Get", (SecretNameRequest { name: name.into() },))
            .await?;
        Ok(Some(reply.value))
    }

    async fn put_server(&self, id: &str) -> tinybus::Result<()> {
        let _: serde_json::Value = self
            .object("Documents")
            .call(
                "Put",
                (json!({
                    "collection": "mcp.servers",
                    "document_id": id,
                    "document_json": json!({"qualified_name": id}).to_string(),
                    "precondition": {"kind": "none"},
                }),),
            )
            .await?;
        Ok(())
    }

    async fn server(&self, id: &str) -> tinybus::Result<serde_json::Value> {
        self.object("Documents")
            .call(
                "Get",
                (json!({"collection": "mcp.servers", "document_id": id}),),
            )
            .await
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_module_reaches_its_secrets_through_the_attested_linked_host() {
    let _turn = TURN.lock().await;
    let bus = boot(Arc::new(MemoryStorage::new())).await;
    bus.set_secret("mcp_client_env/github", b"token-1")
        .await
        .expect("the confidential route is delivered to the linked storage module");
    assert_eq!(
        bus.secret("mcp_client_env/github").await.unwrap(),
        Some(b"token-1".to_vec())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_secret_outside_the_modules_grant_is_still_refused() {
    let _turn = TURN.lock().await;
    let bus = boot(Arc::new(MemoryStorage::new())).await;
    let refused = bus.set_secret("someone_elses/key", b"x").await;
    assert!(
        refused.is_err(),
        "grant must be enforced behind attestation"
    );
    assert!(!matches!(refused, Err(tinybus::Error::NotAttested { .. })));
}

#[tokio::test(flavor = "multi_thread")]
async fn servers_and_env_secrets_survive_a_restart_of_the_bus_and_host() {
    let _turn = TURN.lock().await;
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    {
        let first = boot(Arc::clone(&backend)).await;
        first.put_server("io.example/files").await.unwrap();
        first
            .set_secret("mcp_client_env/io.example/files", b"{\"API_KEY\":\"k\"}")
            .await
            .unwrap();
    }
    // A new broker, module host and storage host over the same backend: what
    // a core restart does.
    let second = boot(backend).await;
    let server = second.server("io.example/files").await.unwrap();
    assert_eq!(server["document_id"], "io.example/files");
    assert_eq!(
        second
            .secret("mcp_client_env/io.example/files")
            .await
            .unwrap(),
        Some(b"{\"API_KEY\":\"k\"}".to_vec())
    );
}

/// Why the objects are served from a linked module at all: the same host
/// served from an ordinary connection is not an attested recipient, so the
/// broker refuses every secret before it reaches the host.
#[tokio::test(flavor = "multi_thread")]
async fn secrets_served_from_a_plain_connection_are_refused_not_attested() {
    let transport = MemoryBus::new();
    let broker = Broker::new();
    broker.spawn(transport.clone());
    let server = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let host = Arc::new(StorageHost::new(
        Arc::new(Directory(caller.unique_name().unwrap())),
        Arc::new(Env(Arc::new(MemoryStorage::new()))),
        Arc::new(NoAudit),
    ));
    serve_objects(&server, &host, None).await.unwrap();
    let bus = Bus {
        caller,
        _modules: ModuleHost::new(Broker::new()),
    };
    let refused = bus.set_secret("mcp_client_env/x", b"v").await;
    assert!(
        matches!(&refused, Err(tinybus::Error::MethodFailed { name, .. })
            if name == tinybus::Error::NOT_ATTESTED),
        "{refused:?}"
    );
}
