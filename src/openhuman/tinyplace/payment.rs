//! x402 payment fulfillment bridge — **section-agnostic**.
//!
//! Turns a `402 Payment Required` [`PaymentChallenge`] into a signed
//! [`X402PaymentMap`] by paying on-chain through the OpenHuman wallet and then
//! signing the x402 authorization with the tiny.place identity key.
//!
//! The flow is shared verbatim across register / buy / bid / offer; callers
//! parameterise the purpose and any extra signed metadata via [`PaymentContext`].
//!
//! ## What this module does NOT do
//!
//! - It does **not** build the canonical message, sign it, or flatten the map —
//!   that lives in the SDK ([`tinyplace::x402::build_x402_payment_map`]).
//! - It does **not** *pick* a Solana cluster — `challenge.network` is passed
//!   through verbatim and the asset is routed by symbol. It exposes an
//!   *advisory* [`ensure_cluster_matches`] check (Risk R6); it does NOT block,
//!   because tiny.place labels every cluster with the mainnet genesis, so the
//!   challenge network is not an authoritative cluster signal. Cluster alignment
//!   is governed by `OPENHUMAN_SOLANA_CLUSTER` + operator config.
//! - It does **not** expose an RPC controller. The section write-handlers
//!   (register / marketplace, in later PRs) call [`fulfill_payment`] and attach
//!   the returned payment map to their domain request.
//!
//! Marketplace write handlers (buy / bid / offer) are still pending, so a few
//! helpers here are not yet referenced; keep the targeted allows until those
//! PRs land rather than deleting otherwise-correct code.

use std::collections::HashMap;

use tinyplace::signer::Signer;
use tinyplace::x402::{
    build_x402_payment_map, generate_nonce, X402PaymentAuthorizationOptions, X402PaymentMap,
    X402PaymentReferenceOptions,
};
use tinyplace::PaymentChallenge;

use crate::openhuman::wallet::rpc::with_tinyplace_solana_endpoints;
use crate::openhuman::wallet::{
    execute_prepared, prepare_transfer, solana_cluster, tinyplace_solana_rpc_endpoints,
    ExecutePreparedParams, PrepareTransferParams, SolanaCluster, WalletChain,
};

const LOG_PREFIX: &str = "[tinyplace-pay]";

/// Reject a challenge whose expiry is within this many seconds of now — leaves
/// headroom for the on-chain broadcast + verification round-trip.
const EXPIRY_SKEW_SECS: i64 = 30;

// ── Public types ──────────────────────────────────────────────────────────────

/// Caller-supplied purpose + extra metadata. Parameterises the otherwise
/// identical register / buy / bid / offer flows.
#[derive(Debug, Clone)]
pub(crate) struct PaymentContext {
    /// Folded into the signed metadata as `purpose` (e.g. `"identity.register"`,
    /// `"marketplace.buy"`).
    pub(crate) purpose: String,
    /// Nonce prefix used when the challenge omits a nonce (e.g. `"register"` →
    /// `register_<hex>`).
    pub(crate) nonce_prefix: String,
    /// Extra signed metadata (e.g. `{ "identity": "@handle" }`,
    /// `{ "listingId": "…" }`).
    pub(crate) extra_metadata: HashMap<String, String>,
}

/// Result of a completed on-chain payment plus its signed x402 authorization.
#[derive(Debug, Clone)]
pub(crate) struct FulfilledPayment {
    /// The flat x402 payment map to attach to the domain request.
    pub(crate) payment_map: X402PaymentMap,
    /// The on-chain Solana transaction signature that moved the funds.
    pub(crate) on_chain_tx: String,
    /// The wallet quote id the transfer was executed under. Surfaced for
    /// diagnostics/audit; not all callers read it (the register handler only
    /// needs `on_chain_tx`).
    #[allow(dead_code)]
    pub(crate) quote_id: String,
}

// ── Internal types ────────────────────────────────────────────────────────────

