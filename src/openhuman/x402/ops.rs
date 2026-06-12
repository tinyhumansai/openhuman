//! x402 client operations — parse 402 challenges, build payment transactions
//! (Solana SPL or EVM ERC-20), sign, and retry with proof.
//!
//! Solana `exact` scheme layout:
//!  1. ComputeBudget::SetComputeUnitLimit
//!  2. ComputeBudget::SetComputeUnitPrice
//!  3. SPL Token `TransferChecked`
//!  4. (optional) SPL Memo with `extra.memo` or random nonce
//!
//! EVM `exact` scheme:
//!  EIP-3009 `transferWithAuthorization` signed by the wallet's EVM key.
//!  The facilitator submits the signed authorization on-chain.

use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use log::{debug, warn};
use reqwest::header::HeaderMap;
use sha2::{Digest, Sha256};

use super::types::*;

const LOG_PREFIX: &str = "[x402]";

/// Reasonable compute budget defaults for a single SPL TransferChecked.
const DEFAULT_COMPUTE_UNITS: u32 = 50_000;
const DEFAULT_COMPUTE_UNIT_PRICE: u64 = 1000; // micro-lamports per CU

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// High-level x402 client. Wraps a `reqwest::Client` and knows how to
/// intercept 402 responses, build Solana payments, and retry transparently.
pub struct X402Client {
    http: reqwest::Client,
}

impl X402Client {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// Send a request. If the server returns 402 with a `PAYMENT-REQUIRED`
    /// header, attempt to pay using the wallet's Solana key and retry.
    ///
    /// `signing_key` — the wallet's ed25519 key (caller derives from mnemonic).
    /// `max_amount` — optional ceiling in atomic units; rejects challenges above
    ///                this to prevent runaway spending.
    pub async fn try_paid_request(
        &self,
        request: reqwest::Request,
        signing_key: &SigningKey,
        max_amount: Option<u64>,
    ) -> Result<reqwest::Response, X402Error> {
        let method = request.method().clone();
        let url = request.url().clone();
        let headers = request.headers().clone();
        let body_bytes = request
            .body()
            .and_then(|b| b.as_bytes())
            .map(|b| b.to_vec());

        debug!("{LOG_PREFIX} initial request {} {}", method, url);
        let response = self
            .http
            .execute(request)
            .await
            .map_err(X402Error::Transport)?;

        if response.status() != reqwest::StatusCode::PAYMENT_REQUIRED {
            return Ok(response);
        }

        let challenge = parse_402_headers(response.headers())?;
        debug!(
            "{LOG_PREFIX} got 402 challenge version={} accepts={}",
            challenge.x402_version,
            challenge.accepts.len()
        );

        let (requirement, chain) = challenge
            .best_exact_requirement()
            .ok_or_else(|| X402Error::NoPaymentOption)?;

        let amount: u64 = requirement.amount.parse().map_err(|e| {
            X402Error::Protocol(format!("invalid amount '{}': {e}", requirement.amount))
        })?;

        if let Some(cap) = max_amount {
            if amount > cap {
                return Err(X402Error::AmountExceedsCap {
                    requested: amount,
                    cap,
                });
            }
        }

        debug!(
            "{LOG_PREFIX} paying {} atomic units of {} to {} chain={:?} (fee_payer={:?})",
            amount,
            requirement.asset,
            requirement.pay_to,
            chain,
            requirement.fee_payer_pubkey(),
        );

        let payment = match chain {
            PaymentChain::Solana => {
                build_solana_payment(signing_key, &challenge, requirement).await?
            }
            PaymentChain::Evm => build_evm_payment(&challenge, requirement).await?,
        };
        let encoded = B64.encode(serde_json::to_string(&payment).unwrap());

        let mut retry_req = self.http.request(method, url);
        for (key, value) in headers.iter() {
            retry_req = retry_req.header(key, value);
        }
        retry_req = retry_req.header(HEADER_PAYMENT_SIGNATURE, &encoded);
        if let Some(body) = body_bytes {
            retry_req = retry_req.body(body);
        }

        debug!("{LOG_PREFIX} retrying with payment proof");
        let paid_response = retry_req.send().await.map_err(X402Error::Transport)?;

        if let Some(receipt_header) = paid_response.headers().get(HEADER_PAYMENT_RESPONSE) {
            match parse_settlement_response(receipt_header.to_str().unwrap_or("")) {
                Ok(receipt) => {
                    if receipt.success {
                        debug!(
                            "{LOG_PREFIX} payment settled tx={} network={}",
                            receipt.transaction, receipt.network
                        );
                    } else {
                        warn!(
                            "{LOG_PREFIX} payment settlement failed reason={:?}",
                            receipt.error_reason
                        );
                    }
                }
                Err(e) => warn!("{LOG_PREFIX} could not parse settlement response: {e}"),
            }
        }

        Ok(paid_response)
    }
}

