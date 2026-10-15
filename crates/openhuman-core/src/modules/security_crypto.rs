//! Host adapter for the TinySecurity confidential password, keyring and pairing
//! members (contract 1.14).
//!
//! Only the bus contract is linked. Every call is confidential, bound by the
//! module to the verified `trusted_host`, and fails closed: any transport,
//! attestation or decode failure is an error, never a fallback to host crypto.
//! Messages never carry secret material: errors are fixed strings.

use tinysecurity_bus::{
    names, CallerContext, KeyringAccess2, KeyringConsentRequest2, KeyringConsentResponse2,
    KeyringOperation2, KeyringRequest2, KeyringResponse2, PairingOperation2, PairingRequest2,
    PairingResponse2, PasswordOperation2, PasswordPayload2, PasswordRequest2, PasswordResponse2,
};

use crate::security::keyring_consent::types::PolicyDecision;

const LOG_PREFIX: &str = "[security-crypto]";
/// Argon2id derivation (64 MiB, 3 passes) can queue behind two other
/// derivations and a cold start, so password calls outlive the 2 s policy budget.
const PASSWORD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Legacy `EncryptedPayload` shape accepted by `encryption.*` RPC callers.
pub(crate) type LegacyPayload = PasswordPayload2;

/// Seals `plaintext` under `password` through `PasswordCrypt2`. A supplied
/// `salt` selects the legacy separate-salt layout.
pub(crate) async fn password_encrypt(
    proxy: &tinybus::Proxy,
    password: String,
    plaintext: Vec<u8>,
    salt: Option<Vec<u8>>,
) -> Result<LegacyPayload, String> {
    let response = password_call(
        proxy,
        PasswordOperation2::Encrypt {
            password,
            plaintext,
            salt,
        },
    )
    .await?;
    response
        .payload
        .ok_or_else(|| "TinySecurity PasswordCrypt2 returned no payload".to_string())
}

/// Opens a legacy payload through `PasswordCrypt2`.
pub(crate) async fn password_decrypt(
    proxy: &tinybus::Proxy,
    password: String,
    payload: LegacyPayload,
) -> Result<Vec<u8>, String> {
    Ok(
        password_call(proxy, PasswordOperation2::Decrypt { password, payload })
            .await?
            .bytes,
    )
}

async fn password_call(
    proxy: &tinybus::Proxy,
    operation: PasswordOperation2,
) -> Result<PasswordResponse2, String> {
    log::debug!("{LOG_PREFIX} PasswordCrypt2 call");
    proxy
        .clone()
        .with_timeout(PASSWORD_TIMEOUT)
        .call_confidential(
            names::methods::PASSWORD_CRYPT2,
            (PasswordRequest2 { operation },),
        )
        .await
        .map_err(|_| "TinySecurity PasswordCrypt2 failed".to_string())
}

/// Reads a named secret; `None` only when the backend confirms absence.
pub(crate) async fn keyring_get(
    proxy: &tinybus::Proxy,
    caller: CallerContext,
    purpose: &str,
    name: &str,
) -> Result<Option<Vec<u8>>, String> {
    let response = keyring_call(
        proxy,
        caller,
        purpose,
        KeyringOperation2::Get {
            name: name.to_owned(),
        },
    )
    .await?;
    Ok(response.found.then_some(response.bytes))
}

/// Stores a named secret.
pub(crate) async fn keyring_set(
    proxy: &tinybus::Proxy,
    caller: CallerContext,
    purpose: &str,
    name: &str,
    value: Vec<u8>,
) -> Result<(), String> {
    keyring_call(
        proxy,
        caller,
        purpose,
        KeyringOperation2::Set {
            name: name.to_owned(),
            value,
        },
    )
    .await
    .map(|_| ())
}

/// Removes a named secret; returns whether one existed.
pub(crate) async fn keyring_delete(
    proxy: &tinybus::Proxy,
    caller: CallerContext,
    purpose: &str,
    name: &str,
) -> Result<bool, String> {
    Ok(keyring_call(
        proxy,
        caller,
        purpose,
        KeyringOperation2::Delete {
            name: name.to_owned(),
        },
    )
    .await?
    .found)
}

async fn keyring_call(
    proxy: &tinybus::Proxy,
    caller: CallerContext,
    purpose: &str,
    operation: KeyringOperation2,
) -> Result<KeyringResponse2, String> {
    log::debug!("{LOG_PREFIX} KeyringAccess2 call");
    proxy
        .call_confidential(
            names::methods::KEYRING_ACCESS2,
            (KeyringRequest2 {
                caller,
                purpose: purpose.to_owned(),
                operation,
            },),
        )
        .await
        .map_err(|_| "TinySecurity KeyringAccess2 failed".to_string())
}

/// One pairing operation on an account registered at module init.
pub(crate) async fn pairing_call(
    proxy: &tinybus::Proxy,
    account: &str,
    operation: PairingOperation2,
) -> Result<PairingResponse2, String> {
    log::debug!("{LOG_PREFIX} PairingControl2 call");
    proxy
        .call_confidential(
            names::methods::PAIRING_CONTROL2,
            (PairingRequest2 {
                account: account.to_owned(),
                operation,
            },),
        )
        .await
        .map_err(|_| "TinySecurity PairingControl2 failed".to_string())
}

/// Decides one `KeyringConsent2.Consent` request from the host's keychain
/// consent state. Consent UX stays host-side: when the OS keyring is
/// unavailable and no answer was recorded, the existing consent-required event
/// is published and this request is denied.
pub(crate) fn keyring_consent_response(
    request: &KeyringConsentRequest2,
    decision: PolicyDecision,
) -> KeyringConsentResponse2 {
    let allowed = matches!(decision, PolicyDecision::Proceed);
    log::debug!(
        "{LOG_PREFIX} keyring consent access={} allowed={allowed}",
        match request.access {
            KeyringAccess2::Read => "read",
            KeyringAccess2::Write => "write",
            KeyringAccess2::Delete => "delete",
        }
    );
    KeyringConsentResponse2 { allowed }
}

/// The `Consent` callback body: the host consent policy applied to a request.
pub(crate) fn handle_keyring_consent(request: &KeyringConsentRequest2) -> KeyringConsentResponse2 {
    keyring_consent_response(
        request,
        crate::security::keyring_consent::policy::check_secret_access(),
    )
}

#[cfg(test)]
#[path = "security_crypto_tests.rs"]
mod tests;