/// A challenge whose required fields have been validated and asset-routed.
#[derive(Debug, Clone)]
struct ValidatedChallenge {
    network: String,
    asset: String,
    amount: String,
    to: String,
    nonce: Option<String>,
    expires_at: Option<String>,
    /// `None` for native SOL, `Some("USDC")` for the SPL token.
    asset_symbol: Option<String>,
}

// ── Pure helpers (unit-tested; no network, no funds) ──────────────────────────

/// Validate required challenge fields, check expiry, and route the asset.
fn validate_challenge(challenge: &PaymentChallenge) -> Result<ValidatedChallenge, String> {
    let asset = non_empty(&challenge.asset).ok_or("x402 challenge missing 'asset'")?;
    let amount = non_empty(&challenge.amount).ok_or("x402 challenge missing 'amount'")?;
    let to = non_empty(&challenge.to).ok_or("x402 challenge missing 'to'")?;
    let network = non_empty(&challenge.network).ok_or("x402 challenge missing 'network'")?;

    let asset_symbol = match asset.as_str() {
        "SOL" => None,
        "USDC" => Some("USDC".to_string()),
        other => return Err(format!("unsupported x402 asset: {other}")),
    };

    let expires_at = non_empty(&challenge.expires_at);
    if let Some(expiry) = &expires_at {
        if is_expired(expiry) {
            return Err("payment challenge expired".to_string());
        }
    }

    Ok(ValidatedChallenge {
        network,
        asset,
        amount,
        to,
        nonce: non_empty(&challenge.nonce),
        expires_at,
        asset_symbol,
    })
}

/// Map a validated challenge to wallet transfer params (asset routing lives here).
fn to_transfer_params(v: &ValidatedChallenge) -> PrepareTransferParams {
    PrepareTransferParams {
        chain: WalletChain::Solana,
        to_address: v.to.clone(),
        amount_raw: v.amount.clone(),
        asset_symbol: v.asset_symbol.clone(),
        evm_network: None,
    }
}

/// Build and sign the x402 payment map via the SDK. Offline — needs only a
/// signer. The `on_chain_tx` is attached to the payment **references**
/// (`onChainTx`/`tx`/`transaction`), never to the `signature` field (which is
/// the off-chain Ed25519 authorization signature).
async fn build_payment_map(
    signer: &dyn Signer,
    v: &ValidatedChallenge,
    on_chain_tx: &str,
    ctx: &PaymentContext,
) -> Result<X402PaymentMap, String> {
    let mut metadata = ctx.extra_metadata.clone();
    metadata.insert("purpose".to_string(), ctx.purpose.clone());

    // Prefer the challenge nonce; otherwise mint one with the caller's prefix so
    // the SDK default ("pay") does not leak in.
    let nonce = v
        .nonce
        .clone()
        .unwrap_or_else(|| generate_nonce(Some(&ctx.nonce_prefix)));

    let options = X402PaymentAuthorizationOptions {
        network: v.network.clone(),
        asset: v.asset.clone(),
        amount: v.amount.clone(),
        from: Some(signer.agent_id()),
        to: v.to.clone(),
        nonce: Some(nonce),
        expires_at: v.expires_at.clone(),
        metadata: Some(metadata),
        references: X402PaymentReferenceOptions {
            on_chain_tx: Some(on_chain_tx.to_string()),
            tx: Some(on_chain_tx.to_string()),
            transaction: Some(on_chain_tx.to_string()),
            ..Default::default()
        },
        // scheme (→ "exact"), expires_in_ms, domain (→ "tiny.place") and
        // public_key_base64 (→ from signer) all take SDK defaults.
        ..Default::default()
    };

    build_x402_payment_map(signer, options)
        .await
        .map_err(|e| format!("x402 authorization signing failed: {e}"))
}

// ── Devnet guard (Risk R6) ────────────────────────────────────────────────────

/// CAIP-2 genesis-hash references for the public Solana clusters.
const MAINNET_GENESIS_PREFIX: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp";
const DEVNET_GENESIS_PREFIX: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1";