/// Standalone entry point — parse a 402 response's headers and return the
/// challenge with the index of the best payment option and its chain family.
pub fn handle_402(
    headers: &HeaderMap,
) -> Result<(PaymentRequired, usize, PaymentChain), X402Error> {
    let challenge = parse_402_headers(headers)?;
    // Prefer Solana (lower fees, faster finality), fall back to EVM
    let (idx, chain) = challenge
        .accepts
        .iter()
        .enumerate()
        .find(|(_, r)| r.scheme == "exact" && r.network.starts_with("solana:"))
        .map(|(i, _)| (i, PaymentChain::Solana))
        .or_else(|| {
            challenge
                .accepts
                .iter()
                .enumerate()
                .find(|(_, r)| r.scheme == "exact" && r.network.starts_with("eip155:"))
                .map(|(i, _)| (i, PaymentChain::Evm))
        })
        .ok_or(X402Error::NoPaymentOption)?;
    Ok((challenge, idx, chain))
}

/// Build a payment and return the encoded header value ready to attach.
/// Separated from `try_paid_request` so callers that manage their own HTTP
/// layer can still use the payment construction.
pub async fn try_paid_request(
    signing_key: &SigningKey,
    challenge: &PaymentRequired,
    requirement: &PaymentRequirements,
) -> Result<String, X402Error> {
    let chain = if requirement.network.starts_with("eip155:") {
        PaymentChain::Evm
    } else {
        PaymentChain::Solana
    };
    let payment = match chain {
        PaymentChain::Solana => build_solana_payment(signing_key, challenge, requirement).await?,
        PaymentChain::Evm => build_evm_payment(challenge, requirement).await?,
    };
    let json = serde_json::to_string(&payment)
        .map_err(|e| X402Error::Protocol(format!("serialize payment: {e}")))?;
    Ok(B64.encode(json))
}

/// Result of a successful x402 payment retry — the payment header value and
/// metadata for the ledger.
pub struct X402PaymentResult {
    pub header_value: String,
    pub amount_atomic: u64,
    pub asset: String,
    pub recipient: String,
    pub network: String,
    pub url: String,
}

/// End-to-end 402 handler for the HTTP tool layer. Given a 402 response's
/// headers and the original URL:
///
/// 1. Parses the PAYMENT-REQUIRED challenge
/// 2. Checks the spending budget
/// 3. Derives the wallet's signing key (Solana preferred, EVM fallback)
/// 4. Builds a partially-signed payment transaction
/// 5. Returns the encoded PAYMENT-SIGNATURE header value
///
/// The caller retries the original request with this header attached and
/// records the payment outcome in the ledger.
pub async fn handle_402_and_pay(
    response_headers: &HeaderMap,
    request_url: &str,
) -> Result<X402PaymentResult, X402Error> {
    let (challenge, idx, chain) = handle_402(response_headers)?;
    let requirement = &challenge.accepts[idx];

    let amount: u64 = requirement.amount.parse().map_err(|e| {
        X402Error::Protocol(format!("invalid amount '{}': {e}", requirement.amount))
    })?;

    let budget_check =
        super::store::with_ledger(|l| l.check_budget(amount)).map_err(|e| X402Error::Wallet(e))?;

    match budget_check {
        super::store::BudgetCheck::Allowed => {}
        super::store::BudgetCheck::ExceedsPerRequest { requested, cap } => {
            return Err(X402Error::AmountExceedsCap { requested, cap });
        }
        super::store::BudgetCheck::ExceedsDailyBudget { current, cap } => {
            return Err(X402Error::BudgetExceeded {
                period: "daily",
                current,
                cap,
            });
        }
        super::store::BudgetCheck::ExceedsMonthlyBudget { current, cap } => {
            return Err(X402Error::BudgetExceeded {
                period: "monthly",
                current,
                cap,
            });
        }
    }

    debug!(
        "{LOG_PREFIX} paying {} atomic {} to {} for {} chain={:?}",
        amount, requirement.asset, requirement.pay_to, request_url, chain
    );

    let payment = match chain {
        PaymentChain::Solana => {
            let signing_key = derive_wallet_signing_key().await?;
            build_solana_payment(&signing_key, &challenge, requirement).await?
        }
        PaymentChain::Evm => build_evm_payment(&challenge, requirement).await?,
    };

    let header_value = serde_json::to_string(&payment)
        .map(|json| B64.encode(json))
        .map_err(|e| X402Error::Protocol(format!("serialize payment: {e}")))?;

    Ok(X402PaymentResult {
        header_value,
        amount_atomic: amount,
        asset: requirement.asset.clone(),
        recipient: requirement.pay_to.clone(),
        network: requirement.network.clone(),
        url: request_url.to_string(),
    })
}

