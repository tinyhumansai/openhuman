//! Calling the `tinyflows` module over the bus, and serving its host objects.
//!
//! The module owns the scheduler runtime (poll loop, claims, retry, completion),
//! the cron job and run store, and the schedule maths behind
//! `tinyflows_bus::schedule` (contract v6). This host file is the whole
//! transport story for it:
//!
//! - [`proxy`] loads the module lazily and returns a proxy for its `Schedule`
//!   object. A registry `ModuleClient` cannot be used here because it reaches
//!   a module's *primary* interface (the authoring one), and the scheduler is a
//!   second interface on a second object path.
//! - [`serve`] / [`unserve`] export a host object (the `ScheduleHost` callbacks
//!   or the document port) on the module bus, on the module runtime. Objects
//!   must outlive whichever short-lived runtime asked first, so they are never
//!   served from the caller's runtime.
//!
//! Hosts reach the module through nothing else; `cron::client` wraps these in
//! the typed calls the rest of the core uses.
//!
//! With the `[components] tinyflows` switch off nothing here loads the module
//! and every call answers `COMPONENT_DISABLED:tinyflows`. A build without the
//! `modules` feature has no loader at all and answers that cron is unavailable.

use crate::config::Config;

#[cfg(feature = "modules")]
#[path = "tinyflows_caps_manifest.rs"]
mod caps_manifest;

/// Registry id of the module these calls go to.
pub(crate) const MODULE_ID: &str = "tinyflows";

/// The bus name the host's callback and document objects are served under.
///
/// The module addresses every callback by `(destination, object path)`; one
/// name serves them all, and the object path names the scope.
pub(crate) const HOST_BUS_NAME: &str = "ai.tinyhumans.openhuman.CronHost";

/// Owns the interfaces served at one module-bus object path.
pub(crate) struct ServedObjectRegistration(String);

impl Drop for ServedObjectRegistration {
    fn drop(&mut self) {
        unserve(std::mem::take(&mut self.0));
    }
}

/// The switched-off message callers surface verbatim.
pub(crate) fn disabled_message() -> String {
    format!(
        "{}:{MODULE_ID} (set [components] {MODULE_ID} = true to enable)",
        crate::core::runtime::components::COMPONENT_DISABLED
    )
}

#[cfg(feature = "modules")]
mod imp {
    #[cfg(not(test))]
    use super::MODULE_ID;
    use super::{disabled_message, ServedObjectRegistration, HOST_BUS_NAME};
    use crate::config::Config;
    #[cfg(not(test))]
    use crate::modules::ops;
    use crate::modules::runtime;
    use tinybus::{ObjectPath, Proxy};
    use tinyflows_bus::schedule;

    /// Longest a store or maths call waits. Job execution is not a call to the
    /// module's `Schedule` object that returns quickly, so [`run_proxy`] has its
    /// own bound.
    const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