/// Loosely classify a challenge's `network` string into a Solana cluster.
///
/// Recognises both the human form (`"solana-devnet"`, `"…mainnet…"`) and the
/// CAIP-2 genesis-hash form (`"solana:5eykt4…"`). Returns `None` when the
/// format is unrecognised — the backend remains the source of truth, so an
/// unknown network is allowed through rather than blocked.
fn classify_network(network: &str) -> Option<SolanaCluster> {
    let lower = network.to_lowercase();
    if lower.contains("devnet") {
        return Some(SolanaCluster::Devnet);
    }
    if lower.contains("mainnet") {
        return Some(SolanaCluster::Mainnet);
    }
    // Testnet is unsupported; treat as unknown (None) so we never silently map
    // it onto mainnet/devnet.
    if network.contains(DEVNET_GENESIS_PREFIX) {
        return Some(SolanaCluster::Devnet);
    }
    if network.contains(MAINNET_GENESIS_PREFIX) {
        return Some(SolanaCluster::Mainnet);
    }
    None
}

/// Advisory cluster check (does **not** block).
///
/// The original design hard-blocked when the challenge's `network` genesis hash
/// implied a different cluster than [`solana_cluster`]. That premise is wrong for
/// tiny.place: the backend hard-codes the CAIP-2 Solana network to the **mainnet
/// genesis** (`solana:5eykt4…`) for *every* deployment — including devnet — and
/// only switches the real settlement chain via its `SOLANA_RPC_URL` /
/// `SOLANA_USDC_MINT` env (confirmed against the backend's `/payments/supported`,
/// which returns the **devnet** USDC mint on staging). So the challenge label
/// carries no reliable cluster signal, and a genesis-based hard block wrongly
/// rejects valid devnet payments.
///
/// Cluster alignment is therefore governed by explicit config
/// (`OPENHUMAN_SOLANA_CLUSTER`) + the operator pointing the wallet at the same
/// chain the backend settles on. The wallet only ever transfers the configured
/// cluster's USDC mint, so a true mismatch fails *safely* at verification (the
/// backend never credits a tx it can't see) rather than mis-spending. We log a
/// warning when the (advisory) label looks inconsistent.
///
/// The robust cross-check is now implemented as [`ensure_backend_mint_matches`],
/// which compares `solana_cluster().usdc_mint()` against the backend's
/// `solana.info()` USDC asset address. This advisory network-label check is
/// retained for observability (it logs when the label looks inconsistent).
pub(crate) fn ensure_cluster_matches(network: &str) -> Result<(), String> {
    cluster_guard(solana_cluster(), network)
}

/// Pure advisory check (no env access — testable in isolation). Always returns
/// `Ok`; logs a warning when the challenge label implies a different cluster than
/// configured (tiny.place's label is not authoritative — see
/// [`ensure_cluster_matches`]).
fn cluster_guard(configured: SolanaCluster, network: &str) -> Result<(), String> {
    match classify_network(network) {
        Some(challenge_cluster) if challenge_cluster != configured => {
            log::warn!(
                "{LOG_PREFIX} advisory: challenge network label='{network}' implies \
                 {challenge_cluster:?} but wallet configured for {configured:?}; tiny.place labels \
                 every cluster with the mainnet genesis, so proceeding — ensure the backend \
                 settles on {configured:?} (its /payments/supported USDC mint should match)"
            );
        }
        other => {
            log::debug!(
                "{LOG_PREFIX} cluster guard ok: network='{network}' classified={other:?} \
                 configured={configured:?}"
            );
        }
    }
    Ok(())
}

