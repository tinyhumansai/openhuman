//! Calling the `tinywallet` module.
//!
//! The host half of `ai.tinyhumans.tinywallet.Wallet`. Two ways to get a signed
//! transaction, and which one a chain uses is a property of that chain.
//!
//! # The confidential flow — preferred, and what BTC, EVM and Tron use
//!
//! [`sign_transaction_in_module`] sends the recovery phrase itself, once, and
//! the module derives, encodes, signs and assembles. No private key is ever
//! reassembled in this process.
//!
//! ```text
//!   host  ==SignTransaction{phrase, path, fields}==>  module   (confidential)
//!   host  <=={raw transaction, txid}================  module
//! ```
//!
//! The phrase only goes to a recipient tinybus has attested, and
//! [`attested_proxy`] additionally insists the attested digest is one
//! `registry.rs` pinned — see there for why both checks are worth having.
//!
//! # The split flow — still here, still correct for some hosts
//!
//! [`sign_transaction`] keeps the key in this process: the module returns
//! digests, this process signs them, the module reassembles.
//!
//! ```text
//!   host  --BuildUnsigned{fields, public key}-->  module
//!   host  <--[digests to sign]-------------------  module
//!   host    (signs locally, with a key the module never sees)
//!   host  --AttachSignature{fields, signatures}->  module
//!   host  <--{raw transaction, txid}-------------  module
//! ```
//!
//! It is not deprecated. It is the only option for a backend reached across a
//! transport, where the bus cannot say what is on the other end, and it is what
//! Solana still uses here — Solana hand-builds SPL messages that
//! `TransactionSpec::Solana` does not model, so there is nothing to send.
//!
//! # What the confidential flow does and does not buy
//!
//! A loaded module shares this address space and could read the phrase out of
//! process memory whichever flow is used, so neither is a hard isolation
//! boundary and neither is claimed as one.
//!
//! What changes is that this binary no longer performs derivation or signing at
//! all — the key exists only inside the module, for the duration of one call,
//! rather than being assembled here and held across two round trips. The bus
//! also refuses to carry the phrase to anything that is not an allowlisted,
//! hash-verified module, which the split flow could not express.
//!
//! # The fields are sent twice in the split flow, deliberately
//!
//! `AttachSignature` re-sends everything `BuildUnsigned` was given rather than a
//! handle to something the module remembered. That is what lets the module hold
//! no state between the calls — no store, no bound on it, no expiry for a host
//! that never comes back. Building is deterministic, so the module rebuilds the
//! transaction the digests were computed over. The confidential flow needs none
//! of this: it is one call.
//!
//! # Two signing schemes, and the difference matters
//!
//! A `Secp256k1Prehash` payload is **already hashed** and must be signed with a
//! prehash entry point; hashing it again produces a valid signature over the
//! wrong thing. An `Ed25519` payload is the whole message and must **not** be
//! pre-hashed, because ed25519 hashes internally. The module tags every payload
//! with which it is, and [`sign_payload`] dispatches on the tag rather than on
//! the chain — so a chain that changes scheme cannot silently sign wrongly.

use serde::de::DeserializeOwned;
use serde::Serialize;
use tinywallet_bus::names::methods;
use tinywallet_bus::wire::{
    DerivedAccount, ExportRequest, ExportedKey, Scheme, SecretMaterial, SignMessageRequest,
    SignRequest, Signature, SignedTransaction, TransactionSpec,
};

use super::{host, ops, registry};
use crate::config::Config;

/// Registry id of the module these calls go to.
const MODULE_ID: &str = "tinywallet";

/// Why a wallet call did not produce a signed transaction.
///
/// Three variants because the wallet tools map them onto three different
/// user-facing outcomes: something the caller can correct, something it cannot,
/// and a capability that is not present on this host at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalletCallError {
    /// The module is not loaded and cannot be: unsupported host, downloads off,
    /// disabled in config, or a load that already failed in this process.
    Unavailable(String),
    /// The request was rejected. The caller can act on this.
    InvalidInput(String),
    /// Building, signing, or assembly failed.
    Failed(String),
}

impl std::fmt::Display for WalletCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::InvalidInput(message) | Self::Failed(message) => {
                f.write_str(message)
            }
        }
    }
}

