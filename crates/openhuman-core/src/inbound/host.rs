//! The host callbacks the `tinyeventqueues` module calls:
//! `ai.tinyhumans.tinyeventqueues.Host` (`ClassifySalience`, `Deliver`).
//!
//! Served on the module runtime's connection before any module loads
//! (`modules::runtime::install_callbacks`), when the `eventqueues` component
//! is on. The module declares the interface optional: without it, deliveries
//! back off until it is served.

use std::sync::Arc;

use tinybus::ObjectPath;
use tinyeventqueues_bus::names::host::{INTERFACE as NAME, OBJECT_PATH as PATH};
use tinyeventqueues_bus::{
    ClassifySalienceRequest, ClassifySalienceResponse, DeliverRequest, DeliverResponse,
};

use super::deliver::Deliverer;
use crate::core::runtime::DispatchCtx;
use crate::triage::RemoteTriage;

/// The served callback object.
#[derive(Clone)]
pub(crate) struct InboundHost {
    deliverer: Arc<Deliverer>,
}

impl InboundHost {
    pub(crate) fn new(deliverer: Arc<Deliverer>) -> Self {
        Self { deliverer }
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinyeventqueues.Host")]
impl InboundHost {
    /// Classify a borderline event by triaging it. A failure keeps the class
    /// the module's score gave the event.
    async fn classify_salience(
        &self,
        request: ClassifySalienceRequest,
    ) -> tinybus::Result<ClassifySalienceResponse> {
        self.deliverer
            .classify(request)
            .await
            .map_err(|message| tinybus::Error::MethodFailed {
                name: "ai.tinyhumans.tinyeventqueues.Error.Host".to_string(),
                message,
            })
    }

    /// Deliver one due turn: triage it and answer with the ack semantics.
    async fn deliver(&self, request: DeliverRequest) -> tinybus::Result<DeliverResponse> {
        Ok(self.deliverer.deliver(request).await)
    }
}

/// Serve the callbacks on `connection`, triaging under the host's own context.
pub(crate) async fn install(connection: &tinybus::Connection) -> tinybus::Result<()> {
    let ctx = DispatchCtx::host_default();
    // The triage timeout is read from `[inbound]` on the first callback:
    // loading the config here would initialise one on a host that has none.
    let deliverer = Deliverer::configured(Arc::new(RemoteTriage), ctx);
    connection
        .serve_at(
            ObjectPath::new(PATH)?,
            // Only the TinyEventQueues module may deliver a turn: any other peer
            // on the shared module connection could otherwise start one.
            crate::modules::callback_owner::OwnedBy::new(
                InboundHost::new(Arc::new(deliverer)),
                connection,
                tinyeventqueues_bus::names::INTERFACE,
            ),
        )
        .await?;
    connection.request_name(NAME).await?;
    tracing::debug!("[inbound] serving the tinyeventqueues host callbacks");
    Ok(())
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