/// Pure mint cross-check: compares the configured wallet's USDC mint against
/// the backend's reported USDC mint from `solana.info()`.
///
/// - **Both match** → Ok (the common case).
/// - **Mismatch** → Err with a clear diagnostic (e.g. configured=mainnet
///   EPjFWdd… but backend reports devnet 4zMMC9…).
/// - **Backend USDC not found** (`backend_usdc_mint` is None) → Ok with a
///   log (the backend might not list USDC; fail-open).
///
/// This is the "does the wallet agree with the backend" check the TODO at
/// line 233-236 asked for. It replaces the genesis-based label check with
/// an authoritative mint comparison.
fn mint_cross_check(configured_mint: &str, backend_usdc_mint: Option<&str>) -> Result<(), String> {
    match backend_usdc_mint {
        Some(backend_mint) if backend_mint == configured_mint => {
            log::debug!(
                "{LOG_PREFIX} mint cross-check ok: configured={configured_mint} \
                 backend={backend_mint}"
            );
            Ok(())
        }
        Some(backend_mint) => {
            // TRUE MISMATCH — this is the real-funds protection case.
            Err(format!(
                "cluster mismatch: wallet configured USDC mint={configured_mint} but \
                 the backend reports USDC mint={backend_mint}; refusing to spend — \
                 check OPENHUMAN_SOLANA_CLUSTER and the backend's Solana config"
            ))
        }
        None => {
            // Backend did not report a USDC asset in solana.info().assets.
            // Fail-open: the backend might be running an older version or USDC is
            // not configured. The payment will still fail safely at verification
            // if there is a real mismatch.
            log::warn!(
                "{LOG_PREFIX} mint cross-check: backend solana.info() did not include a \
                 USDC asset; cannot verify against configured mint={configured_mint}; \
                 proceeding (fail-open)"
            );
            Ok(())
        }
    }
}

/// Async mint cross-check: fetches `client.solana.info()`, finds the USDC
/// asset, and compares its mint address against the configured wallet's
/// `solana_cluster().usdc_mint()`.
///
/// **Fail-open on transient errors**: if the fetch fails (backend
/// unreachable, endpoint not deployed), logs a warning and returns Ok.
/// Only blocks on a CONFIRMED mint mismatch where both sides returned.
pub(crate) async fn ensure_backend_mint_matches(
    client: &tinyplace::TinyPlaceClient,
) -> Result<(), String> {
    let configured_mint = solana_cluster().usdc_mint();

    let chain_info = match client.solana.info().await {
        Ok(info) => info,
        Err(e) => {
            log::warn!(
                "{LOG_PREFIX} mint cross-check: solana.info() failed ({e}); \
                 proceeding without mint verification (fail-open)"
            );
            return Ok(());
        }
    };

    log::debug!(
        "{LOG_PREFIX} mint cross-check: backend network='{}' assets_count={}",
        chain_info.network,
        chain_info.assets.len(),
    );

    let backend_usdc_mint = chain_info
        .assets
        .iter()
        .find(|a| a.symbol == "USDC")
        .and_then(|a| a.address.as_deref());

    mint_cross_check(configured_mint, backend_usdc_mint)
}

// ── High-level orchestrator (thin; logic delegated to the tested helpers) ─────

/// Validate the challenge, pay on-chain (`prepare_transfer` + `execute_prepared`
/// with `confirmed: true`), sign the x402 authorization, and return the payment
/// map plus on-chain tx and quote id.
///
/// Spends real funds when it reaches the wallet calls — callers MUST gate this
/// behind an explicit, user-confirmed action.
pub(crate) async fn fulfill_payment(
    challenge: &PaymentChallenge,
    signer: &dyn Signer,
    ctx: PaymentContext,
) -> Result<FulfilledPayment, String> {
    let v = validate_challenge(challenge)?;
    log::debug!(
        "{LOG_PREFIX} fulfill purpose={} asset={} amount={} to={}",
        ctx.purpose,
        v.asset,
        v.amount,
        truncate(&v.to),
    );

    // Run the prepare + broadcast inside the tiny.place Solana endpoint scope so
    // both hit the tiny.place settlement RPC (where agent accounts are funded),
    // falling back to the public cluster only if that endpoint is unreachable.
    // Scoped to this task — general wallet ops are unaffected.
    let endpoints = tinyplace_solana_rpc_endpoints();
    log::debug!(
        "{LOG_PREFIX} settlement solana endpoints={} (primary + fallbacks)",
        endpoints.len()
    );
    let (quote_id, on_chain_tx) = with_tinyplace_solana_endpoints(endpoints, async {
        let prepared = prepare_transfer(to_transfer_params(&v)).await?.value;
        let quote_id = prepared.quote_id;
        log::debug!("{LOG_PREFIX} prepared transfer quote_id={quote_id}");

        let exec = execute_prepared(ExecutePreparedParams {
            quote_id: quote_id.clone(),
            confirmed: true,
        })
        .await?
        .value;
        Ok::<(String, String), String>((quote_id, exec.transaction_hash))
    })
    .await?;
    log::debug!(
        "{LOG_PREFIX} transfer broadcast tx={} quote_id={quote_id}",
        truncate(&on_chain_tx),
    );

    let payment_map = build_payment_map(signer, &v, &on_chain_tx, &ctx).await?;
    log::debug!(
        "{LOG_PREFIX} x402 authorization signed purpose={} nonce_present_in_challenge={}",
        ctx.purpose,
        v.nonce.is_some(),
    );

    Ok(FulfilledPayment {
        payment_map,
        on_chain_tx,
        quote_id,
    })
}