/// Derive the wallet's Solana ed25519 signing key from the encrypted mnemonic.
async fn derive_wallet_signing_key() -> Result<SigningKey, X402Error> {
    use crate::openhuman::wallet::WalletChain;

    let secret = crate::openhuman::wallet::secret_material(WalletChain::Solana)
        .await
        .map_err(|e| X402Error::Wallet(format!("wallet secret: {e}")))?;

    let config = crate::openhuman::config::rpc::load_config_with_timeout()
        .await
        .map_err(|e| X402Error::Wallet(format!("load config: {e}")))?;

    let mnemonic =
        crate::openhuman::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
            .await
            .map_err(|e| X402Error::Wallet(format!("decrypt mnemonic: {e}")))?
            .value;

    derive_solana_keypair_from_mnemonic(&mnemonic, &secret.derivation_path)
}

fn derive_solana_keypair_from_mnemonic(
    mnemonic: &str,
    derivation_path: &str,
) -> Result<SigningKey, X402Error> {
    use coins_bip39::{English, Mnemonic};
    use ed25519_dalek::SECRET_KEY_LENGTH;
    use hmac::{Hmac, Mac};
    use sha2::Sha512;

    let mnemonic_obj: Mnemonic<English> = mnemonic
        .trim()
        .parse()
        .map_err(|e| X402Error::Wallet(format!("invalid mnemonic: {e}")))?;
    let seed = mnemonic_obj
        .to_seed(None)
        .map_err(|e| X402Error::Wallet(format!("seed derivation: {e}")))?;

    // SLIP-0010 ed25519 derivation
    type HmacSha512 = Hmac<Sha512>;
    let mut mac = HmacSha512::new_from_slice(b"ed25519 seed")
        .map_err(|e| X402Error::Wallet(format!("HMAC init: {e}")))?;
    mac.update(&seed);
    let i = mac.finalize().into_bytes();
    let mut key = [0u8; 32];
    let mut chain_code = [0u8; 32];
    key.copy_from_slice(&i[..32]);
    chain_code.copy_from_slice(&i[32..]);

    let path = parse_derivation_path(derivation_path)?;
    for index in path {
        let hardened = index | 0x8000_0000;
        let mut mac = HmacSha512::new_from_slice(&chain_code)
            .map_err(|e| X402Error::Wallet(format!("HMAC init: {e}")))?;
        mac.update(&[0u8]);
        mac.update(&key);
        mac.update(&hardened.to_be_bytes());
        let i = mac.finalize().into_bytes();
        key.copy_from_slice(&i[..32]);
        chain_code.copy_from_slice(&i[32..]);
    }

    let bytes: [u8; SECRET_KEY_LENGTH] = key;
    Ok(SigningKey::from_bytes(&bytes))
}

