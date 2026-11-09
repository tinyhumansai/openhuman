//! Invocation-scoped, Documents-only storage callbacks for module imports.
//!
//! Unlike the long-lived storage host, this endpoint is bound to the exact
//! ready module instance that will make one import call. Its `Storage` is
//! supplied by the caller and therefore retains the captured profile and
//! import-claim conditions. The endpoint has no Secrets capability and does
//! not expose data operations on the other ports.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinybus::module::ModuleRuntime;
use tinybus::{BusName, ObjectPath};
use tinystoragedrivers_api::BlobReader;
use tinystoragedrivers_bus::{
    BlobCapabilities, BlobKeyRequest, BlobStreamHead, CapabilitiesReply, KvCapabilitiesReply,
    PutBlobStreamRequest, QueueFeaturesReply, StorageFailure, StorageKind, WatchChanged,
};
use tinystoragedrivers_busproxy::{
    Admission, HostFailure, Object, ServedStorage, StorageAddress, StorageCall, StorageEndpoint,
    StorageServer,
};
use tokio::sync::broadcast;

use super::storage_grants::{self, Plan, StorageGrant};
use crate::storage::Storage;

static NEXT_INVOCATION: AtomicU64 = AtomicU64::new(1);

/// The callback address fields used by the transport-neutral owner request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CallbackAddress {
    pub host: String,
    pub object_root: String,
}

/// Owns an invocation's temporary Documents endpoint and its registrations.
///
/// Keep this value alive for the entire module RPC. Dropping it closes dispatch
/// before the TSD guard unregisters the invocation's paths.
pub(crate) struct InvocationStorageBinding {
    address: CallbackAddress,
    active: Arc<AtomicBool>,
    served: Option<ServedStorage>,
}

impl InvocationStorageBinding {
    /// Serve the supplied claim-bound storage to this exact, currently ready
    /// module instance.
    pub(crate) async fn serve(
        runtime: &ModuleRuntime,
        _module_descriptor_name: &str,
        module_id: &str,
        storage: Storage,
    ) -> anyhow::Result<Self> {
        let expected_peer = runtime
            .host()
            .sender_for_module(module_id)
            .ok_or_else(|| anyhow::anyhow!("module is not a live admitted instance"))?;
        let connection = runtime.connection();
        let host = connection
            .unique_name()
            .ok_or_else(|| anyhow::anyhow!("module runtime connection has no unique bus name"))?;
        let serial = NEXT_INVOCATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map_err(|_| anyhow::anyhow!("invocation storage address space is exhausted"))?;
        let object_root = ObjectPath::new(format!(
            "{}/call-{serial}",
            tinystoragedrivers_busproxy::INVOCATION_OBJECT_ROOT
        ))?;
        let address = StorageAddress::try_new(host, object_root)?;
        let grant = storage_grants::declared(module_id)
            .ok_or_else(|| anyhow::anyhow!("module has no declared storage grant"))?;
        let active = Arc::new(AtomicBool::new(true));
        let endpoint = Arc::new(DocumentsOnlyEndpoint {
            expected_peer,
            active: Arc::clone(&active),
            grant,
            server: StorageServer::new(storage),
        });
        let served =
            tinystoragedrivers_busproxy::serve_at_address(connection, address.clone(), endpoint)
                .await?;
        Ok(Self {
            address: CallbackAddress {
                host: address.host().to_string(),
                object_root: address.object_root().to_string(),
            },
            active,
            served: Some(served),
        })
    }

    /// The address for the owner request. It conveys routing only; it carries
    /// no scope, grant, or write-condition authority.
    pub(crate) fn address(&self) -> &CallbackAddress {
        &self.address
    }

    /// Synchronously revoke calls and unregister the invocation paths.
    pub(crate) async fn close(mut self) -> anyhow::Result<()> {
        self.active.store(false, Ordering::Release);
        if let Some(served) = self.served.take() {
            served.shutdown().await?;
        }
        Ok(())
    }
}