// ── Small helpers ─────────────────────────────────────────────────────────────

/// Trim + treat empty as absent.
fn non_empty(field: &Option<String>) -> Option<String> {
    field
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// True when `expires_at` parses to a time within [`EXPIRY_SKEW_SECS`] of now.
/// Lenient on parse failure: logs and treats the challenge as non-expired (the
/// backend remains the source of truth on expiry).
fn is_expired(expires_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(expires_at) {
        Ok(exp) => {
            let cutoff = chrono::Utc::now() + chrono::Duration::seconds(EXPIRY_SKEW_SECS);
            exp.with_timezone(&chrono::Utc) < cutoff
        }
        Err(e) => {
            log::warn!(
                "{LOG_PREFIX} could not parse challenge expiry '{expires_at}': {e}; \
                 treating as non-expired"
            );
            false
        }
    }
}

/// Truncate an identifier for logs (`head…tail`). Char-based so it never panics
/// on a multi-byte UTF-8 boundary. Never used on secret material.
fn truncate(s: &str) -> String {
    let count = s.chars().count();
    if count <= 12 {
        s.to_string()
    } else {
        let head: String = s.chars().take(6).collect();
        let tail: String = s.chars().skip(count - 4).collect();
        format!("{head}…{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::{general_purpose::STANDARD as B64, Engine as _};
    use ed25519_dalek::{Signature, SigningKey, Verifier};
    use tinyplace::signer::LocalSigner;
    use tinyplace::x402::{build_canonical_message, X402AuthorizationFields};

    const TEST_SEED: [u8; 32] = [7u8; 32];

    fn test_signer() -> LocalSigner {
        LocalSigner::from_seed(&TEST_SEED).expect("test seed is 32 bytes")
    }

    fn future_expiry() -> String {
        (chrono::Utc::now() + chrono::Duration::minutes(10))
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
    }

    fn past_expiry() -> String {
        (chrono::Utc::now() - chrono::Duration::minutes(10))
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
    }

    fn mock_challenge(asset: &str) -> PaymentChallenge {
        PaymentChallenge {
            scheme: Some("exact".into()),
            network: Some("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".into()),
            asset: Some(asset.into()),
            amount: Some("10000000".into()),
            to: Some("FaciL1tatorBase58Recipient0000000000000000".into()),
            expires_at: Some(future_expiry()),
            ..Default::default()
        }
    }

    fn test_ctx() -> PaymentContext {
        let mut extra = HashMap::new();
        extra.insert("identity".to_string(), "@tester".to_string());
        PaymentContext {
            purpose: "identity.register".to_string(),
            nonce_prefix: "register".to_string(),
            extra_metadata: extra,
        }
    }

    // ── validate_challenge / asset routing ────────────────────────────────────

    #[test]
    fn validate_usdc_routes_to_spl() {
        let v = validate_challenge(&mock_challenge("USDC")).expect("valid");
        assert_eq!(v.asset_symbol.as_deref(), Some("USDC"));
        assert_eq!(v.asset, "USDC");
        assert_eq!(v.amount, "10000000");
        assert_eq!(v.to, "FaciL1tatorBase58Recipient0000000000000000");
    }

    #[test]
    fn validate_sol_routes_to_native() {
        let v = validate_challenge(&mock_challenge("SOL")).expect("valid");
        assert_eq!(v.asset_symbol, None);
        assert_eq!(v.asset, "SOL");
    }

    #[test]
    fn validate_rejects_unsupported_asset() {
        let err = validate_challenge(&mock_challenge("WBTC")).unwrap_err();
        assert!(err.contains("unsupported"), "got: {err}");
        assert!(err.contains("WBTC"), "got: {err}");
    }

    #[test]
    fn validate_rejects_missing_amount_or_to() {
        let mut c = mock_challenge("USDC");
        c.amount = None;
        assert!(validate_challenge(&c).unwrap_err().contains("amount"));

        let mut c = mock_challenge("USDC");
        c.to = Some("   ".into()); // whitespace counts as absent
        assert!(validate_challenge(&c).unwrap_err().contains("'to'"));

        let mut c = mock_challenge("USDC");
        c.network = None;
        assert!(validate_challenge(&c).unwrap_err().contains("network"));
    }

    #[test]
    fn validate_rejects_expired_challenge() {
        let mut c = mock_challenge("USDC");
        c.expires_at = Some(past_expiry());
        assert!(validate_challenge(&c).unwrap_err().contains("expired"));
    }

    #[test]
    fn validate_accepts_future_expiry() {
        let mut c = mock_challenge("USDC");
        c.expires_at = Some(future_expiry());
        assert!(validate_challenge(&c).is_ok());
        // Unparseable expiry is lenient (non-expired), not a hard failure.
        c.expires_at = Some("not-a-timestamp".into());
        assert!(validate_challenge(&c).is_ok());
    }

    // ── cluster guard (devnet R6; pure, env-independent) ──────────────────────

    #[test]
    fn classify_network_recognises_human_and_genesis_forms() {
        assert_eq!(
            classify_network("solana-devnet"),
            Some(SolanaCluster::Devnet)
        );
        assert_eq!(
            classify_network("solana-mainnet-beta"),
            Some(SolanaCluster::Mainnet)
        );
        // CAIP-2 genesis-hash references.
        assert_eq!(
            classify_network("solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1xyz"),
            Some(SolanaCluster::Devnet)
        );
        assert_eq!(
            classify_network("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"),
            Some(SolanaCluster::Mainnet)
        );
        // Unrecognised → None (allowed through; backend is source of truth).
        assert_eq!(classify_network("solana:someunknownhash"), None);
        assert_eq!(classify_network("solana-testnet"), None);
    }

    #[test]
    fn cluster_guard_is_advisory_never_blocks() {
        // Match → Ok.
        assert!(cluster_guard(SolanaCluster::Devnet, "solana-devnet").is_ok());
        assert!(cluster_guard(SolanaCluster::Mainnet, "solana-mainnet").is_ok());
        // Mismatch → still Ok (advisory only; tiny.place labels every cluster with
        // the mainnet genesis, so the label is not authoritative). This is the
        // exact case that previously blocked valid devnet payments.
        assert!(cluster_guard(SolanaCluster::Devnet, "solana-mainnet").is_ok());
        assert!(cluster_guard(
            SolanaCluster::Devnet,
            "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
        )
        .is_ok());
        assert!(cluster_guard(SolanaCluster::Mainnet, "solana-devnet").is_ok());
        // Unknown network → Ok regardless of configured cluster.
        assert!(cluster_guard(SolanaCluster::Devnet, "solana:unknownhash").is_ok());
    }

    #[test]
    fn ensure_cluster_matches_never_blocks() {
        // Env-independent: advisory check is Ok for any network, including the
        // mainnet-genesis label tiny.place emits on devnet.
        assert!(ensure_cluster_matches("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp").is_ok());
        assert!(ensure_cluster_matches("solana:unparseable-network-id").is_ok());
    }

    // ── mint cross-check (pure, no env, no network) ─────────────────────────

    /// Known USDC mint addresses for the test assertions.
    const MAINNET_USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
    const DEVNET_USDC: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";

    #[test]
    fn mint_cross_check_match_devnet() {
        // configured=devnet, backend=devnet → Ok
        assert!(mint_cross_check(DEVNET_USDC, Some(DEVNET_USDC)).is_ok());
    }

    #[test]
    fn mint_cross_check_match_mainnet() {
        // configured=mainnet, backend=mainnet → Ok
        assert!(mint_cross_check(MAINNET_USDC, Some(MAINNET_USDC)).is_ok());
    }

    #[test]
    fn mint_cross_check_mismatch_configured_mainnet_backend_devnet() {
        // configured=mainnet, backend=devnet → Err (the real-funds danger case)
        let err = mint_cross_check(MAINNET_USDC, Some(DEVNET_USDC)).unwrap_err();
        assert!(
            err.contains("cluster mismatch"),
            "error should mention mismatch, got: {err}"
        );
        assert!(
            err.contains(MAINNET_USDC),
            "error should cite the configured mint, got: {err}"
        );
        assert!(
            err.contains(DEVNET_USDC),
            "error should cite the backend mint, got: {err}"
        );
    }

    #[test]
    fn mint_cross_check_mismatch_configured_devnet_backend_mainnet() {
        // configured=devnet, backend=mainnet → Err
        let err = mint_cross_check(DEVNET_USDC, Some(MAINNET_USDC)).unwrap_err();
        assert!(err.contains("cluster mismatch"), "got: {err}");
    }

    #[test]
    fn mint_cross_check_backend_usdc_not_found() {
        // Backend did not include a USDC asset → Ok (fail-open)
        assert!(mint_cross_check(MAINNET_USDC, None).is_ok());
        assert!(mint_cross_check(DEVNET_USDC, None).is_ok());
    }

    #[test]
    fn mint_cross_check_with_unknown_mint() {
        // Backend reports an unknown mint → Err (mismatch)
        let err = mint_cross_check(DEVNET_USDC, Some("SomeOtherMintAddress123")).unwrap_err();
        assert!(err.contains("cluster mismatch"), "got: {err}");
    }

    // ── truncate (log helper) ─────────────────────────────────────────────────

    #[test]
    fn truncate_is_char_boundary_safe() {
        // ASCII base58/base64 (the real inputs) abbreviate as head…tail.
        assert_eq!(truncate("5SoLaNaTxSignature0000"), "5SoLaN…0000");
        assert_eq!(truncate("short"), "short");
        // Multi-byte UTF-8 must not panic on a byte-boundary slice.
        let multibyte = "日本語のながいテキストです１２３４";
        let out = truncate(multibyte); // would panic with byte slicing
        assert!(out.contains('…'));
    }

    // ── to_transfer_params ────────────────────────────────────────────────────

    #[test]
    fn transfer_params_shape() {
        let v = validate_challenge(&mock_challenge("USDC")).unwrap();
        let p = to_transfer_params(&v);
        assert_eq!(p.chain, WalletChain::Solana);
        assert_eq!(p.to_address, v.to);
        assert_eq!(p.amount_raw, "10000000");
        assert_eq!(p.asset_symbol.as_deref(), Some("USDC"));
        assert!(p.evm_network.is_none());
    }

    // ── build_payment_map (offline; deterministic test signer) ────────────────

    async fn build_map(asset: &str, ctx: &PaymentContext, on_chain_tx: &str) -> X402PaymentMap {
        let signer = test_signer();
        let v = validate_challenge(&mock_challenge(asset)).unwrap();
        build_payment_map(&signer, &v, on_chain_tx, ctx)
            .await
            .expect("payment map")
    }

    #[tokio::test]
    async fn payment_map_has_core_fields() {
        let signer = test_signer();
        let map = build_map(
            "USDC",
            &test_ctx(),
            "5SoLaNaTxSignature000000000000000000000000",
        )
        .await;
        assert_eq!(map.get("scheme").map(String::as_str), Some("exact"));
        assert_eq!(
            map.get("network").map(String::as_str),
            Some("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp")
        );
        assert_eq!(map.get("asset").map(String::as_str), Some("USDC"));
        assert_eq!(map.get("amount").map(String::as_str), Some("10000000"));
        assert_eq!(
            map.get("from").map(String::as_str),
            Some(signer.agent_id().as_str())
        );
        assert_eq!(
            map.get("to").map(String::as_str),
            Some("FaciL1tatorBase58Recipient0000000000000000")
        );
        assert!(map.contains_key("nonce"));
        assert!(map.contains_key("expiresAt"));
        assert!(map.contains_key("signature"));
    }

    #[tokio::test]
    async fn on_chain_tx_in_references_not_signature() {
        let tx = "5SoLaNaTxSignature000000000000000000000000";
        let map = build_map("USDC", &test_ctx(), tx).await;
        // On-chain tx is carried as references, top-level and in signed metadata.
        assert_eq!(map.get("onChainTx").map(String::as_str), Some(tx));
        assert_eq!(map.get("tx").map(String::as_str), Some(tx));
        assert_eq!(map.get("transaction").map(String::as_str), Some(tx));
        // The `signature` field is the off-chain Ed25519 authorization sig — NOT
        // the on-chain tx signature.
        assert_ne!(map.get("signature").map(String::as_str), Some(tx));
        let sig = map.get("signature").expect("signature present");
        let raw = B64.decode(sig).expect("signature is base64");
        assert_eq!(raw.len(), 64, "Ed25519 signature is 64 bytes");
    }

    #[tokio::test]
    async fn purpose_and_extra_metadata_in_map() {
        let map = build_map(
            "USDC",
            &test_ctx(),
            "5SoLaNaTx000000000000000000000000000000000",
        )
        .await;
        assert_eq!(
            map.get("metadata.purpose").map(String::as_str),
            Some("identity.register")
        );
        assert_eq!(
            map.get("metadata.identity").map(String::as_str),
            Some("@tester")
        );
    }

    #[tokio::test]
    async fn nonce_prefix_used_when_challenge_nonce_absent() {
        // mock_challenge leaves nonce = None.
        let map = build_map(
            "USDC",
            &test_ctx(),
            "5SoLaNaTx000000000000000000000000000000000",
        )
        .await;
        let nonce = map.get("nonce").expect("nonce");
        assert!(nonce.starts_with("register_"), "got: {nonce}");
    }

    #[tokio::test]
    async fn challenge_nonce_preferred_when_present() {
        let signer = test_signer();
        let mut c = mock_challenge("USDC");
        c.nonce = Some("challenge-supplied-nonce".into());
        let v = validate_challenge(&c).unwrap();
        let map = build_payment_map(
            &signer,
            &v,
            "5SoLaNaTx000000000000000000000000000000000",
            &test_ctx(),
        )
        .await
        .unwrap();
        assert_eq!(
            map.get("nonce").map(String::as_str),
            Some("challenge-supplied-nonce")
        );
    }

    #[tokio::test]
    async fn signature_verifies_against_pubkey() {
        // Reconstruct the signed canonical message from the flattened map and
        // verify the authorization signature against the signer's public key —
        // exactly what the backend does.
        let tx = "5SoLaNaTxSignature000000000000000000000000";
        let map = build_map("USDC", &test_ctx(), tx).await;

        let metadata: HashMap<String, String> = map
            .iter()
            .filter_map(|(k, val)| {
                k.strip_prefix("metadata.")
                    .map(|kk| (kk.to_string(), val.clone()))
            })
            .collect();
        let fields = X402AuthorizationFields {
            scheme: map["scheme"].clone(),
            network: map["network"].clone(),
            asset: map["asset"].clone(),
            amount: map["amount"].clone(),
            from: map["from"].clone(),
            to: map["to"].clone(),
            nonce: map["nonce"].clone(),
            expires_at: map["expiresAt"].clone(),
            metadata: Some(metadata),
        };
        let canonical = build_canonical_message(&fields);

        let sig_bytes = B64.decode(&map["signature"]).expect("base64 signature");
        let signature = Signature::from_slice(&sig_bytes).expect("64-byte signature");
        let verifying_key = SigningKey::from_bytes(&TEST_SEED).verifying_key();

        verifying_key
            .verify(canonical.as_bytes(), &signature)
            .expect("authorization signature verifies over the canonical message");
    }
}