fn parse_derivation_path(path: &str) -> Result<Vec<u32>, X402Error> {
    let trimmed = path.trim();
    let mut iter = trimmed.split('/');
    match iter.next() {
        Some("m") => {}
        _ => {
            return Err(X402Error::Wallet(format!(
                "path must start with 'm': {path}"
            )))
        }
    }
    let mut out = Vec::new();
    for seg in iter {
        let stripped = seg
            .strip_suffix('\'')
            .ok_or_else(|| X402Error::Wallet(format!("non-hardened segment in: {path}")))?;
        let v: u32 = stripped
            .parse()
            .map_err(|e| X402Error::Wallet(format!("invalid path segment '{seg}': {e}")))?;
        out.push(v);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum X402Error {
    Transport(reqwest::Error),
    NoPaymentHeader,
    NoPaymentOption,
    AmountExceedsCap {
        requested: u64,
        cap: u64,
    },
    BudgetExceeded {
        period: &'static str,
        current: u64,
        cap: u64,
    },
    Protocol(String),
    Wallet(String),
}

impl std::fmt::Display for X402Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "x402 transport: {e}"),
            Self::NoPaymentHeader => write!(f, "402 response missing PAYMENT-REQUIRED header"),
            Self::NoPaymentOption => {
                write!(
                    f,
                    "no supported payment option (Solana exact or EVM exact) in 402 challenge"
                )
            }
            Self::AmountExceedsCap { requested, cap } => {
                write!(f, "x402 amount {requested} exceeds per-request cap {cap}")
            }
            Self::BudgetExceeded {
                period,
                current,
                cap,
            } => {
                write!(
                    f,
                    "x402 {period} budget exceeded: {current}/{cap} atomic units"
                )
            }
            Self::Protocol(msg) => write!(f, "x402 protocol: {msg}"),
            Self::Wallet(msg) => write!(f, "x402 wallet: {msg}"),
        }
    }
}

impl std::error::Error for X402Error {}

// ---------------------------------------------------------------------------
// Header parsing
// ---------------------------------------------------------------------------

fn parse_402_headers(headers: &HeaderMap) -> Result<PaymentRequired, X402Error> {
    let raw = headers
        .get(HEADER_PAYMENT_REQUIRED)
        .or_else(|| headers.get(HEADER_PAYMENT_REQUIRED_V1))
        .ok_or(X402Error::NoPaymentHeader)?;
    let b64_str = raw.to_str().map_err(|e| {
        X402Error::Protocol(format!("PAYMENT-REQUIRED header not valid UTF-8: {e}"))
    })?;
    let json_bytes = B64
        .decode(b64_str.trim())
        .map_err(|e| X402Error::Protocol(format!("PAYMENT-REQUIRED base64 decode: {e}")))?;
    let challenge: PaymentRequired = serde_json::from_slice(&json_bytes)
        .map_err(|e| X402Error::Protocol(format!("PAYMENT-REQUIRED JSON parse: {e}")))?;
    if challenge.x402_version != X402_VERSION {
        warn!(
            "{LOG_PREFIX} unexpected x402 version {} (expected {X402_VERSION})",
            challenge.x402_version
        );
    }
    Ok(challenge)
}

fn parse_settlement_response(b64_str: &str) -> Result<SettlementResponse, String> {
    let json_bytes = B64
        .decode(b64_str.trim())
        .map_err(|e| format!("PAYMENT-RESPONSE base64 decode: {e}"))?;
    serde_json::from_slice(&json_bytes).map_err(|e| format!("PAYMENT-RESPONSE JSON parse: {e}"))
}

// ---------------------------------------------------------------------------
// Solana transaction construction
// ---------------------------------------------------------------------------

