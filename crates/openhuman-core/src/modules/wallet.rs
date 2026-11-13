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
//! `registry.rs` pinned, or this executable's own when the wallet is linked
//! into it — see there for why both checks are worth having.
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
///
/// # Linked code is accepted as host code
///
/// A wallet linked into the executable (`[modules] source = "linked"`, built
/// through `openhuman-modules-linked`) has no release artifact. tinybus attests
/// it with the digest of the executable that contains it, so it can never match
/// a release pin, and refusing it would make a linked wallet unusable for the
/// very calls linking exists to serve. That code is part of this binary: it is
/// exactly as trusted as the core making this check, and anything able to swap
/// it could swap the check too. So the running executable's own digest is also
/// accepted, but only when this binary registered a linked tinywallet factory
/// (see [`linked_host_code`]). The name must still match, and any digest that is
/// neither pinned nor this executable's is still refused.
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

    admit_attestation(record, &attestation, linked_host_code())?;
    Ok(proxy)
}

/// The decision [`attested_proxy`] makes once it holds an attestation.
///
/// `host_code` is the running executable's digest when this binary links its
/// own tinywallet, and `None` otherwise (see [`linked_host_code`]).
fn admit_attestation(
    record: &super::ModuleRecord,
    attestation: &tinybus::attest::Attestation,
    host_code: Option<&str>,
) -> Result<(), WalletCallError> {
    if attestation.name.as_str() != record.bus_name {
        return Err(WalletCallError::Failed(format!(
            "the wallet module's attestation names '{}' rather than '{}'",
            attestation.name.as_str(),
            record.bus_name
        )));
    }

    if digest_is_pinned(record, &attestation.sha256) {
        return Ok(());
    }

    if host_code.is_some_and(|digest| digest.eq_ignore_ascii_case(&attestation.sha256)) {
        log::debug!("[modules:wallet] attestation admitted as linked host code module={MODULE_ID}");
        return Ok(());
    }

    Err(WalletCallError::Failed(
        "the loaded wallet module is not one of the artifacts this build pinned, so it will not \
         be sent key material"
            .to_string(),
    ))
}

/// The digest a wallet linked into this executable is attested with, if this
/// binary links one.
///
/// Only a binary that registered a tinywallet factory
/// (`openhuman-modules-linked::install`) can serve the wallet as linked code,
/// so only such a binary recognizes the executable's digest. tinybus documents
/// that a sender of a secret must recognize linked registration explicitly
/// before accepting that digest (`Attestation::sha256`); this is that check. In
/// every other build this is `None` and the executable is never hashed.
fn linked_host_code() -> Option<&'static str> {
    if !super::source::linked_factory_available(MODULE_ID) {
        return None;
    }
    host_executable_digest()
}

/// SHA-256 of the running executable, as tinybus attests linked code.
///
/// `ModuleHost::attach_linked_with_config` hashes `std::env::current_exe()`
/// with `tinybus::module::sha256_file` and records that as the linked module's
/// attestation. This is the same function on the same file, so the two agree
/// on encoding by construction. Hashed once per process, like tinybus does; a
/// failure is not cached, also like tinybus, so a transient read error does not
/// disable the wallet for the life of the process. A failure leaves `None`,
/// which fails closed.
fn host_executable_digest() -> Option<&'static str> {
    static DIGEST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(digest) = DIGEST.get() {
        return Some(digest.as_str());
    }
    let executable = std::env::current_exe().ok()?;
    let digest = tinybus::module::sha256_file(executable).ok()?;
    Some(DIGEST.get_or_init(|| digest).as_str())
}

