//! The bus endpoint of the storage host: how `tinystoragedrivers-busproxy`
//! reaches [`StorageHost`], and how a caller's watch invalidations get from the
//! caller's bound server to the proxy's unicast forwarder.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinybus::BusName;
use tinystoragedrivers::BlobReader;
use tinystoragedrivers_bus::{BlobKeyRequest, BlobStreamHead, PutBlobStreamRequest, WatchChanged};
use tinystoragedrivers_busproxy::{HostFailure, StorageCall, StorageEndpoint};
use tokio::sync::broadcast;

use super::{Binding, HostError, StorageHost};

/// Relays kept for callers that never became a bound module.
const MAX_IDLE_RELAYS: usize = 256;

/// Where one caller's watch invalidations are collected for the bus endpoint.
///
/// The endpoint asks for a caller's signals synchronously and before the
/// caller's first `WatchOpen` reaches the host, when the host has not yet
/// resolved which module the caller is. So the relay exists per caller from
/// the first ask, and [`StorageHost::handle`] feeds it from the caller's bound
/// server as soon as the caller is a known module (before the call is
/// forwarded, so no signal is missed).
pub(super) struct Relay {
    pub(super) tx: broadcast::Sender<WatchChanged>,
    /// The binding currently feeding `tx`, so a rebind starts a new bridge.
    pub(super) fed_by: Option<std::sync::Weak<Binding>>,
    pub(super) bridge: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        if let Some(bridge) = self.bridge.take() {
            bridge.abort();
        }
    }
}

/// Signals buffered per caller before the slowest reader loses the oldest.
const RELAY_BUFFER: usize = 256;

impl StorageHost {
    /// The watch invalidations of `sender`, for the bus endpoint to unicast.
    pub(super) fn watch_signals(
        &self,
        sender: &BusName,
    ) -> Option<broadcast::Receiver<WatchChanged>> {
        let mut relays = self.relays.lock().ok()?;
        if !relays.contains_key(sender) {
            // Callers that never became a module leave an idle relay behind;
            // keep those bounded.
            if relays.len() >= MAX_IDLE_RELAYS {
                relays.retain(|_, relay| relay.bridge.is_some() || relay.tx.receiver_count() > 0);
            }
            if relays.len() >= MAX_IDLE_RELAYS {
                return None;
            }
            let (tx, _) = broadcast::channel(RELAY_BUFFER);
            relays.insert(
                sender.clone(),
                Relay {
                    tx,
                    fed_by: None,
                    bridge: None,
                },
            );
        }
        relays.get(sender).map(|relay| relay.tx.subscribe())
    }

    /// Starts feeding `sender`'s relay from `binding`'s server, replacing a
    /// bridge from an earlier binding. The bridge subscribes before it is
    /// spawned, so nothing the server signals afterwards is lost.
    pub(super) fn feed_relay(&self, sender: &BusName, binding: &Arc<Binding>) {
        let Ok(mut relays) = self.relays.lock() else {
            return;
        };
        let Some(relay) = relays.get_mut(sender) else {
            // Nobody asked for this caller's signals (it never opened a watch
            // through the bus), so there is nothing to feed.
            return;
        };
        let current = relay
            .fed_by
            .as_ref()
            .is_some_and(|fed| fed.ptr_eq(&Arc::downgrade(binding)));
        if current
            && relay
                .bridge
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        if let Some(stale) = relay.bridge.take() {
            stale.abort();
        }
        let mut signals = binding.server.signals();
        let tx = relay.tx.clone();
        relay.fed_by = Some(Arc::downgrade(binding));
        relay.bridge = Some(
            // The forwarder only moves signals between channels and reads no
            // scope, so it needs no tenant.
            crate::core::runtime::DispatchCtx::host_default().spawn(async move {
                loop {
                    match signals.recv().await {
                        Ok(signal) => {
                            // No receiver just means the bus forwarder is gone.
                            let _ = tx.send(signal);
                        }
                        Err(broadcast::error::RecvError::Lagged(missed)) => {
                            // The watch ids in the skipped batch are gone; the
                            // signal carries no reset form, so record the loss.
                            tracing::warn!(
                                target: "storage_host",
                                missed,
                                "[storage-host] watch relay lagged; dropped watch signals"
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            }),
        );
    }
}

/// The host as the bus endpoint `tinystoragedrivers-busproxy` serves the four
/// objects from. The proxy owns the wire (positional arguments, the peer-bound
/// stream accept and open, unicast watch signals); every decision stays in
/// [`StorageHost`].
pub(super) struct HostEndpoint {
    host: Arc<StorageHost>,
    /// Where the work runs when the objects are served from a linked module's
    /// own runtime: the host's, which the backend handles were opened on.
    runtime: Option<tokio::runtime::Handle>,
}

impl HostEndpoint {
    pub(super) fn new(host: Arc<StorageHost>, runtime: Option<tokio::runtime::Handle>) -> Self {
        Self { host, runtime }
    }

    /// Runs `work` on the host's runtime when one was given.
    async fn on_host<T, F>(&self, work: F) -> Result<T, HostFailure>
    where
        T: Send + 'static,
        F: std::future::Future<Output = Result<T, HostError>> + Send + 'static,
    {
        match &self.runtime {
            None => work.await.map_err(Into::into),
            Some(runtime) => runtime
                .spawn(work)
                .await
                .map_err(|_| HostFailure::Denied("call-cancelled"))?
                .map_err(Into::into),
        }
    }
}

#[async_trait]
impl StorageEndpoint for HostEndpoint {
    async fn call(&self, call: StorageCall<'_>) -> Result<Value, HostFailure> {
        let host = Arc::clone(&self.host);
        let (object, request, admission) = (call.object, call.request, call.admission);
        let selector = call.selector.to_string();
        let sender = call.sender.cloned();
        self.on_host(async move {
            host.handle(object, &selector, request, sender.as_ref(), admission)
                .await
        })
        .await
    }

    async fn put_blob_stream(
        &self,
        sender: Option<&BusName>,
        head: PutBlobStreamRequest,
        body: BlobReader,
    ) -> Result<Value, HostFailure> {
        let host = Arc::clone(&self.host);
        let sender = sender.cloned();
        self.on_host(async move { host.put_blob_stream(sender.as_ref(), head, body).await })
            .await
    }

    async fn get_blob_stream(
        &self,
        sender: Option<&BusName>,
        request: BlobKeyRequest,
    ) -> Result<Option<(BlobStreamHead, BlobReader)>, HostFailure> {
        let host = Arc::clone(&self.host);
        let sender = sender.cloned();
        self.on_host(async move { host.get_blob_stream(sender.as_ref(), request).await })
            .await
    }

    fn watch_signals(&self, sender: &BusName) -> Option<broadcast::Receiver<WatchChanged>> {
        self.host.watch_signals(sender)
    }
}
