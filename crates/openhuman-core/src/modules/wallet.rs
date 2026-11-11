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
    DeriveMnemonicSeedRequest, DeriveMnemonicSeedResponse, DerivedAccount,
    EncodeErc20TransferRequest, EncodeErc20TransferResponse, ExportRequest, ExportedKey,
    GenerateMnemonicRequest, GenerateMnemonicResponse, Scheme, SecretMaterial, SignMessageRequest,
    SignRequest, Signature, SignedTransaction, TransactionSpec,
};
use zeroize::Zeroize;

use super::{ops, registry};
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
    log::debug!(
        "[modules:wallet] derive_account chain={:?} module={MODULE_ID} (confidential)",
        secret.chain
    );
    let mut request = secret.clone();
    let result = call_confidential(config, methods::DERIVE_ACCOUNT, &request).await;
    request.mnemonic.zeroize();
    result
}

/// Generate a recovery phrase inside the attested TinyWallet module.
///
/// # Errors
///
/// [`WalletCallError`]. The phrase is returned to the host for encryption and
/// custody, but generation and BIP-39 vocabulary remain module-owned.
pub async fn generate_mnemonic(
    config: &Config,
    word_count: u8,
) -> Result<GenerateMnemonicResponse, WalletCallError> {
    call_confidential(
        config,
        methods::GENERATE_MNEMONIC,
        &GenerateMnemonicRequest { word_count },
    )
    .await
}

/// Derive a standard BIP-39 seed inside the attested module.
///
/// The caller uses the seed only for OpenHuman's generic storage-key KDF.
///
/// # Errors
///
/// [`WalletCallError`]. Invalid or unsupported phrases are classified as
/// [`WalletCallError::InvalidInput`].
pub async fn derive_mnemonic_seed(
    config: &Config,
    mnemonic: String,
) -> Result<DeriveMnemonicSeedResponse, WalletCallError> {
    let mut request = DeriveMnemonicSeedRequest { mnemonic };
    let result = call_confidential(config, methods::DERIVE_MNEMONIC_SEED, &request).await;
    request.mnemonic.zeroize();
    result
}

