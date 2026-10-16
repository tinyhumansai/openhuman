//! The one place this crate plugs into the core's generic registries.
//!
//! [`register_all`] runs from [`install`](crate::install) (and so from
//! [`RuntimeBuilder::build`](crate::RuntimeBuilder::build), which installs
//! first). Each area adds exactly one line; an area registers through the
//! `openhuman_embed::__host` extension points and nothing else.
//!
//! Registration is idempotent: re-registering an id replaces it in place, so a
//! second `install` is harmless.

use crate::install::InstallOptions;
#[cfg(feature = "voice")]
use crate::voice;
use crate::{channels, client, inference, integrations, memory};

/// Registers every area's hooks with the core.
pub(crate) fn register_all(options: &InstallOptions) {
    log::debug!("[tinyhumans] register_all: registering core hooks");
    client::register(options);
    integrations::register(options);
    #[cfg(feature = "voice")]
    voice::register(options);
    inference::register(options);
    channels::register(options);
    memory::register(options);
}

#[cfg(test)]
#[path = "register_tests.rs"]
mod tests;