/// Derive, build, sign and assemble entirely inside the module.
///
/// The counterpart to [`sign_transaction`], and the one to prefer: the phrase
/// crosses the bus once and no key material is ever reassembled here.
///
/// # Errors
///
/// [`WalletCallError`]. `Unavailable` if the module is not an attested
/// recipient — see [`attested_proxy`], which refuses to send the phrase at all
/// in that case.
pub async fn sign_transaction_in_module(
    config: &Config,
    transaction: &TransactionSpec,
    secret: &SecretMaterial,
) -> Result<SignedTransaction, WalletCallError> {
    let proxy = attested_proxy(config).await?;
    log::debug!(
        "[modules:wallet] sign_transaction chain={:?} module={MODULE_ID} (confidential)",
        transaction.chain()
    );
    proxy
        .call_confidential(
            methods::SIGN_TRANSACTION,
            (SignRequest {
                secret: secret.clone(),
                transaction: transaction.clone(),
            },),
        )
        .await
        .map_err(|error| classify(&error))
}

/// Ask the module for an address and public key.
///
/// # Errors
///
/// [`WalletCallError`].
pub async fn derive_account(
    config: &Config,
    secret: &SecretMaterial,
) -> Result<DerivedAccount, WalletCallError> {
    let proxy = attested_proxy(config).await?;
    log::debug!(
        "[modules:wallet] derive_account chain={:?} module={MODULE_ID} (confidential)",
        secret.chain
    );
    proxy
        .call_confidential(methods::DERIVE_ACCOUNT, (secret.clone(),))
        .await
        .map_err(|error| classify(&error))
}

/// Sign opaque bytes with the key derived from `secret`.
///
/// Blind: the module cannot check what the bytes mean, so
/// [`sign_transaction_in_module`] is the right call wherever the request can be
/// expressed as a `TransactionSpec`. This exists for the two encodings the wire
/// contract does not model — Solana SPL transfers and x402 payments — where the
/// alternative is not a verified signature but deriving the key in this process
/// and signing here, which is what these callers used to do.
///
/// # Errors
///
/// [`WalletCallError`].
pub async fn sign_message(
    config: &Config,
    secret: &SecretMaterial,
    message: &[u8],
    scheme: Scheme,
) -> Result<Signature, WalletCallError> {
    let proxy = attested_proxy(config).await?;
    log::debug!(
        "[modules:wallet] sign_message chain={:?} bytes={} module={MODULE_ID} (confidential)",
        secret.chain,
        message.len()
    );
    proxy
        .call_confidential(
            methods::SIGN_MESSAGE,
            (SignMessageRequest {
                secret: secret.clone(),
                message_hex: hex(message),
                scheme,
            },),
        )
        .await
        .map_err(|error| classify(&error))
}

/// Ask the module for the raw derived key.
///
/// A compatibility call for downstream hosts that must drive a signer locally.
/// OpenHuman's own wallet paths sign inside the module.
///
/// # Errors
///
/// [`WalletCallError`].
pub async fn export_key(
    config: &Config,
    secret: &SecretMaterial,
) -> Result<ExportedKey, WalletCallError> {
    let proxy = attested_proxy(config).await?;
    log::debug!(
        "[modules:wallet] export_key chain={:?} module={MODULE_ID} (confidential)",
        secret.chain
    );
    proxy
        .call_confidential(
            methods::EXPORT_KEY,
            (ExportRequest {
                secret: secret.clone(),
            },),
        )
        .await
        .map_err(|error| classify(&error))
}

/// Call a typed Wallet, Web3, or x402 member with one request argument.
/// Business rules stay in the module; the host only transports the request.
pub async fn call<Request, Reply>(
    config: &Config,
    member: &str,
    request: Request,
) -> Result<Reply, WalletCallError>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    let (runtime, record) = ready(config).await?;
    proxy(runtime, record)?
        .call::<Reply>(member, (request,))
        .await
        .map_err(|error| classify(&error))
}

/// Call a Wallet member carrying secret material over TinyBus's confidential
/// transport. The loaded module must be attested and match this build's pin.
pub async fn call_confidential<Request, Reply>(
    config: &Config,
    member: &str,
    request: &Request,
) -> Result<Reply, WalletCallError>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    attested_proxy(config)
        .await?
        .call_confidential::<Reply>(member, (request,))
        .await
        .map_err(|error| classify(&error))
}

