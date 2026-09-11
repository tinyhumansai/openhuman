use base64::engine::{general_purpose::STANDARD as B64, Engine as _};

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
    /// The signing key is no longer a parameter: derivation and signing both
    /// happen inside the loaded wallet module, so there is no key for a caller
    /// to hold or pass. The wallet's phrase is resolved here and handed over on
    /// a confidential call.
    ///
    /// `max_amount` — optional ceiling in atomic units; rejects challenges above
    ///                this to prevent runaway spending.
    pub async fn try_paid_request(
        &self,
        request: reqwest::Request,
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
                let (config, signing_secret, our_pubkey) = wallet_signer().await?;
                build_solana_payment(
                    &config,
                    &signing_secret,
                    our_pubkey,
                    &challenge,
                    requirement,
                )
                .await?
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
    challenge: &PaymentRequired,
    requirement: &PaymentRequirements,
) -> Result<String, X402Error> {
    let chain = if requirement.network.starts_with("eip155:") {
        PaymentChain::Evm
    } else {
        PaymentChain::Solana
    };
    let payment = match chain {
        PaymentChain::Solana => {
            let (config, signing_secret, our_pubkey) = wallet_signer().await?;
            build_solana_payment(&config, &signing_secret, our_pubkey, challenge, requirement)
                .await?
        }
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
        super::store::with_ledger(|l| l.check_budget(amount)).map_err(X402Error::Wallet)?;

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
            let (config, signing_secret, our_pubkey) = wallet_signer().await?;
            build_solana_payment(
                &config,
                &signing_secret,
                our_pubkey,
                &challenge,
                requirement,
            )
            .await?
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
/// The phrase to sign a payment with, its config, and the wallet's public key.
///
/// Derivation happens in the loaded wallet module; this process never holds the
/// private key. The phrase is handed over on a confidential call, and only to a
/// module that has proved it is an artifact this build pinned.
async fn wallet_signer() -> Result<
    (
        crate::openhuman::config::Config,
        tinywallet_bus::wire::SecretMaterial,
        [u8; 32],
    ),
    X402Error,
> {
    use crate::openhuman::web3::wallet::WalletChain;

    let secret = crate::openhuman::web3::wallet::secret_material(WalletChain::Solana)
        .await
        .map_err(|e| X402Error::Wallet(format!("wallet secret: {e}")))?;

    let config = crate::openhuman::config::rpc::load_config_with_timeout()
        .await
        .map_err(|e| X402Error::Wallet(format!("load config: {e}")))?;

    let mnemonic = crate::openhuman::security::encryption::rpc::decrypt_secret(
        &config,
        &secret.encrypted_mnemonic,
    )
    .await
    .map_err(|e| X402Error::Wallet(format!("decrypt mnemonic: {e}")))?
    .value;

    let signing_secret = tinywallet_bus::wire::SecretMaterial {
        mnemonic,
        derivation_path: secret.derivation_path.clone(),
        chain: tinywallet_bus::Chain::Solana,
    };
    let account = crate::openhuman::modules::wallet::derive_account(&config, &signing_secret)
        .await
        .map_err(|e| X402Error::Wallet(format!("derive account: {e}")))?;
    let pubkey = b58_to_32(&account.address)?;
    Ok((config, signing_secret, pubkey))
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
    config: &crate::openhuman::config::Config,
    signing_secret: &tinywallet_bus::wire::SecretMaterial,
    our_pubkey: [u8; 32],
    challenge: &PaymentRequired,
    req: &PaymentRequirements,
) -> Result<PaymentPayload, X402Error> {
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
        .unwrap_or_else(random_memo_nonce);

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

    // Signed in the module: the phrase goes over a confidential call and the
    // private key is never assembled in this process.
    let signature = crate::openhuman::modules::wallet::sign_message(
        config,
        signing_secret,
        &message,
        tinywallet_bus::wire::Scheme::Ed25519,
    )
    .await
    .map_err(|e| X402Error::Wallet(format!("sign payment: {e}")))?;
    let tinywallet_bus::wire::Signature::Ed25519 { signature_hex } = signature else {
        return Err(X402Error::Wallet(
            "the wallet module returned a non-ed25519 signature".to_string(),
        ));
    };
    let sig_bytes = hex_to_32_bytes_64(&signature_hex)?;
    wire.extend(sig_bytes); // slot 1: our signature
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
    let (config, signing_secret, from_address) = evm_signer().await?;
    let authorization = evm_payment_authorization(&from_address, req)?;

    // Signed in the wallet module over the prehashed EIP-712 digest. This
    // process never holds the EVM key.
    let signature = crate::openhuman::modules::wallet::sign_message(
        &config,
        &signing_secret,
        &authorization.digest,
        tinywallet_bus::wire::Scheme::Secp256k1Prehash,
    )
    .await
    .map_err(|e| X402Error::Wallet(format!("sign EIP-3009: {e}")))?;
    let tinywallet_bus::wire::Signature::Secp256k1 {
        rs_hex,
        recovery_id,
    } = signature
    else {
        return Err(X402Error::Wallet(
            "the wallet module returned a non-secp256k1 signature".to_string(),
        ));
    };
    let rs = hex::decode(&rs_hex)
        .map_err(|e| X402Error::Wallet(format!("invalid signature hex: {e}")))?;
    if rs.len() != 64 {
        return Err(X402Error::Wallet(
            "the wallet module returned a malformed signature".to_string(),
        ));
    }
    let mut sig_bytes = [0u8; 65];
    sig_bytes[..64].copy_from_slice(&rs);
    sig_bytes[64] = recovery_id
        .checked_add(27)
        .ok_or_else(|| X402Error::Wallet("recovery id out of range".to_string()))?;

    evm_payment_payload(&authorization, sig_bytes, &from_address, challenge, req)
}

/// The EIP-712 digest to sign, and the fields the payload needs alongside it.
///
/// Split out from signing so that production (which signs in the wallet module)
/// and the tests (which sign locally, to check the construction against a fixed
/// vector) share one implementation of the part that can be wrong. Only *who
/// holds the key* differs between them.
pub(crate) struct EvmPaymentAuthorization {
    /// The 32-byte EIP-712 digest.
    pub digest: [u8; 32],
    /// The EIP-3009 nonce, echoed into the payload.
    pub nonce: [u8; 32],
    valid_after_secs: u64,
    valid_before_secs: u64,
}