    /// Longest `ExecuteJob` (Run Now) waits: the host runs the job's real work
    /// (an agent turn, a shell command) inside the call.
    const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2 * 60 * 60);

    /// Make sure the module serves. In unit tests this attaches the linked
    /// module built into the test binary; everywhere else it goes through the
    /// registry resolver (release, `source` or `linked` per configuration).
    async fn ensure(config: &Config) -> Result<(), String> {
        if !config.components.tinyflows {
            return Err(disabled_message());
        }
        #[cfg(test)]
        {
            let _ = config;
            super::fixture::attach().await
        }
        #[cfg(not(test))]
        {
            ops::ensure_loaded(config, MODULE_ID).await
        }
    }

    async fn ensure_workflow_ready(config: &Config) -> Result<(), String> {
        #[cfg(feature = "flows")]
        crate::storage::import::tinyflows::ensure_flow_scope_ready(config).await?;
        #[cfg(not(feature = "flows"))]
        let _ = config;
        Ok(())
    }

    async fn schedule_proxy(
        config: &Config,
        timeout: std::time::Duration,
    ) -> Result<Proxy, String> {
        ensure(config).await?;
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        runtime
            .connection()
            .proxy(
                schedule::BUS_NAME,
                schedule::OBJECT_PATH,
                schedule::INTERFACE,
            )
            .map(|proxy| proxy.with_timeout(timeout))
            .map_err(|error| format!("the tinyflows schedule object is unreachable: {error}"))
    }

    async fn copilot_interface_proxy(config: &Config) -> Result<Proxy, String> {
        ensure(config).await?;
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        runtime
            .connection()
            .proxy(
                tinyflows_bus::BUS_NAME,
                tinyflows_bus::OBJECT_PATH,
                tinyflows_bus::copilot::INTERFACE,
            )
            .map(|proxy| proxy.with_timeout(CALL_TIMEOUT))
            .map_err(|error| format!("the tinyflows copilot object is unreachable: {error}"))
    }

    /// Proxy to the module-owned Authoring contract.
    pub(crate) async fn authoring_proxy(config: &Config) -> Result<Proxy, String> {
        ensure_workflow_ready(config).await?;
        ensure(config).await?;
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        runtime
            .connection()
            .proxy(
                tinyflows_bus::BUS_NAME,
                tinyflows_bus::OBJECT_PATH,
                tinyflows_bus::INTERFACE,
            )
            .map(|proxy| proxy.with_timeout(CALL_TIMEOUT))
            .map_err(|error| format!("the tinyflows authoring object is unreachable: {error}"))
    }

    /// Proxy to the module-owned Catalog contract.
    pub(crate) async fn catalog_proxy(config: &Config) -> Result<Proxy, String> {
        ensure_workflow_ready(config).await?;
        catalog_proxy_for_migration(config).await
    }

    /// Catalog proxy used only while migrations or bounded legacy imports run.
    pub(crate) async fn catalog_proxy_for_migration(config: &Config) -> Result<Proxy, String> {
        ensure(config).await?;
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        runtime
            .connection()
            .proxy(
                tinyflows_bus::BUS_NAME,
                tinyflows_bus::catalog::OBJECT_PATH,
                tinyflows_bus::catalog::INTERFACE,
            )
            .map(|proxy| proxy.with_timeout(CALL_TIMEOUT))
            .map_err(|error| format!("the tinyflows catalog object is unreachable: {error}"))
    }

    /// Proxy to the module-owned portability helpers used by CapsHost.
    pub(crate) async fn portability_proxy(config: &Config) -> Result<Proxy, String> {
        ensure_workflow_ready(config).await?;
        ensure(config).await?;
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        runtime
            .connection()
            .proxy(
                tinyflows_bus::BUS_NAME,
                tinyflows_bus::OBJECT_PATH,
                tinyflows_bus::portability::INTERFACE,
            )
            .map(|proxy| proxy.with_timeout(CALL_TIMEOUT))
            .map_err(|error| format!("the tinyflows portability object is unreachable: {error}"))
    }

    /// Proxy to the module-owned run admission and control surface.
    pub(crate) async fn runs_proxy(config: &Config) -> Result<Proxy, String> {
        ensure_workflow_ready(config).await?;
        ensure(config).await?;
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        runtime
            .connection()
            .proxy(
                tinyflows_bus::BUS_NAME,
                tinyflows_bus::runs::OBJECT_PATH,
                tinyflows_bus::runs::INTERFACE,
            )
            .map(|proxy| proxy.with_timeout(CALL_TIMEOUT))
            .map_err(|error| format!("the tinyflows runs object is unreachable: {error}"))
    }

    async fn copilot_invoke_interface_proxy(config: &Config) -> Result<Proxy, String> {
        ensure(config).await?;
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        runtime
            .connection()
            .proxy(
                tinyflows_bus::BUS_NAME,
                tinyflows_bus::OBJECT_PATH,
                tinyflows_bus::copilot::INTERFACE,
            )
            .map(|proxy| proxy.with_timeout(RUN_TIMEOUT))
            .map_err(|error| format!("the tinyflows copilot object is unreachable: {error}"))
    }

    /// A proxy for the module's `Schedule` object (store, maths and
    /// validation members, which answer quickly).
    pub(crate) async fn proxy(config: &Config) -> Result<Proxy, String> {
        schedule_proxy(config, CALL_TIMEOUT).await
    }

    /// A proxy whose deadline fits a call that runs a whole job (`ExecuteJob`).
    pub(crate) async fn run_proxy(config: &Config) -> Result<Proxy, String> {
        schedule_proxy(config, RUN_TIMEOUT).await
    }

    /// A proxy for the module's Copilot tool, prompt and fallback services.
    pub(crate) async fn copilot_proxy(config: &Config) -> Result<Proxy, String> {
        ensure_workflow_ready(config).await?;
        copilot_interface_proxy(config).await
    }

    /// A long-deadline Copilot proxy for module-owned tool execution.
    pub(crate) async fn copilot_invoke_proxy(config: &Config) -> Result<Proxy, String> {
        ensure_workflow_ready(config).await?;
        copilot_invoke_interface_proxy(config).await
    }

    /// Serve `object` at `path` on the module bus, on the module runtime, and
    /// own the host bus name. Idempotent per path only if `path` is unserved
    /// first ([`unserve`]).
    pub(crate) async fn serve(
        path: String,
        object: impl tinybus::service::Interface + 'static,
    ) -> Result<(), String> {
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        let connection = runtime.connection().clone();
        let served = runtime
            .spawn(async move {
                let path = ObjectPath::new(&path).map_err(|e| e.to_string())?;
                connection
                    .request_name(HOST_BUS_NAME)
                    .await
                    .map_err(|e| e.to_string())?;
                connection
                    .serve_at(path, object)
                    .await
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|error| format!("serving the host object did not finish: {error}"))?;
        served
    }

    /// Serve one callback interface on the module runtime and return an RAII
    /// registration for it. The token is created inside the spawned task so
    /// cancellation of the caller cannot leave an unowned interface behind.
    /// CapsHost registrations advertise the v2 callback contract before their
    /// address can be handed to the Runs module.
    pub(crate) async fn serve_registered(
        path: String,
        object: impl tinybus::service::Interface + 'static,
    ) -> Result<ServedObjectRegistration, String> {
        let announces_caps_host = super::caps_manifest::is_caps_host_v2(&object)?;
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        let connection = runtime.connection().clone();
        runtime
            .spawn(async move {
                let object_path = ObjectPath::new(&path).map_err(|error| error.to_string())?;
                connection
                    .request_name(HOST_BUS_NAME)
                    .await
                    .map_err(|error| error.to_string())?;
                connection
                    .serve_at(object_path.clone(), object)
                    .await
                    .map_err(|error| error.to_string())?;
                if announces_caps_host {
                    if let Err(error) =
                        super::caps_manifest::announce_caps_host_v2(&connection).await
                    {
                        let _ = connection.unserve(&object_path).await;
                        return Err(error);
                    }
                }
                Ok(ServedObjectRegistration(path))
            })
            .await
            .map_err(|error| format!("serving the host object did not finish: {error}"))?
    }

    /// Serve two interfaces at one object path and return an RAII token for
    /// their shared registration. The token is created on the module runtime
    /// task, so if the caller is cancelled while this operation is in flight,
    /// the detached task's output drops the token and unregisters the object.
    pub(crate) async fn serve_pair(
        path: String,
        first: impl tinybus::service::Interface + 'static,
        second: impl tinybus::service::Interface + 'static,
    ) -> Result<ServedObjectRegistration, String> {
        serve_pair_after_first(path, first, second, async {}).await
    }

    pub(crate) async fn serve_pair_after_first<F>(
        path: String,
        first: impl tinybus::service::Interface + 'static,
        second: impl tinybus::service::Interface + 'static,
        after_first: F,
    ) -> Result<ServedObjectRegistration, String>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let runtime = runtime::runtime()
            .await
            .map_err(|error| format!("the module bus is unavailable: {error}"))?;
        let connection = runtime.connection().clone();
        runtime
            .spawn(async move {
                let object_path = ObjectPath::new(&path).map_err(|error| error.to_string())?;
                connection
                    .request_name(HOST_BUS_NAME)
                    .await
                    .map_err(|error| error.to_string())?;
                connection
                    .serve_at(object_path.clone(), first)
                    .await
                    .map_err(|error| error.to_string())?;
                after_first.await;
                if let Err(error) = connection.serve_at(object_path.clone(), second).await {
                    let _ = connection.unserve(&object_path).await;
                    return Err(error.to_string());
                }
                Ok(ServedObjectRegistration(path))
            })
            .await
            .map_err(|error| format!("serving the host object did not finish: {error}"))?
    }

    /// Stop serving `path`. Fire and forget: a call already admitted finishes.
    /// Without a runtime to run on (a sync drop at exit) the object is left;
    /// the process is ending anyway.
    pub(crate) fn unserve(path: String) {
        if !runtime::is_started() {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            let Ok(runtime) = runtime::runtime().await else {
                return;
            };
            let connection = runtime.connection().clone();
            let _ = runtime
                .spawn(async move {
                    if let Ok(object) = ObjectPath::new(&path) {
                        let removed = connection.unserve(&object).await;
                        tracing::trace!(path = %path, removed, "[cron:bus] host object unserved");
                    }
                })
                .await;
        });
    }
}