/// Build a partially-signed Solana transaction for the `exact` scheme.
///
/// Layout:
///   account_keys[0] = fee_payer (facilitator) — signer, writable
///   account_keys[1] = our_pubkey (transfer authority) — signer, writable
///   account_keys[2] = src_ata — writable
///   account_keys[3] = dst_ata — writable
///   account_keys[4] = mint — readonly
///   account_keys[5] = token_program — readonly
///   account_keys[6] = compute_budget_program — readonly
///   account_keys[7] = memo_program — readonly (if memo present)
///
/// Instructions:
///   0. SetComputeUnitLimit(DEFAULT_COMPUTE_UNITS)
///   1. SetComputeUnitPrice(DEFAULT_COMPUTE_UNIT_PRICE)
///   2. TransferChecked { amount, decimals=6 }
///   3. Memo (if extra.memo set, otherwise random 16-byte hex nonce)
async fn build_solana_payment(
    signing_key: &SigningKey,
    challenge: &PaymentRequired,
    req: &PaymentRequirements,
) -> Result<PaymentPayload, X402Error> {
    let our_pubkey = signing_key.verifying_key().to_bytes();
    let amount: u64 = req
        .amount
        .parse()
        .map_err(|e| X402Error::Protocol(format!("invalid amount '{}': {e}", req.amount)))?;

    let fee_payer = req
        .fee_payer_pubkey()
        .ok_or_else(|| X402Error::Protocol("no fee_payer in payment requirements".into()))?;
    let fee_payer_bytes = b58_to_32(fee_payer)?;
    let pay_to_bytes = b58_to_32(&req.pay_to)?;
    let mint_bytes = b58_to_32(&req.asset)?;

    let token_program = b58_to_32(SPL_TOKEN_PROGRAM)?;
    let compute_budget = b58_to_32(COMPUTE_BUDGET_PROGRAM)?;
    let memo_program = b58_to_32(SPL_MEMO_PROGRAM)?;

    let src_ata = derive_ata(&our_pubkey, &mint_bytes, &token_program)?;
    let dst_ata = derive_ata(&pay_to_bytes, &mint_bytes, &token_program)?;

    let memo_data = req
        .memo_value()
        .map(|m| m.as_bytes().to_vec())
        .unwrap_or_else(|| random_memo_nonce());

    // -- account keys (order matters) --
    let account_keys: Vec<[u8; 32]> = vec![
        fee_payer_bytes, // 0: fee payer (signer, writable)
        our_pubkey,      // 1: transfer authority (signer, writable)
        src_ata,         // 2: source ATA (writable)
        dst_ata,         // 3: destination ATA (writable)
        mint_bytes,      // 4: mint (readonly)
        token_program,   // 5: SPL Token program (readonly)
        compute_budget,  // 6: Compute Budget program (readonly)
        memo_program,    // 7: SPL Memo program (readonly)
    ];

    // header: [num_required_sigs, num_readonly_signed, num_readonly_unsigned]
    // 2 signers (fee_payer + us), 0 readonly signed, 4 readonly unsigned
    // (mint, token_program, compute_budget, memo_program)
    let header = [2u8, 0u8, 4u8];

    // -- instructions --
    let set_cu_limit = build_set_compute_unit_limit(6, DEFAULT_COMPUTE_UNITS);
    let set_cu_price = build_set_compute_unit_price(6, DEFAULT_COMPUTE_UNIT_PRICE);
    let transfer_checked = build_transfer_checked(
        5, // token_program index
        2, // src_ata index
        4, // mint index
        3, // dst_ata index
        1, // authority (our_pubkey) index
        amount, 6, // USDC decimals
    );
    let memo = build_memo(7, &memo_data);

    let instructions = vec![set_cu_limit, set_cu_price, transfer_checked, memo];

    // -- fetch recent blockhash --
    let blockhash = fetch_recent_blockhash_for_x402().await?;

    // -- encode message --
    let message = encode_legacy_message(&header, &account_keys, &blockhash, &instructions);

    // -- build wire: 2 signature slots, sign only ours (index 1) --
    let mut wire = Vec::with_capacity(1 + 128 + message.len());
    wire.extend(encode_shortvec(2)); // 2 required signatures
    wire.extend([0u8; 64]); // slot 0: fee_payer (left zeroed for facilitator)

    let sig = signing_key.sign(&message);
    wire.extend(sig.to_bytes()); // slot 1: our signature
    wire.extend(&message);

    let tx_b64 = B64.encode(&wire);
    debug!(
        "{LOG_PREFIX} built payment tx {} bytes, amount={amount} asset={}",
        wire.len(),
        req.asset
    );

    Ok(PaymentPayload {
        x402_version: X402_VERSION,
        resource: Some(challenge.resource.clone()),
        accepted: req.clone(),
        payload: PaymentProof::Solana(SolanaPaymentProof {
            transaction: tx_b64,
        }),
        extensions: serde_json::Map::new(),
    })
}

// ---------------------------------------------------------------------------
// EVM payment construction (EIP-3009 transferWithAuthorization)
// ---------------------------------------------------------------------------