// Embedded library checksums verified from all 17 archives in the
// registry-pinned TinyWallet v0.30.1 release. Each library is trusted only
// while its archive checksum remains pinned in the supplied ModuleRecord.
const PINNED_WALLET_LIBRARIES: &[(&str, &str)] = &[
    // archlinux-arm64
    (
        "f49630c0433fbdf4c048974904731118d2b633617753048e1b7c6818a24de719",
        "aa0140c9d8a11ebfb5120907a960e68a94db4f3033ded683559abc7f37a1a009",
    ),
    // archlinux-x86_64
    (
        "e2d9e21d437f222c848f52af975b0764106486cc2595b55e6d6b27924dd1641b",
        "855d03d856389628e386bd5393359ce10d99e056d31ce558530a28cfb763eeeb",
    ),
    // fedora-43-arm64
    (
        "51284db3fe59f60d28d09f3f8d1802539a478d7edf113d787fbf759d5ff019f3",
        "e90756c0ab31f903f7492ae4c7083679390914311901ca87b8a27b0f6fe7ff72",
    ),
    // fedora-43-x86_64
    (
        "320710a3cb9bfbf24a095234e2bc7bd06c3ba605571474979b671b6e4deae168",
        "9fd054fff142c2f4df537cb7d34a3a4908234a9cf2df139d984d02ccc5220b82",
    ),
    // fedora-44-arm64
    (
        "49f0b1c6dbb454eb1aa9abd7f51e99fad6c92e245c6de9550ad95acd8012c14f",
        "1919bfa167392426ebd00ce6f67aac13f17541ff9f866a24e71c4b3da2c8cdeb",
    ),
    // fedora-44-x86_64
    (
        "3dd7588bcf6b2b5ad4720892f9513a12fd6e5f4b7c0be418b4bcf3e808ddfb4e",
        "72eb44021874d1ffecfa3f665e6a41d9e3088f1791a967d02b88dfe21789e5d2",
    ),
    // macos-15-arm64
    (
        "8577f4fe71cac23e37e7aaa9d60a328e3754a5df488540fdc55069d36d67a5e7",
        "a55e68e3f92ad30f7339058bf6e548cb5af028d3caad7acb2fdd34bb7ce8967c",
    ),
    // macos-15-x86_64
    (
        "c9be1266337f0272162906d846f6d2cd71106f6c4b1df50692c0b89c8c438bc4",
        "e50a4ad3bd2a23dc10c2e6e6223d45e7001e9926f4ac5168199f857ddecb0974",
    ),
    // macos-26-arm64
    (
        "e53d8d5c44d7e22e2dfcc0108edec488bb957f2b0acd882c28013d3cc0bb82ea",
        "b1edb8cdd35ee165a2ea6ca50b0b46993b746615f198ff63e51d3cc7bf6ea910",
    ),
    // macos-26-x86_64
    (
        "e5886fa75211d9fc64da2d8d8c1ce58761c1f31050481a75d53153372599815f",
        "3b0098d4c9f80d7bde177eb001fa3eee6a715195e71dc60476845b9fb4280135",
    ),
    // ubuntu-22.04-arm64
    (
        "7d56f1ccb7ea84335a4371e18484cb73aad1ff2fd744bebb155fa6ac93117ea1",
        "776063a16cce8fde0229ce83a5102ceb4ddc5689be83274aac0ed51fe82b4b59",
    ),
    // ubuntu-22.04-x86_64
    (
        "373f4d317ccdc6f1b5c12849ee5ddac03591b48b15f7ca9016222ea17851b720",
        "607df407982522a1b5e16f6348600f0ddf0923781be79c5f2275e8a23615fad2",
    ),
    // ubuntu-24.04-arm64
    (
        "a96ccc6975a7f69d310a752515ed3fd52cdee9d84bf8c686256b5cd881c95cb5",
        "d98391ae2e75361b3516d9da2ebb317d4c1de26e056c6399ec50b83990949975",
    ),
    // ubuntu-24.04-x86_64
    (
        "a90c058cf549f28c9092c629299a3bc7c8ed99114b11fad34ac30628d1de4fe1",
        "6f073bc8c508a18ecbaeddf9291a336a1c492323f5881db06d371dfe1572d41f",
    ),
    // windows-11-arm64
    (
        "fbadd74049901eb74a2741ad8af28241f5a856445d6dd65906bef8d314287afc",
        "ab1237366422a729e5559ec5dcb72cb97e913cd2da56c8746a17680632f1d7ea",
    ),
    // windows-2022-x86_64
    (
        "0e5f7ef529019c70939862c16944648145cb7aa8cb67007f4aafa50c27371d52",
        "1d5e3ac4ac387be032c1e443c23ceaa8919d1c4a7033a45da6f800dd6daf8428",
    ),
    // windows-2025-x86_64
    (
        "4e9b92a6c1480410fe0296c6d4905cd0234f169b2cb6db69042deab57a881323",
        "e617b7fc9eed7f152ac037d536e36591102e8d2c9dbe5f1257d607a85ed306f4",
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