#[cfg(all(test, feature = "modules"))]
#[path = "tinyflows_fixture.rs"]
mod fixture;

#[cfg(all(test, feature = "modules"))]
pub(crate) use imp::serve_pair_after_first;
#[cfg(feature = "modules")]
pub(crate) use imp::{
    authoring_proxy, catalog_proxy, catalog_proxy_for_migration, copilot_invoke_proxy,
    copilot_proxy, portability_proxy, proxy, run_proxy, runs_proxy, serve, serve_pair,
    serve_registered, unserve,
};

#[cfg(not(feature = "modules"))]
mod imp {
    use super::{disabled_message, ServedObjectRegistration};
    use crate::config::Config;

    fn unavailable(config: &Config) -> String {
        if config.components.tinyflows {
            "scheduled jobs are unavailable: this build has no module loader (build with the `modules` feature)"
                .to_string()
        } else {
            disabled_message()
        }
    }

    pub(crate) async fn proxy(config: &Config) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn run_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn copilot_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn authoring_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn catalog_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn catalog_proxy_for_migration(
        config: &Config,
    ) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn portability_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn runs_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn copilot_invoke_proxy(config: &Config) -> Result<tinybus::Proxy, String> {
        Err(unavailable(config))
    }

    pub(crate) async fn serve(
        _path: String,
        _object: impl tinybus::service::Interface + 'static,
    ) -> Result<(), String> {
        Err("scheduled jobs are unavailable: this build has no module loader".to_string())
    }

    pub(crate) async fn serve_registered(
        _path: String,
        _object: impl tinybus::service::Interface + 'static,
    ) -> Result<ServedObjectRegistration, String> {
        Err("scheduled jobs are unavailable: this build has no module loader".to_string())
    }

    pub(crate) async fn serve_pair(
        _path: String,
        _first: impl tinybus::service::Interface + 'static,
        _second: impl tinybus::service::Interface + 'static,
    ) -> Result<ServedObjectRegistration, String> {
        Err("scheduled jobs are unavailable: this build has no module loader".to_string())
    }

    pub(crate) fn unserve(_path: String) {}
}

#[cfg(not(feature = "modules"))]
pub(crate) use imp::{
    authoring_proxy, catalog_proxy, catalog_proxy_for_migration, copilot_invoke_proxy,
    copilot_proxy, portability_proxy, proxy, run_proxy, runs_proxy, serve, serve_pair,
    serve_registered, unserve,
};

/// Whether the module can serve in this build and configuration (the switch
/// is on and the loader exists). Does not load anything.
#[must_use]
pub(crate) fn available(config: &Config) -> bool {
    config.components.tinyflows && cfg!(feature = "modules")
}

#[cfg(test)]
#[path = "tinyflows_tests.rs"]
mod tests;