/// Build an EVM payment using EIP-3009 `transferWithAuthorization`.
/// Signs the typed data with the wallet's EVM key and returns the proof
/// for the facilitator to submit on-chain.
async fn build_evm_payment(
    challenge: &PaymentRequired,
    req: &PaymentRequirements,
) -> Result<PaymentPayload, X402Error> {
    let (signer, from_address) = derive_evm_signer().await?;
    build_evm_payment_with_signer(&signer, from_address, challenge, req)
}

/// Core EVM payment construction — separated from wallet derivation for testability.
pub(crate) fn build_evm_payment_with_signer(
    signer: &ethers_signers::LocalWallet,
    from_address: ethers_core::types::Address,
    challenge: &PaymentRequired,
    req: &PaymentRequirements,
) -> Result<PaymentPayload, X402Error> {
    use ethers_core::types::{Address, U256};
    use ethers_signers::Signer;
    use std::str::FromStr;

    let chain_id = req
        .evm_chain_id()
        .ok_or_else(|| X402Error::Protocol(format!("not an EVM network: {}", req.network)))?;

    let amount = U256::from_dec_str(&req.amount)
        .map_err(|e| X402Error::Protocol(format!("invalid amount '{}': {e}", req.amount)))?;

    let pay_to = Address::from_str(&req.pay_to).map_err(|e| {
        X402Error::Protocol(format!("invalid EVM payTo address '{}': {e}", req.pay_to))
    })?;

    let token_address = Address::from_str(&req.asset).map_err(|e| {
        X402Error::Protocol(format!("invalid EVM token address '{}': {e}", req.asset))
    })?;

    // EIP-3009 parameters
    let valid_after = U256::zero();
    let valid_before = U256::from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + req.max_timeout_seconds,
    );

    // Random nonce for EIP-3009
    let nonce = {
        let mut hasher = Sha256::new();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        hasher.update(ts.to_le_bytes());
        hasher.update(std::process::id().to_le_bytes());
        let hash: [u8; 32] = hasher.finalize().into();
        hash
    };

    // EIP-712 typed data for `transferWithAuthorization`
    let domain_name = req
        .extra
        .as_ref()
        .and_then(|e| e.name.as_deref())
        .unwrap_or("USD Coin");
    let domain_version = req
        .extra
        .as_ref()
        .and_then(|e| e.version.as_deref())
        .unwrap_or("2");
    let domain_separator =
        eip712_domain_separator_named(token_address, chain_id, domain_name, domain_version);
    let struct_hash = eip3009_struct_hash(
        from_address,
        pay_to,
        amount,
        valid_after,
        valid_before,
        nonce,
    );

    let mut digest_input = Vec::with_capacity(66);
    digest_input.extend(b"\x19\x01");
    digest_input.extend(domain_separator);
    digest_input.extend(struct_hash);
    let digest: [u8; 32] = {
        use ethers_core::types::H256;
        let h = H256::from(ethers_core::utils::keccak256(&digest_input));
        h.into()
    };

    let signature = signer
        .sign_hash(ethers_core::types::H256::from(digest))
        .map_err(|e| X402Error::Wallet(format!("EVM sign EIP-3009: {e}")))?;

    let sig_hex = format!("0x{}", hex::encode(signature.to_vec()));
    let nonce_hex = format!("0x{}", hex::encode(nonce));

    debug!(
        "{LOG_PREFIX} built EVM payment chain_id={chain_id} amount={} asset={} from={:#x} to={:#x}",
        req.amount, req.asset, from_address, pay_to
    );

    Ok(PaymentPayload {
        x402_version: X402_VERSION,
        resource: Some(challenge.resource.clone()),
        accepted: req.clone(),
        payload: PaymentProof::Evm(EvmPaymentProof {
            signature: sig_hex,
            authorization: EvmAuthorization {
                from: format!("{from_address:#x}"),
                to: format!("{pay_to:#x}"),
                value: req.amount.clone(),
                valid_after: "0".to_string(),
                valid_before: valid_before.to_string(),
                nonce: nonce_hex,
            },
        }),
        extensions: serde_json::Map::new(),
    })
}

