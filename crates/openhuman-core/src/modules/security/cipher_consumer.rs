//! Exact-module receiver for TinySecurity's confidential config-cipher output.
//!
//! Plaintext never comes back in an ordinary `ExecuteSecretCipher2` result.
//! The receiver owns bounded, zeroizing slots and only releases a slot after
//! the caller receives the matching successful result from TinySecurity.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tinysecurity_bus::{
    callback_names2, SecretCipherDeliveryIdentity2, SecretCipherDeliveryRequest2,
    SecretCipherDeliveryResponse2,
};
use zeroize::{Zeroize, Zeroizing};

const MAX_CIPHER_OUTPUT_BYTES: usize = 1_048_576;

/// Output staged by one exact TinySecurity module instance.
struct StagedOutput {
    identity: SecretCipherDeliveryIdentity2,
    plaintext: Zeroizing<Vec<u8>>,
    replacement: Option<Zeroizing<Vec<u8>>>,
}

impl Drop for StagedOutput {
    fn drop(&mut self) {
        self.plaintext.zeroize();
        if let Some(replacement) = &mut self.replacement {
            replacement.zeroize();
        }
    }
}

/// One registration's consumer identity and its bounded physical slots.
struct Route {
    consumer_id: String,
    slots: HashMap<u16, StagedOutput>,
}

/// Callback object scoped to the TinySecurity binding that loaded it.
///
/// TinySecurity delivers every runtime registration's cipher output to the
/// one consumer object on the binding, so the receiver keeps one route per
/// secret registration: an output is staged only for the registration and
/// consumer identity the host registered together, in that registration's
/// own slots. A retired registration's route is gone, so its late outputs
/// are refused.
pub(crate) struct Consumer {
    routes: Mutex<HashMap<String, Route>>,
}

impl std::fmt::Debug for Consumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretCipherConsumer([redacted])")
    }
}

impl Consumer {
    /// A receiver that accepts output for no registration yet.
    pub(crate) fn empty() -> Self {
        Self {
            routes: Mutex::new(HashMap::new()),
        }
    }

    /// A receiver routed for exactly one registration.
    #[cfg(test)]
    pub(crate) fn new(registration_id: String, consumer_id: String) -> Self {
        let consumer = Self::empty();
        consumer.add(registration_id, consumer_id).unwrap();
        consumer
    }

    /// Route `registration_id`'s outputs, staged under `consumer_id`.
    /// A registration keeps the one consumer it was routed with.
    pub(crate) fn add(
        &self,
        registration_id: String,
        consumer_id: String,
    ) -> Result<(), &'static str> {
        let mut routes = self
            .routes
            .lock()
            .map_err(|_| "cipher output unavailable")?;
        match routes.get(&registration_id) {
            Some(route) if route.consumer_id == consumer_id => Ok(()),
            Some(_) => Err("cipher consumer already routed"),
            None => {
                routes.insert(
                    registration_id,
                    Route {
                        consumer_id,
                        slots: HashMap::new(),
                    },
                );
                Ok(())
            }
        }
    }

    /// Stop routing a retired registration; its staged plaintext is wiped.
    pub(crate) fn remove(&self, registration_id: &str) {
        if let Ok(mut routes) = self.routes.lock() {
            routes.remove(registration_id);
        }
    }

    /// Serve this receiver only on the exact callback binding used to load
    /// TinySecurity. The binding itself authenticates the loaded module peer.
    pub(crate) async fn serve(
        self: Arc<Self>,
        binding: &tinybus::module::HostCallbackBinding,
    ) -> tinybus::Result<()> {
        binding
            .serve_at(callback_names2::OBJECT_PATH.parse()?, self)
            .await
    }

    /// Consume output only when TinySecurity returned the identical staged
    /// identity. Failed or mismatched calls leave their physical slot occupied.
    pub(crate) fn take(
        &self,
        identity: &SecretCipherDeliveryIdentity2,
    ) -> Result<(Zeroizing<String>, Option<Zeroizing<String>>), &'static str> {
        let mut routes = self
            .routes
            .lock()
            .map_err(|_| "cipher output unavailable")?;
        let route = routes
            .get_mut(&identity.registration_id)
            .filter(|route| route.consumer_id == identity.consumer_id)
            .ok_or("cipher output unavailable")?;
        let staged = route
            .slots
            .get(&identity.slot)
            .ok_or("cipher output unavailable")?;
        if &staged.identity != identity {
            return Err("cipher output unavailable");
        }
        let mut staged = route
            .slots
            .remove(&identity.slot)
            .ok_or("cipher output unavailable")?;
        let plaintext = String::from_utf8(staged.plaintext.to_vec())
            .map_err(|_| "cipher output is not valid UTF-8")?;
        let replacement = staged
            .replacement
            .take()
            .map(|bytes| String::from_utf8(bytes.to_vec()).map(Zeroizing::new))
            .transpose()
            .map_err(|_| "cipher replacement is not valid UTF-8")?;
        Ok((Zeroizing::new(plaintext), replacement))
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.SecretCipherConsumer2")]
impl Consumer {
    #[tinybus(confidential)]
    async fn deliver(
        &self,
        request: SecretCipherDeliveryRequest2,
    ) -> tinybus::Result<SecretCipherDeliveryResponse2> {
        let identity = &request.identity;
        if identity.expires_at_ms == 0
            || request.plaintext.len() > MAX_CIPHER_OUTPUT_BYTES
            || request
                .replacement
                .as_ref()
                .is_some_and(|bytes| bytes.len() > MAX_CIPHER_OUTPUT_BYTES)
        {
            return Err(tinybus::Error::failed("secret cipher output denied"));
        }
        let mut routes = self
            .routes
            .lock()
            .map_err(|_| tinybus::Error::failed("secret cipher output denied"))?;
        let Some(route) = routes
            .get_mut(&identity.registration_id)
            .filter(|route| route.consumer_id == identity.consumer_id)
        else {
            return Err(tinybus::Error::failed("secret cipher output denied"));
        };
        if route.slots.contains_key(&identity.slot) {
            return Err(tinybus::Error::failed("secret cipher output denied"));
        }
        let staged = StagedOutput {
            identity: identity.clone(),
            plaintext: Zeroizing::new(request.plaintext),
            replacement: request.replacement.map(Zeroizing::new),
        };
        route.slots.insert(identity.slot, staged);
        Ok(SecretCipherDeliveryResponse2 {
            identity: identity.clone(),
            staged: true,
        })
    }
}

#[cfg(test)]
#[path = "cipher_consumer_tests.rs"]
mod tests;
