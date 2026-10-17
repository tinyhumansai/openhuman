//! Identity-scoped channel objects and cancellation-safe callback ownership.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tinybus::module::{ModuleCallError, ModuleClient, ModuleObjectClient};
use tinybus::{Connection, ObjectPath, Proxy};
use tinychannels_bus::{
    methods, ChannelRuntimeReservation, HOST_BUS_NAME, HOST_OBJECT_PATH, OBJECT_PATH,
};
use tokio::sync::Mutex;

const CLOSE_BUDGET: Duration = Duration::from_secs(15);

/// Keeps a captured host adapter exported until the module acknowledges stop.
#[derive(Clone)]
pub(crate) struct ChannelRuntimeLease(Arc<LeaseState>);

struct LeaseState {
    object: ModuleObjectClient,
    cleanup: Cleanup,
    closing: Mutex<Option<tokio::task::JoinHandle<Result<(), ModuleCallError>>>>,
    runtime: &'static tinybus::module::ModuleRuntime,
}

#[derive(Clone)]
struct Cleanup {
    proxy: Proxy,
    connection: Connection,
    callback: ObjectPath,
    handle: String,
    closed: Arc<AtomicBool>,
    reported: Arc<AtomicBool>,
}

impl Cleanup {
    async fn run(&self) -> Result<(), ModuleCallError> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        // Stop is idempotent. A transport deadline never licenses removal of
        // the adapter: the module may still own listeners or pending sends.
        let result: tinybus::Result<()> = self
            .proxy
            .call_confidential(methods::STOP_RUNTIME, (self.handle.clone(),))
            .await;
        result.map_err(|error| {
            let record = super::registry::find("tinychannels").expect("registered channel module");
            if !self.reported.swap(true, Ordering::AcqRel) {
                super::report::proxy_failure(record, &error)
            } else {
                ModuleCallError::TransportFailed
            }
        })?;
        self.connection.unserve(&self.callback).await;
        self.closed.store(true, Ordering::Release);
        Ok(())
    }
}

impl Drop for LeaseState {
    fn drop(&mut self) {
        if self.cleanup.closed.load(Ordering::Acquire) {
            return;
        }
        let pending = self.closing.get_mut().take();
        let cleanup = self.cleanup.clone();
        // Retain both the callback connection and its captured identity after
        // cancellation or last-owner drop. Never abort a module's shutdown.
        self.runtime.spawn(async move {
            if let Some(pending) = pending {
                if matches!(pending.await, Ok(Ok(()))) {
                    return;
                }
            }
            loop {
                if cleanup.run().await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }
}

impl ChannelRuntimeLease {
    pub(crate) async fn open(
        client: ModuleClient,
        callback: impl tinybus::service::Interface + 'static,
    ) -> Result<Self, ModuleCallError> {
        // This check performs lazy admission before starting any callback work.
        let reservation: ChannelRuntimeReservation = client
            .call_confidential("tinychannels", methods::RESERVE_RUNTIME, ())
            .await?;
        validate_reservation(&reservation)?;
        let runtime = super::runtime::runtime()
            .await
            .map_err(|_| ModuleCallError::TransportFailed)?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        runtime.spawn(async move {
            let result = Self::activate(client, reservation, callback, runtime).await;
            // A dropped receiver drops the lease, which owns cleanup. The
            // activation task itself is independent of the requesting runtime.
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| ModuleCallError::TransportFailed)?
    }

    async fn activate(
        client: ModuleClient,
        reservation: ChannelRuntimeReservation,
        callback: impl tinybus::service::Interface,
        runtime: &'static tinybus::module::ModuleRuntime,
    ) -> Result<Self, ModuleCallError> {
        let callback_path = ObjectPath::new(&reservation.host_object_path)
            .map_err(|_| ModuleCallError::ModuleFault)?;
        let (connection, proxy) = runtime_transport(&client, runtime)?;
        let cleanup = Cleanup {
            proxy,
            connection: connection.clone(),
            callback: callback_path.clone(),
            handle: reservation.handle.clone(),
            closed: Arc::new(AtomicBool::new(false)),
            reported: Arc::new(AtomicBool::new(false)),
        };
        // Acquire an owned lease before exporting the adapter. Any subsequent
        // failure follows the same acknowledged-stop path as explicit close.
        let object = match client
            .object_client("tinychannels", &reservation.object_path)
            .await
        {
            Ok(object) => object,
            Err(error) => {
                runtime.spawn(async move {
                    while cleanup.run().await.is_err() {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                });
                return Err(error);
            }
        };
        let lease = Self(Arc::new(LeaseState {
            object,
            cleanup,
            closing: Mutex::new(None),
            runtime,
        }));
        connection
            .request_name(HOST_BUS_NAME)
            .await
            .map_err(|_| ModuleCallError::TransportFailed)?;
        connection
            .serve_at(callback_path, callback)
            .await
            .map_err(|_| ModuleCallError::TransportFailed)?;
        client
            .call_confidential::<()>(
                "tinychannels",
                methods::START_RUNTIME,
                (reservation.handle,),
            )
            .await?;
        Ok(lease)
    }

    pub(crate) fn object(&self) -> &ModuleObjectClient {
        &self.0.object
    }

    pub(crate) async fn close(&self) -> Result<(), ModuleCallError> {
        if self.0.cleanup.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        join_cleanup(&self.0.closing, || {
            let cleanup = self.0.cleanup.clone();
            self.0.runtime.spawn(async move { cleanup.run().await })
        })
        .await
    }
}

async fn join_cleanup(
    closing: &Mutex<Option<tokio::task::JoinHandle<Result<(), ModuleCallError>>>>,
    start: impl FnOnce() -> tokio::task::JoinHandle<Result<(), ModuleCallError>>,
) -> Result<(), ModuleCallError> {
    let mut closing = closing.lock().await;
    if closing.is_none() {
        *closing = Some(start());
    }
    let result = tokio::time::timeout(CLOSE_BUDGET, closing.as_mut().expect("cleanup worker"))
        .await
        .map_err(|_| ModuleCallError::TransportFailed)?;
    closing.take();
    result.map_err(|_| ModuleCallError::TransportFailed)?
}

fn runtime_transport(
    client: &ModuleClient,
    runtime: &'static tinybus::module::ModuleRuntime,
) -> Result<(Connection, Proxy), ModuleCallError> {
    #[cfg(test)]
    if let Some(proxy) = client.fixture_proxy() {
        return Ok((proxy.connection().clone(), proxy));
    }
    let _ = client;
    Ok((
        runtime.connection().clone(),
        runtime
            .proxy(tinychannels_bus::BUS_NAME, OBJECT_PATH)
            .map_err(|_| ModuleCallError::IncompatibleContract)?,
    ))
}

fn validate_reservation(reservation: &ChannelRuntimeReservation) -> Result<(), ModuleCallError> {
    if reservation.handle.len() != 32
        || !reservation
            .handle
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || reservation.object_path != format!("{OBJECT_PATH}/runtime_{}", reservation.handle)
        || reservation.host_object_path
            != format!("{HOST_OBJECT_PATH}/runtime_{}", reservation.handle)
    {
        crate::modules::report::malformed_reply("tinychannels");
        return Err(ModuleCallError::ModuleFault);
    }
    Ok(())
}

#[cfg(test)]
#[path = "channel_runtime_tests.rs"]
mod tests;