/// Derive the wallet's EVM signer from the encrypted mnemonic.
async fn derive_evm_signer(
) -> Result<(ethers_signers::LocalWallet, ethers_core::types::Address), X402Error> {
    use crate::openhuman::wallet::WalletChain;
    use ethers_signers::{coins_bip39::English, MnemonicBuilder, Signer};

    let secret = crate::openhuman::wallet::secret_material(WalletChain::Evm)
        .await
        .map_err(|e| X402Error::Wallet(format!("wallet secret: {e}")))?;

    let config = crate::openhuman::config::rpc::load_config_with_timeout()
        .await
        .map_err(|e| X402Error::Wallet(format!("load config: {e}")))?;

    let mnemonic =
        crate::openhuman::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
            .await
            .map_err(|e| X402Error::Wallet(format!("decrypt mnemonic: {e}")))?
            .value;

    let wallet = MnemonicBuilder::<English>::default()
        .phrase(mnemonic.as_str())
        .derivation_path(&secret.derivation_path)
        .map_err(|e| {
            X402Error::Wallet(format!(
                "invalid EVM derivation path '{}': {e}",
                secret.derivation_path
            ))
        })?
        .build()
        .map_err(|e| X402Error::Wallet(format!("derive EVM signer: {e}")))?;

    let address = wallet.address();
    Ok((wallet, address))
}

/// EIP-712 domain separator with default USDC params ("USD Coin", "2").
pub(crate) fn eip712_domain_separator(
    verifying_contract: ethers_core::types::Address,
    chain_id: u64,
) -> [u8; 32] {
    eip712_domain_separator_named(verifying_contract, chain_id, "USD Coin", "2")
}

