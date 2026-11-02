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
use openhuman_embed::__host::core::slots::Extensions;

/// Where registrations contributed during setup should be visible.
pub(crate) enum DoctorRegistration<'a> {
    /// Compatibility hosts install the TinyHumans layer for the whole process.
    Process,
    /// RuntimeBuilder hosts bind checks to the CoreContext they create.
    Context(&'a Extensions),
}
#[cfg(feature = "voice")]
use crate::voice;
use crate::{channels, client, devices, inference, integrations, memory};

/// Registers every area's hooks with the core.
pub(crate) fn register_all(options: &InstallOptions, doctor_registration: DoctorRegistration<'_>) {
    log::debug!("[tinyhumans] register_all: registering core hooks");
    openhuman_embed::__host::credentials::install_provider_credential_source(std::sync::Arc::new(
        crate::provider_oauth::OpenAiProviderCredentialSource::default(),
    ));
    crate::hosted_config::register();
    client::register(options);
    crate::socket::register();
    devices::tunnel_client::register();
    integrations::register(options);
    #[cfg(feature = "voice")]
    voice::register(options);
    inference::register(options);
    channels::register(options);
    memory::register(options);
    crate::doctor::register(doctor_registration);
}

#[cfg(test)]
#[path = "register_tests.rs"]
mod tests;