/// A proxy that has proved it is the artifact this build pinned.
///
/// # Why check here when the broker already refuses
///
/// The broker will not route a confidential call to an unattested recipient, so
/// omitting this would still be safe against the case it covers. Two reasons to
/// check anyway.
///
/// The first is the error. A broker refusal arrives after the request has been
/// serialized, which means a frame containing the recovery phrase was built and
/// handed to the bus before anything said no. Checking first means the phrase is
/// never put into a buffer on a call that was always going to fail.
///
/// The second is that this compares the digest against **this build's own
/// table**, which the broker cannot do — it only knows the host vouched for
/// something. Here we can insist it vouched for one of the artifacts
/// `registry.rs` names. That closes the gap where a host is somehow induced to
/// attest a different artifact for this bus name.
///
/// Both are belt-and-braces over a check that already exists. That is the right
/// posture for the one code path that hands over a recovery phrase.
async fn attested_proxy(config: &Config) -> Result<tinybus::Proxy, WalletCallError> {
    let (runtime, record) = ready(config).await?;
    let proxy = proxy(runtime, record)?;

    let attestation = proxy
        .attestation()
        .await
        .map_err(|error| WalletCallError::Failed(error.to_string()))?
        .ok_or_else(|| {
            WalletCallError::Unavailable(
                "the wallet module is loaded but not an attested recipient, so it cannot be sent \
                 key material. This host is running a build whose module loader does not record \
                 an attestation for pinned releases; upgrade it rather than working around this."
                    .to_string(),
            )
        })?;

    if attestation.name.as_str() != record.bus_name {
        return Err(WalletCallError::Failed(format!(
            "the wallet module's attestation names '{}' rather than '{}'",
            attestation.name.as_str(),
            record.bus_name
        )));
    }

    if !digest_is_pinned(record, &attestation.sha256) {
        return Err(WalletCallError::Failed(
            "the loaded wallet module is not one of the artifacts this build pinned, so it will \
             not be sent key material"
                .to_string(),
        ));
    }

    Ok(proxy)
}

// Library checksums verified against modules.toml and the library bytes in each
// registry-pinned TinyWallet v0.13.0 archive. A library is trusted only while
// its corresponding archive remains pinned in the supplied ModuleRecord.
const PINNED_WALLET_LIBRARIES: &[(&str, &str)] = &[
    // ubuntu-24.04-x86_64
    (
        "284983d01c548109b3e9d5bbca1951787912f713b4a98920b9b45cfd034b6760",
        "f9a13c65e14b7cf9de57beeb40e8753a212234795a75fe8d8d11564a4ccabb67",
    ),
    // ubuntu-24.04-arm64
    (
        "18ef8ffe518f731dc594d1606554ca12c840219e54f230382937061c395ab860",
        "66961c39b0e2ebcb3c69ab1f6c1aa7cb59a830d787565b7fbb91339ca594d644",
    ),
    // ubuntu-22.04-x86_64
    (
        "92d0d0fe9ab20fdf4c44aa58d762b7e28b6a08876899a5b2d368df3feabcd2d5",
        "7b6fdd843622a29b610633e0f5cabf6f48c8aa7566a466696916e99d1221eb08",
    ),
    // ubuntu-22.04-arm64
    (
        "4d0e17d778c0c84df17e4c5b7fa60aa9296efecc5726902008946409b3a3c9e1",
        "6f91352fbf5bdf69b7c76ae33f100b67320e3239573d4bb5402ba21ed51e7680",
    ),
    // macos-26-arm64
    (
        "e01a70efbcece8e134abfa84c70181425802415525303d7ba2a7798afbab1041",
        "a8181f4a927f86182fadc4c65f12854cf7f719dc57b00ac1ff7da2e0a1ecc5b4",
    ),
    // macos-26-x86_64
    (
        "c48f1f7807d0b25c2b54dca97237f6e64d02ec3a9396370cd8e3b1857419296a",
        "684f2d74d3abaca8712eddafa0e7dd67ddfa9fa64ca03427959a03d20e31e3c0",
    ),
    // macos-15-arm64
    (
        "95f69f2e17ff1785d032f6e1017e12de5c6dc09af9633e67fc9c225dfcef9ce6",
        "39f3674672bfaef485838eba98a60fb21c173d5f161449abc0dacc4c2a8aa8ff",
    ),
    // macos-15-x86_64
    (
        "f3114694e2d666ea1b030db952c589f08d79c60ab79aadb844327e273c5faaf5",
        "3ed1c36990a6fde7710d6616f7ddc2d68d0ed029c73f93aef859a0a023ed5e4b",
    ),
    // windows-2025-x86_64
    (
        "7b79a25a1fccef17a4c6e0d400f49646c3a5f321821bb03ddc23e76d6030717a",
        "66278854fe0843d46f3029baa9e5deccaf9fa20d757eea20ceae321f3ee72310",
    ),
    // windows-2022-x86_64
    (
        "0b7126be04b23dd5fad883d7f1dd33dd65c1042af68f93593590ea07c5daa95e",
        "b547c227e56400f92ffd21829d510f455e4be446d049106e7490039919acb267",
    ),
    // windows-11-arm64
    (
        "483d3fda2c46de71c8efd5dfa4ec828c0b6c86bb164c8362c1356f85cb8ec231",
        "ee10089c09b5e3b5f178484816fc90f297ad7c23e209d3281a10e0668d0aeaca",
    ),
    // archlinux-arm64
    (
        "ad2222bd1d2dd680568f486e40389c0b963cf5ba2d170f9d658985f31cabb511",
        "66766056becbb6d55078952989ae542c88758075bae58ce5e9c9294b7712701e",
    ),
    // archlinux-x86_64
    (
        "d67f541330f7712106238b90d993550d2e2e55f33d3c8c9f50669a1a51c1e981",
        "725a256ded1ee0f2bc0f6360f1dac84f94423b8816162c4202b6c71d2194f1b1",
    ),
    // fedora-43-arm64
    (
        "a2d62b03f2e99008ee3fe7ea84290fd050e12016ce36e89fda24d5449e795261",
        "3fb82b1ae8dbce52999be065b8e64bcc3668d056a2fba90a5f6f067343ea6161",
    ),
    // fedora-43-x86_64
    (
        "0c8c5dbf2aa7b7caa371aa6366c7cfdd232fda41e2dbd894d2ef344c7c2cfa0a",
        "7be79421594597b6fc8b9c15a16a7df3218f8d78651a3ac85153f6cb88d6532e",
    ),
    // fedora-44-arm64
    (
        "272b535910fc4bc84099887b7ccfbdd9d41a6d187616086c174f86cb77f21757",
        "09226fb7f644746cd9ed12523ffc08962be035d5a2f1d5004f5e1bc70d74cb0d",
    ),
    // fedora-44-x86_64
    (
        "3dcad60a65d4f0bbb1af838468a2f52d648d22b2fbc849ae0548a30ed9806646",
        "b6eab1fc5076a55764cb44a04e500cc52928604a8c1b42ed9426e4ba9966ba4b",
    ),
];