impl Drop for InvocationStorageBinding {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
        // `ServedStorage::drop` schedules path removal on the module runtime.
        // Keeping it in this binding ties registration lifetime to the import
        // call, including error and cancellation paths.
        self.served.take();
    }
}

struct DocumentsOnlyEndpoint {
    expected_peer: BusName,
    active: Arc<AtomicBool>,
    grant: StorageGrant,
    server: StorageServer,
}

impl DocumentsOnlyEndpoint {
    fn authorize(&self, sender: Option<&BusName>) -> Result<(), HostFailure> {
        if !self.active.load(Ordering::Acquire) {
            return Err(HostFailure::Denied("storage-endpoint-closed"));
        }
        match sender {
            Some(sender) if sender == &self.expected_peer => Ok(()),
            Some(_) => Err(HostFailure::Denied("wrong-module-instance")),
            None => Err(HostFailure::Denied("unauthenticated-caller")),
        }
    }

    fn unsupported() -> HostFailure {
        HostFailure::Storage(StorageFailure {
            kind: StorageKind::Unsupported,
        })
    }

    fn denied() -> HostFailure {
        HostFailure::Denied("documents-only-invocation")
    }
}

#[async_trait]
impl StorageEndpoint for DocumentsOnlyEndpoint {
    async fn call(&self, call: StorageCall<'_>) -> Result<Value, HostFailure> {
        // Authenticate before even answering capability negotiation. An
        // address is routing information, never proof of module identity.
        self.authorize(call.sender)?;
        match (call.object, call.selector) {
            (Object::Documents, _) => {
                let plan = storage_grants::authorize(
                    &self.grant,
                    Object::Documents,
                    call.selector,
                    &call.request,
                )
                .map_err(|denied| HostFailure::Denied(denied.label()))?;
                match plan {
                    Plan::Answer(reply) => Ok(reply),
                    Plan::Forward { filter, .. } => {
                        if matches!(call.selector, "WatchOpen" | "WatchClose") {
                            return Err(Self::unsupported());
                        }
                        let mut reply = self
                            .server
                            .handle(
                                Admission::Private,
                                Object::Documents.as_str(),
                                call.selector,
                                call.request,
                            )
                            .await
                            .map_err(HostFailure::Storage)?;
                        if call.selector == "Capabilities" {
                            if let Some(capabilities) =
                                reply.get_mut("capabilities").and_then(Value::as_array_mut)
                            {
                                capabilities
                                    .retain(|capability| capability.as_str() != Some("watch"));
                            }
                        }
                        Ok(storage_grants::filter_reply(&self.grant, filter, reply))
                    }
                }
            }
            (Object::Streams, "Capabilities") => Ok(json!(CapabilitiesReply::default())),
            (Object::Blobs, "Capabilities") => Ok(json!(BlobCapabilities::default())),
            (Object::Kv, "Capabilities") => Ok(json!(KvCapabilitiesReply::default())),
            (Object::Queues, "Features") => Ok(json!(QueueFeaturesReply::default())),
            (Object::Secrets, _) => Err(Self::unsupported()),
            (Object::Streams | Object::Blobs | Object::Kv | Object::Queues, _) => {
                Err(Self::denied())
            }
        }
    }

    async fn put_blob_stream(
        &self,
        sender: Option<&BusName>,
        _head: PutBlobStreamRequest,
        _body: BlobReader,
    ) -> Result<Value, HostFailure> {
        self.authorize(sender)?;
        Err(Self::denied())
    }

    async fn get_blob_stream(
        &self,
        sender: Option<&BusName>,
        _request: BlobKeyRequest,
    ) -> Result<Option<(BlobStreamHead, BlobReader)>, HostFailure> {
        self.authorize(sender)?;
        Err(Self::denied())
    }

    fn watch_signals(&self, _sender: &BusName) -> Option<broadcast::Receiver<WatchChanged>> {
        None
    }
}

#[cfg(test)]
#[path = "storage_host_invocation_tests.rs"]
mod tests;
