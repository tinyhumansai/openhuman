//! Calling the `tinyflows` module over the bus, and serving its host objects.
//!
//! The module owns the scheduler runtime (poll loop, claims, retry, completion),
//! the cron job and run store, and the schedule maths behind
//! `tinyflows_bus::schedule` (contract v3). This host file is the whole
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

/// Registry id of the module these calls go to.
pub(crate) const MODULE_ID: &str = "tinyflows";

/// The bus name the host's callback and document objects are served under.
///
/// The module addresses every callback by `(destination, object path)`; one
/// name serves them all, and the object path names the scope.
pub(crate) const HOST_BUS_NAME: &str = "ai.tinyhumans.openhuman.CronHost";

/// The switched-off message callers surface verbatim.
pub(crate) fn disabled_message() -> String {
    format!(
        "{}:{MODULE_ID} (set [components] {MODULE_ID} = true to enable)",
        crate::core::runtime::components::COMPONENT_DISABLED
    )
}

#[cfg(feature = "modules")]
mod imp {
    use super::{disabled_message, HOST_BUS_NAME, MODULE_ID};
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

    /// A proxy for the module's `Schedule` object (store, maths and
    /// validation members, which answer quickly).
    pub(crate) async fn proxy(config: &Config) -> Result<Proxy, String> {
        schedule_proxy(config, CALL_TIMEOUT).await
    }

    /// A proxy whose deadline fits a call that runs a whole job (`ExecuteJob`).
    pub(crate) async fn run_proxy(config: &Config) -> Result<Proxy, String> {
        schedule_proxy(config, RUN_TIMEOUT).await
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

#[cfg(feature = "modules")]
pub(crate) use imp::{proxy, run_proxy, serve, unserve};

#[cfg(not(feature = "modules"))]
mod imp {
    use super::disabled_message;
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

    pub(crate) async fn serve(
        _path: String,
        _object: impl tinybus::service::Interface + 'static,
    ) -> Result<(), String> {
        Err("scheduled jobs are unavailable: this build has no module loader".to_string())
    }

    pub(crate) fn unserve(_path: String) {}
}

#[cfg(not(feature = "modules"))]
pub(crate) use imp::{proxy, run_proxy, serve, unserve};

/// Whether the module can serve in this build and configuration (the switch
/// is on and the loader exists). Does not load anything.
#[must_use]
pub(crate) fn available(config: &Config) -> bool {
    config.components.tinyflows && cfg!(feature = "modules")
}

#[cfg(test)]
#[path = "tinyflows_tests.rs"]
mod tests;
