//! The module client in a build without the loader.
//!
//! The `modules` feature gates the loader (and the archive and HTTP stack it
//! drags in), but the domains that call modules (accessibility, tokenjuice,
//! composio, channel health, gitbooks) are compiled either way and answer
//! "unavailable" when there is no loader. [`client`] hands them a
//! `tinybus::module::ModuleClient` over the bare registry: every id is known,
//! nothing can be loaded, and each refusal is reported once with the closed
//! `loader_disabled` reason.

use tinybus::module::{
    AccessFuture, FailureReporter, ModuleAccess, ModuleClient, ProxyFailure, Reason, ReleaseRecord,
};
use tinybus::Proxy;

use super::{registry, report};
use crate::config::Config;

/// A registry with no loader behind it.
struct Unloaded {
    enabled: bool,
    reporter: std::sync::Arc<FailureReporter>,
}

impl ModuleAccess for Unloaded {
    fn find(&self, id: &str) -> Option<&'static ReleaseRecord> {
        registry::release_records()
            .into_iter()
            .find(|record| record.id == id)
    }

    fn enabled(&self) -> bool {
        self.enabled
    }

    fn reporter(&self) -> &FailureReporter {
        &self.reporter
    }

    fn ensure_loaded<'a>(&'a self, id: &'a str) -> AccessFuture<'a, Result<(), String>> {
        Box::pin(async move {
            log::debug!("[modules] '{id}' unavailable: this build has no module loader");
            if let Some(record) = self.find(id) {
                self.reporter
                    .report(record.id, record.version, Reason::LoaderDisabled);
            }
            Err(format!(
                "module '{id}' is unavailable: this build has no module loader"
            ))
        })
    }

    fn proxy<'a>(
        &'a self,
        _bus_name: &'a str,
        _object_path: &'a str,
    ) -> AccessFuture<'a, Result<Proxy, ProxyFailure>> {
        Box::pin(async { Err(ProxyFailure::Transport) })
    }
}

/// A client whose every call is unavailable and reported.
#[must_use]
pub fn client(config: Config) -> ModuleClient {
    ModuleClient::new(Unloaded {
        enabled: config.modules.enabled,
        reporter: report::reporter(),
    })
}

#[cfg(test)]
#[path = "unloaded_tests.rs"]
mod tests;