/// EIP-712 domain separator with explicit name and version from the 402 extra.
pub(crate) fn eip712_domain_separator_named(
    verifying_contract: ethers_core::types::Address,
    chain_id: u64,
    name: &str,
    version: &str,
) -> [u8; 32] {
    use ethers_core::utils::keccak256;

    let type_hash = keccak256(
        b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let name_hash = keccak256(name.as_bytes());
    let version_hash = keccak256(version.as_bytes());

    let mut encoded = Vec::with_capacity(5 * 32);
    encoded.extend(type_hash);
    encoded.extend(name_hash);
    encoded.extend(version_hash);
    let mut chain_id_bytes = [0u8; 32];
    chain_id_bytes[24..].copy_from_slice(&chain_id.to_be_bytes());
    encoded.extend(chain_id_bytes);
    let mut addr_bytes = [0u8; 32];
    addr_bytes[12..].copy_from_slice(verifying_contract.as_bytes());
    encoded.extend(addr_bytes);

    keccak256(&encoded)
}

/// EIP-3009 `TransferWithAuthorization` struct hash.
pub(crate) fn eip3009_struct_hash(
    from: ethers_core::types::Address,
    to: ethers_core::types::Address,
    value: ethers_core::types::U256,
    valid_after: ethers_core::types::U256,
    valid_before: ethers_core::types::U256,
    nonce: [u8; 32],
) -> [u8; 32] {
    use ethers_core::utils::keccak256;

    let type_hash = keccak256(
        b"TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)",
    );

    let mut encoded = Vec::with_capacity(7 * 32);
    encoded.extend(type_hash);

    let mut from_bytes = [0u8; 32];
    from_bytes[12..].copy_from_slice(from.as_bytes());
    encoded.extend(from_bytes);

    let mut to_bytes = [0u8; 32];
    to_bytes[12..].copy_from_slice(to.as_bytes());
    encoded.extend(to_bytes);

    let mut value_bytes = [0u8; 32];
    value.to_big_endian(&mut value_bytes);
    encoded.extend(value_bytes);

    let mut va_bytes = [0u8; 32];
    valid_after.to_big_endian(&mut va_bytes);
    encoded.extend(va_bytes);

    let mut vb_bytes = [0u8; 32];
    valid_before.to_big_endian(&mut vb_bytes);
    encoded.extend(vb_bytes);

    encoded.extend(nonce);

    keccak256(&encoded)
}

// ---------------------------------------------------------------------------
// Solana wire-format helpers (mirrors wallet/chains/solana.rs primitives)
// ---------------------------------------------------------------------------

fn b58_to_32(addr: &str) -> Result<[u8; 32], X402Error> {
    let v = bs58::decode(addr.trim())
        .into_vec()
        .map_err(|e| X402Error::Protocol(format!("invalid base58 '{addr}': {e}")))?;
    if v.len() != 32 {
        return Err(X402Error::Protocol(format!(
            "expected 32-byte key, got {} for '{addr}'",
            v.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    Ok(out)
}

fn derive_ata(
    owner: &[u8; 32],
    mint: &[u8; 32],
    token_program: &[u8; 32],
) -> Result<[u8; 32], X402Error> {
    let ata_program = b58_to_32("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL")?;
    for bump in (0u8..=255).rev() {
        let mut hasher = Sha256::new();
        hasher.update(owner);
        hasher.update(token_program);
        hasher.update(mint);
        hasher.update([bump]);
        hasher.update(&ata_program);
        hasher.update(b"ProgramDerivedAddress");
        let candidate: [u8; 32] = hasher.finalize().into();
        if curve25519_dalek::edwards::CompressedEdwardsY(candidate)
            .decompress()
            .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(X402Error::Protocol("ATA PDA derivation failed".into()))
}

fn encode_shortvec(value: u16) -> Vec<u8> {
    let mut out = Vec::new();
    let mut v = value as u32;
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

struct Instruction {
    program_id_index: u8,
    accounts: Vec<u8>,
    data: Vec<u8>,
}

fn build_set_compute_unit_limit(program_idx: u8, units: u32) -> Instruction {
    let mut data = vec![2u8]; // discriminator
    data.extend(units.to_le_bytes());
    Instruction {
        program_id_index: program_idx,
        accounts: vec![],
        data,
    }
}

fn build_set_compute_unit_price(program_idx: u8, micro_lamports: u64) -> Instruction {
    let mut data = vec![3u8]; // discriminator
    data.extend(micro_lamports.to_le_bytes());
    Instruction {
        program_id_index: program_idx,
        accounts: vec![],
        data,
    }
}

fn build_transfer_checked(
    token_program_idx: u8,
    src_idx: u8,
    mint_idx: u8,
    dst_idx: u8,
    authority_idx: u8,
    amount: u64,
    decimals: u8,
) -> Instruction {
    let mut data = vec![12u8]; // SPL Token: TransferChecked = 12
    data.extend(amount.to_le_bytes());
    data.push(decimals);
    Instruction {
        program_id_index: token_program_idx,
        accounts: vec![src_idx, mint_idx, dst_idx, authority_idx],
        data,
    }
}

fn build_memo(program_idx: u8, memo_data: &[u8]) -> Instruction {
    Instruction {
        program_id_index: program_idx,
        accounts: vec![],
        data: memo_data.to_vec(),
    }
}

fn encode_instruction(ins: &Instruction) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(ins.program_id_index);
    out.extend(encode_shortvec(ins.accounts.len() as u16));
    out.extend(&ins.accounts);
    out.extend(encode_shortvec(ins.data.len() as u16));
    out.extend(&ins.data);
    out
}

fn encode_legacy_message(
    header: &[u8; 3],
    account_keys: &[[u8; 32]],
    recent_blockhash: &[u8; 32],
    instructions: &[Instruction],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(header);
    out.extend(encode_shortvec(account_keys.len() as u16));
    for key in account_keys {
        out.extend(key);
    }
    out.extend(recent_blockhash);
    out.extend(encode_shortvec(instructions.len() as u16));
    for ins in instructions {
        out.extend(encode_instruction(ins));
    }
    out
}

fn random_memo_nonce() -> Vec<u8> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut hasher = Sha256::new();
    hasher.update(ts.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    hex::encode(&hash[..16]).into_bytes()
}

async fn fetch_recent_blockhash_for_x402() -> Result<[u8; 32], X402Error> {
    use crate::openhuman::wallet::WalletChain;

    #[derive(serde::Deserialize)]
    struct BlockhashResponse {
        value: BlockhashValue,
    }
    #[derive(serde::Deserialize)]
    struct BlockhashValue {
        blockhash: String,
    }

    let result: BlockhashResponse = crate::openhuman::wallet::rpc::rpc_call(
        WalletChain::Solana,
        "getLatestBlockhash",
        serde_json::json!([{"commitment": "finalized"}]),
    )
    .await
    .map_err(|e| X402Error::Wallet(format!("fetch blockhash: {e}")))?;

    b58_to_32(&result.value.blockhash)
}
