//! OS keyring operations through TinySecurity, which wait on the user.

use super::super::policy::KEYRING_CONSENT_TIMEOUT_MS;
use super::{Client, SecurityError, SecurityProxy};
use tinysecurity_bus::{KeyringRequest2, KeyringResponse2};

/// The host's deadline for one keyring operation: the module's consent
/// deadline plus the keyring store itself.
const KEYRING_ACCESS_DEADLINE: std::time::Duration =
    std::time::Duration::from_millis(KEYRING_CONSENT_TIMEOUT_MS as u64 + 15_000);
const _: () = assert!(KEYRING_ACCESS_DEADLINE.as_millis() > KEYRING_CONSENT_TIMEOUT_MS as u128);

impl SecurityProxy {
    /// The same binding with `timeout` as its call deadline. The test-only
    /// module client keeps its own.
    fn with_timeout(&self, timeout: std::time::Duration) -> Self {
        match self {
            Self::Callback(proxy) => Self::Callback(proxy.clone().with_timeout(timeout)),
            Self::Module(proxy) => Self::Module(proxy.clone()),
        }
    }
}

impl Client {
    /// Perform one OS keyring operation through TinySecurity. The module
    /// obtains fresh, operation-scoped consent before touching the backend,
    /// so the call waits as long as the user may take to answer the prompt.
    pub async fn keyring_access2(
        &self,
        request: KeyringRequest2,
    ) -> Result<KeyringResponse2, SecurityError> {
        self.object
            .with_timeout(KEYRING_ACCESS_DEADLINE)
            .call_confidential(
                tinysecurity_bus::names::methods::KEYRING_ACCESS2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }
}
