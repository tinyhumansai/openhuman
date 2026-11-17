//! Process credential request, mutation, clearing, and teardown operations.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::credentials::ops::gated_services::{
    spawn_integrations_cache_warm, start_credential_gated_services, stop_credential_gated_services,
};
use crate::credentials::responses::AuthStateResponse;

pub(crate) const LOG_PREFIX: &str = "[credentials][set-credential]";
pub const PENDING_BACKEND_VALIDATION_FIELD: &str = "pendingBackendValidation";
pub(crate) static CREDENTIAL_MUTATION_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

mod clear;
mod request;
mod set;
mod teardown;

pub use clear::{clear_credential, clear_session, store_session};
pub(super) use request::resolve;
pub use request::SetCredentialRequest;
pub use set::set_credential;
pub(crate) use teardown::{
    clear_session_credential, publish_credential_changed, refuse_process_credential_in_saas,
};

#[cfg(test)]
pub(crate) use teardown::process_credential_refusal;

#[cfg(test)]
#[path = "../credential_tests.rs"]
mod tests;
