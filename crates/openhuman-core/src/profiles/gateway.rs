//! The profile side of a SaaS gateway request: which profile a named user
//! runs under.
//!
//! The gateway protocol itself (the user and signature headers, the HMAC
//! check, the refusal statuses) belongs to the server in `openhuman-rpc`
//! (`server/saas_gateway.rs`). The core keeps only what needs the profile
//! host: mapping the user id to a profile, asking the caller's signature
//! check when the operator requires one, and opening the profile. The core
//! never verifies the user's own credential; the gateway did that.

use std::sync::Arc;

use super::host::{Profile, ProfileHost};
use super::lease::OpenError;
use super::types::ProfileId;

/// Why a request naming a user got no profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveUserError {
    /// The user id does not name a profile (empty, too long, ...).
    InvalidUser(String),
    /// The signature check refused the request.
    Signature(String),
    /// The profile could not be opened.
    Open(OpenError),
}

impl ProfileHost {
    /// The profile that serves `user_id`, or why there is none.
    ///
    /// When the operator requires a user signature, `check_signature` runs
    /// after the user id is mapped and before the profile is touched, so a
    /// caller with a bad signature learns nothing about which users are
    /// provisioned.
    pub async fn resolve_user(
        &self,
        user_id: &str,
        check_signature: impl FnOnce() -> Result<(), String>,
    ) -> Result<Arc<Profile>, ResolveUserError> {
        let profile = ProfileId::for_user(user_id, self.saas().profile_ids)
            .map_err(ResolveUserError::InvalidUser)?;
        if self.saas().require_user_signature {
            check_signature().map_err(ResolveUserError::Signature)?;
        }
        match self.open(&profile).await {
            Ok(state) => {
                log::debug!("[profiles][gateway] scoped request to an open profile");
                Ok(state)
            }
            Err(error) => Err(ResolveUserError::Open(error)),
        }
    }
}

#[cfg(test)]
#[path = "gateway_proptest_tests.rs"]
mod proptest_tests;
