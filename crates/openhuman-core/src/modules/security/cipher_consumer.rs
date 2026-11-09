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

/// Callback object scoped to the TinySecurity binding that loaded it.
pub(crate) struct Consumer {
    registration_id: String,
    consumer_id: String,
    slots: Mutex<HashMap<u16, StagedOutput>>,
}

impl std::fmt::Debug for Consumer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretCipherConsumer([redacted])")
    }
}

impl Consumer {
    pub(crate) fn new(registration_id: String, consumer_id: String) -> Self {
        Self {
            registration_id,
            consumer_id,
            slots: Mutex::new(HashMap::new()),
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
        let mut slots = self.slots.lock().map_err(|_| "cipher output unavailable")?;
        let staged = slots
            .get(&identity.slot)
            .ok_or("cipher output unavailable")?;
        if &staged.identity != identity {
            return Err("cipher output unavailable");
        }
        let mut staged = slots
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
        if identity.registration_id != self.registration_id
            || identity.consumer_id != self.consumer_id
            || identity.expires_at_ms == 0
            || request.plaintext.len() > MAX_CIPHER_OUTPUT_BYTES
            || request
                .replacement
                .as_ref()
                .is_some_and(|bytes| bytes.len() > MAX_CIPHER_OUTPUT_BYTES)
        {
            return Err(tinybus::Error::failed("secret cipher output denied"));
        }

        let staged = StagedOutput {
            identity: identity.clone(),
            plaintext: Zeroizing::new(request.plaintext),
            replacement: request.replacement.map(Zeroizing::new),
        };
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| tinybus::Error::failed("secret cipher output denied"))?;
        if slots.contains_key(&identity.slot) {
            return Err(tinybus::Error::failed("secret cipher output denied"));
        }
        slots.insert(identity.slot, staged);
        Ok(SecretCipherDeliveryResponse2 {
            identity: identity.clone(),
            staged: true,
        })
    }
}

#[cfg(test)]
#[path = "cipher_consumer_tests.rs"]
mod tests;