/// Encode ERC-20 transfer calldata with the module-owned ABI implementation.
///
/// # Errors
///
/// Returns [`WalletCallError`] if the module is unavailable or rejects the
/// recipient or base-unit amount.
pub async fn encode_erc20_transfer(
    config: &Config,
    to_address: &str,
    amount_raw: &str,
) -> Result<String, WalletCallError> {
    let response: EncodeErc20TransferResponse = call(
        config,
        methods::ENCODE_ERC20_TRANSFER,
        &EncodeErc20TransferRequest {
            to_address: to_address.to_owned(),
            amount_raw: amount_raw.to_owned(),
        },
    )
    .await?;
    Ok(response.calldata)
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

// Embedded library checksums verified from all 17 archives in the
// registry-pinned TinyWallet v0.29.0 release. Each library is trusted only
// while its archive checksum remains pinned in the supplied ModuleRecord.
const PINNED_WALLET_LIBRARIES: &[(&str, &str)] = &[
    // archlinux-arm64
    (
        "90d8cb352a9d6b6403549f053446139c0361d36354ca7c37e0bc0296eb544cae",
        "afc105b22f2164c44b8c2b8c2f47fca18b15d3fa3f41dd32068323fef61c961d",
    ),
    // archlinux-x86_64
    (
        "8e9bc6e2d443807cbbb1759cb85896abbcc0d3294a0ec58007fe0c56aecb5a25",
        "06437156a130c5a31b855e8da09ed0e523bf284868f3be98a801a4e48e95fe99",
    ),
    // fedora-43-arm64
    (
        "71e8ec37af5be258c111169e441fed5f4a46d9b3934c477569ec1185ec7ceba5",
        "eee7d2940b7454e55a1a1dbdfaa019547be64ca0dac35a8defb675697a255d70",
    ),
    // fedora-43-x86_64
    (
        "3e7438212776adbe82a54a5c48ff744cd26ee73d0dee66bce7de42b968608a16",
        "bd09a11778799d091c01889e662e0b73105564f3aed87bd6f6cc5ba646c7c5eb",
    ),
    // fedora-44-arm64
    (
        "7612aeeca130c741b38673279268494e0cf49891dd95574e14310e64634a6840",
        "85d36fc9cb09d5115bcab86dab57841c3e4f42f3b7dc7370e9714123ff0da4ae",
    ),
    // fedora-44-x86_64
    (
        "9807227a79989d6c17be18212462751e244fb1b5f2bbe1224e0054b3f37cc188",
        "d2bd08acfcfba51dbbbc0c98e30ee239c6b96f602804e51e1346308904a59ccf",
    ),
    // macos-15-arm64
    (
        "228b25d3ea7869436cf009bc228dc94f8f40fcf1d7ea81c622651c14b6bbc216",
        "b0b1c14b81d53e9dad0d542b904c2a13bdd006bd0aa4161efdb7908b5dfd1018",
    ),
    // macos-15-x86_64
    (
        "0b173d7ebed6ce23c7224a55a32c9b9716471a054119b02869abc2add15dc391",
        "c1950a184ff12d06fcbaab8ddc885634de27a8f77079d2fd4b23bd8ed769bfc4",
    ),
    // macos-26-arm64
    (
        "61ee2de3ca95cea7f8253de3e47f4da9ae42bd4ac406c65374aaf4cd22b742f3",
        "cf525f878ee525fbc2455c5973f3fb87250ac7c5e1e10bf7da06bf987ac53ac1",
    ),
    // macos-26-x86_64
    (
        "5c7593c4bb15f806a83c25dee97e26bf86a73571046d6523164885804d0c9c7d",
        "8dcfe0bc479614641401f666520bd557ed3908f62765f0730ed6cacbbb592aaa",
    ),
    // ubuntu-22.04-arm64
    (
        "54085d6e1391f98fb44b86735df0f147318502153c4d3e96c5586421835cb5aa",
        "71be33a596e9a7bc85ba27c39d03dfe50dfde82d30520bed9206fcd64d82d86a",
    ),
    // ubuntu-22.04-x86_64
    (
        "4d2016fac3c2001dae9ff152dad91057741aa6c031093b10fce4a4f3e1654045",
        "11ba204877de877aa04286c98d4748fbcdbbcdc6b0fd8e24db0632ea96c4910b",
    ),
    // ubuntu-24.04-arm64
    (
        "dedb29838c947d2da479c9998a38d772de875bd914eae0d0a8b81377bb7b894c",
        "ebbe1494be6ba647c0d6b3162b6c830220fe87f3faf46d179369782fc3bb7b5b",
    ),
    // ubuntu-24.04-x86_64
    (
        "74036ca124938a556687ff760d83fb3ad2eebb9e91a2a9cb93873a537f8e85a3",
        "ca77309f2d5cdb6d13e56cb7fc44145507a333f55c6a31729979a2782a4ab3ae",
    ),
    // windows-11-arm64
    (
        "5fd451a1166e64af8f5936239f22ca88deadb6ff96058e7b9295887fa8315999",
        "24971b8592e3907fe09968abbf4c723d539559a534a89f9cfd2c64b82a218de9",
    ),
    // windows-2022-x86_64
    (
        "b272c66ade706b7cb3c443fbd05c549abc2ef024f044b1cdfeba19f61367db78",
        "553236685e6f444902302f34a1a4f6e02991d4e07360afcb69594a934e974f38",
    ),
    // windows-2025-x86_64
    (
        "2a6b49b81af30bbe6975c7274a9c2f8cef2a3bde10a3cebae1f4c8b9c1bf5333",
        "44c8d1070b3aab490b9b928036f20a42eee9d85e6dabd3865ba4cf3381ed6d2f",
    ),
];

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
) -> Result<
    (
        &'static tinybus::module::ModuleRuntime,
        &'static super::ModuleRecord,
    ),
    WalletCallError,
> {
    ops::ensure_loaded(config, MODULE_ID)
        .await
        .map_err(WalletCallError::Unavailable)?;
    let record = registry::find(MODULE_ID)
        .ok_or_else(|| WalletCallError::Unavailable(format!("unknown module '{MODULE_ID}'")))?;
    let runtime = super::runtime::runtime()
        .await
        .map_err(|_| WalletCallError::Unavailable("the module bus is not running".to_string()))?;
    Ok((runtime, record))
}

/// A proxy for the module's object.
fn proxy(
    runtime: &'static tinybus::module::ModuleRuntime,
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