/// Whether the attested archive or library belongs to a currently pinned release.
///
/// The release loader attests the verified archive; local discovery attests the
/// library from modules.toml. Library checksums therefore require their paired
/// archive to remain in this record, so retiring a release also retires its
/// libraries. Check every platform because the attestation carries no host key.
fn digest_is_pinned(record: &super::ModuleRecord, sha256: &str) -> bool {
    record.assets.iter().any(|asset| {
        asset.sha256.eq_ignore_ascii_case(sha256)
            || PINNED_WALLET_LIBRARIES.iter().any(|(archive, library)| {
                asset.sha256.eq_ignore_ascii_case(archive) && library.eq_ignore_ascii_case(sha256)
            })
    })
}

/// Load the wallet module if it is not already serving.
///
/// Callers do not have to invoke this — the signing calls do — but a caller
/// that wraps its work in a deadline should, *outside* that deadline. A first
/// use may download and verify an artifact, and charging that against a
/// transaction timeout means the first transfer a user ever makes is the one
/// that fails.
///
/// # Errors
///
/// [`WalletCallError::Unavailable`].
pub async fn ensure_ready(config: &Config) -> Result<(), WalletCallError> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(WalletCallError::Unavailable)
}

/// Ensure the module is serving and hand back what a call needs.
async fn ready(
    config: &Config,
) -> Result<(&'static host::ModuleRuntime, &'static super::ModuleRecord), WalletCallError> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(WalletCallError::Unavailable)?;
    let record = registry::find(MODULE_ID)
        .ok_or_else(|| WalletCallError::Unavailable(format!("unknown module '{MODULE_ID}'")))?;
    let runtime = host::runtime()
        .await
        .map_err(|_| WalletCallError::Unavailable("the module bus is not running".to_string()))?;
    Ok((runtime, record))
}

/// A proxy for the module's object.
fn proxy(
    runtime: &'static host::ModuleRuntime,
    record: &super::ModuleRecord,
) -> Result<tinybus::Proxy, WalletCallError> {
    runtime
        .proxy(record.bus_name, record.object_path)
        .map_err(|error| WalletCallError::Failed(error.to_string()))
}

/// Map a bus failure onto the shape a caller can act on.
///
/// The wire name is the contract; the message is for a human. An unrecognised
/// name is `Failed` rather than `InvalidInput`, because telling a caller its
/// input was wrong when it was not points at the wrong fix.
fn classify(error: &tinybus::Error) -> WalletCallError {
    let message = error.to_string();
    match error.wire_name() {
        "ai.tinyhumans.tinywallet.Error.InvalidInput" => WalletCallError::InvalidInput(message),
        "ai.tinyhumans.tinywallet.Error.UnsupportedChain" => WalletCallError::Unavailable(message),
        // The module is loaded but not answering: refused, faulted, or gone.
        name if name.contains("ModuleUnavailable") => WalletCallError::Unavailable(message),
        _ => WalletCallError::Failed(message),
    }
}

/// Lowercase hex, unprefixed — the form every field in the wire contract uses.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
#[path = "wallet_tests.rs"]
mod tests;
