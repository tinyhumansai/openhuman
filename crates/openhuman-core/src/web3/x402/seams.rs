//! The host side of the `tinywallet-x402` seams.
//!
//! The crate never sees a mnemonic, a config or a proxy setting. It is handed:
//!
//! - a [`PaymentSigner`]: [`WalletPaymentSigner`] resolves the wallet's
//!   encrypted secret from the keyring state, decrypts it with the configured
//!   key, and asks the loaded `tinywallet` module to derive the account and sign.
//!   The decrypted phrase lives only for the length of one confidential call;
//!   what crosses back to the crate is an address and finished signatures.
//! - a `Transport`: [`OpenHumanTransport`](crate::web3::wallet::transport::OpenHumanTransport),
//!   the failover-aware RPC adapter the wallet already uses, for the Solana
//!   blockhash.
//! - a [`ProxyPolicy`]: [`RuntimeProxyPolicy`] refuses direct, address-pinned
//!   requests when runtime or environment proxy policy requires a proxy.
//! - a [`ThreadScope`]: [`TaskLocalThread`] names the chat thread running the
//!   tool call, so the ledger can attribute the payment to it.
//!
//! Every `Err(String)` these return is the text a user sees, prefixed exactly as
//! the payment flow did before it moved into the crate (`wallet secret: …`,
//! `load config: …`, `decrypt mnemonic: …`, `derive account: …`).

use std::sync::Arc;

use async_trait::async_trait;
use log::debug;
use tinytools_std::network::NetGate;
use tinytools_std::url_guard::{normalize_allowed_domains, validate_url_with_dns_check};
use tinywallet_x402::crypto::{CryptoPayments, PaymentAccount, PaymentSigner, SignScheme};
use tinywallet_x402::protocol::ProxyPolicy;
use tinywallet_x402::thread::ThreadScope;
use tinywallet_x402::tools::{
    AuthorizedRequest, ProposedRequest, RequestAuthorizationError, RequestGuard, X402RequestTool,
};
use tinywallet_x402::wire::PaymentChain;

use crate::security::approval::APPROVAL_CHAT_CONTEXT;
use crate::security::SecurityPolicy;
use crate::web3::wallet::transport::OpenHumanTransport;
use tinywallet_web3::crypto::wallet::WalletChain;

const LOG_PREFIX: &str = "[x402::seams]";

/// The wallet as x402 signs with it: keyring secret, decrypt, wallet module.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WalletPaymentSigner;

/// The runtime and environment proxy configuration for x402 outbound HTTP.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RuntimeProxyPolicy;

#[derive(Debug)]
pub(crate) struct HostRequestGuard {
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
}

#[async_trait]
impl RequestGuard for HostRequestGuard {
    fn needs_approval(&self) -> bool {
        self.security.network_needs_approval()
    }

    async fn authorize(
        &self,
        request: &ProposedRequest,
    ) -> Result<AuthorizedRequest, RequestAuthorizationError> {
        if !self.security.can_act() {
            return Err(RequestAuthorizationError::Denied(
                "Action blocked: autonomy is read-only".into(),
            ));
        }
        let parsed = reqwest::Url::parse(&request.url)
            .map_err(|error| RequestAuthorizationError::InvalidDestination(error.to_string()))?;
        if parsed.scheme() != "https" {
            return Err(RequestAuthorizationError::InvalidDestination(
                "x402 payment requests require HTTPS".into(),
            ));
        }
        let host = parsed
            .host_str()
            .map(str::to_string)
            .unwrap_or_else(|| "unknown".to_string());
        if let Some(reason) = self.security.local_only_block(&host) {
            return Err(RequestAuthorizationError::Denied(
                reason
                    .strip_prefix("[policy-blocked] ")
                    .unwrap_or(&reason)
                    .to_string(),
            ));
        }
        if self.security.is_rate_limited() {
            return Err(RequestAuthorizationError::Denied(
                "Action blocked: rate limit exceeded".into(),
            ));
        }
        let target = validate_url_with_dns_check(&request.url, &self.allowed_domains)
            .await
            .map_err(|error| RequestAuthorizationError::InvalidDestination(error.to_string()))?;
        if !self.security.record_action() {
            return Err(RequestAuthorizationError::Denied(
                "Action blocked: rate limit exceeded".into(),
            ));
        }
        // A 402 retry adds PAYMENT-SIGNATURE even when the original request
        // has no headers. Disclose that potential metadata before either send.
        self.security
            .disclose(&target.host, request.body.is_some(), true);
        let mut approved_request = request.clone();
        approved_request.url = target.url;
        Ok(AuthorizedRequest {
            request: approved_request,
            host: target.host,
            addrs: target.addrs,
        })
    }
}

/// The chat thread running the current tool call, read from
/// `APPROVAL_CHAT_CONTEXT`.
///
/// `Some(thread_id)` inside an interactive chat turn (the web channel installs the
/// task-local around the run), `None` for CLI, direct JSON-RPC, cron and other
/// non-chat callers, whose payments then carry no `thread_id`. The ledger's
/// `session_id` never comes from here: it is always the ledger's own.
///
/// `tokio::task_local!` propagates across `.await` but **not** across
/// `tokio::spawn`; the crate calls this synchronously on the tool's own task, and
/// the regression test in `seams_tests.rs` pins that it reads the task-local.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TaskLocalThread;

impl ThreadScope for TaskLocalThread {
    fn current_thread(&self) -> Option<String> {
        APPROVAL_CHAT_CONTEXT
            .try_with(|ctx| ctx.thread_id.clone())
            .ok()
    }
}

/// The wallet chain a payment chain is signed as.
fn wallet_chain(chain: PaymentChain) -> WalletChain {
    match chain {
        PaymentChain::Solana => WalletChain::Solana,
        PaymentChain::Evm => WalletChain::Evm,
    }
}

/// The bus chain the wallet module derives for.
fn bus_chain(chain: PaymentChain) -> tinywallet_bus::Chain {
    match chain {
        PaymentChain::Solana => tinywallet_bus::Chain::Solana,
        PaymentChain::Evm => tinywallet_bus::Chain::Evm,
    }
}

/// Resolve the phrase to sign with: the config, and the wallet's decrypted
/// mnemonic packaged for a confidential call.
///
/// Derivation and signing both happen inside the loaded wallet module, so this
/// process never holds a private key. The phrase is handed over on a
/// confidential call, and only to a module that has proved it is an artifact this
/// build pinned.
async fn signing_secret(
    chain: PaymentChain,
) -> Result<(crate::config::Config, tinywallet_bus::wire::SecretMaterial), String> {
    let secret = crate::web3::wallet::secret_material(wallet_chain(chain))
        .await
        .map_err(|e| format!("wallet secret: {e}"))?;

    let config = crate::config::rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("load config: {e}"))?;

    let mnemonic =
        crate::security::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
            .await
            .map_err(|e| format!("decrypt mnemonic: {e}"))?
            .value;

    Ok((
        config,
        tinywallet_bus::wire::SecretMaterial {
            mnemonic,
            derivation_path: secret.derivation_path.clone(),
            chain: bus_chain(chain),
        },
    ))
}

#[async_trait]
impl PaymentSigner for WalletPaymentSigner {
    async fn account(&self, chain: PaymentChain) -> Result<PaymentAccount, String> {
        debug!("{LOG_PREFIX} account chain={chain:?}");
        let (config, secret) = signing_secret(chain).await?;
        let account = crate::modules::wallet::derive_account(&config, &secret)
            .await
            .map_err(|e| match chain {
                PaymentChain::Solana => format!("derive account: {e}"),
                PaymentChain::Evm => format!("derive EVM signer: {e}"),
            })?;
        // The module answers with the address only; the crate decodes a Solana
        // public key from it.
        Ok(PaymentAccount {
            address: account.address,
            pubkey: None,
        })
    }

    async fn sign(
        &self,
        chain: PaymentChain,
        message: &[u8],
        scheme: SignScheme,
    ) -> Result<Vec<u8>, String> {
        debug!(
            "{LOG_PREFIX} sign chain={chain:?} scheme={scheme:?} bytes={}",
            message.len()
        );
        let (config, secret) = signing_secret(chain).await?;
        let wire_scheme = match scheme {
            SignScheme::Ed25519 => tinywallet_bus::wire::Scheme::Ed25519,
            SignScheme::Secp256k1Digest => tinywallet_bus::wire::Scheme::Secp256k1Prehash,
        };
        // Signed in the module; the private key is never assembled here.
        let signature =
            crate::modules::wallet::sign_message(&config, &secret, message, wire_scheme)
                .await
                .map_err(|e| e.to_string())?;
        signature_bytes(signature)
    }
}

/// The raw bytes the crate expects from the module's signature: 64 for ed25519,
/// `r ‖ s ‖ recovery_id` (65) for secp256k1.
fn signature_bytes(signature: tinywallet_bus::wire::Signature) -> Result<Vec<u8>, String> {
    match signature {
        tinywallet_bus::wire::Signature::Ed25519 { signature_hex } => {
            hex::decode(&signature_hex).map_err(|e| format!("invalid signature hex: {e}"))
        }
        tinywallet_bus::wire::Signature::Secp256k1 {
            rs_hex,
            recovery_id,
        } => {
            let mut bytes =
                hex::decode(&rs_hex).map_err(|e| format!("invalid signature hex: {e}"))?;
            if bytes.len() != 64 {
                return Err("the wallet module returned a malformed signature".to_string());
            }
            bytes.push(recovery_id);
            Ok(bytes)
        }
        // `Signature` is non-exhaustive; a scheme this build has never heard of
        // cannot be paid with.
        _ => Err("the wallet module returned an unsupported signature".to_string()),
    }
}

impl ProxyPolicy for RuntimeProxyPolicy {
    fn apply(&self, builder: reqwest13::ClientBuilder, service: &str) -> reqwest13::ClientBuilder {
        super::proxy_compat::apply(builder, service, &crate::config::runtime_proxy_config())
    }

    fn allows_direct_connection(&self, service: &str) -> bool {
        let config = crate::config::runtime_proxy_config();
        direct_connection_allowed(&config, service, |key| {
            std::env::var_os(key).is_some_and(|value| !value.is_empty())
        })
    }
}

/// A guarded request must connect to its vetted address directly. Treat any
/// configured proxy for this service, including process proxy variables, as
/// requiring the proxy until the host can pin an address through that proxy.
fn direct_connection_allowed(
    config: &crate::config::ProxyConfig,
    service: &str,
    env_has_value: impl Fn(&str) -> bool,
) -> bool {
    const PROXY_ENV_KEYS: &[&str] = &[
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ];
    !((config.enabled && config.scope == crate::config::ProxyScope::Environment)
        || config.should_apply_to_service(service)
        || PROXY_ENV_KEYS.iter().any(|key| env_has_value(key)))
}

/// The crypto rail's payment builder, over OpenHuman's wallet and transport.
pub(crate) fn payments() -> CryptoPayments {
    CryptoPayments::new(
        Arc::new(WalletPaymentSigner),
        Arc::new(OpenHumanTransport::new()),
    )
}

/// The `x402_request` tool, over the same seams.
pub(crate) fn request_tool(
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
) -> X402RequestTool {
    X402RequestTool::new(
        Arc::new(WalletPaymentSigner),
        Arc::new(OpenHumanTransport::new()),
        Arc::new(RuntimeProxyPolicy),
    )
    .with_thread_scope(Arc::new(TaskLocalThread))
    .with_request_guard(Arc::new(host_request_guard(security, allowed_domains)))
}

fn host_request_guard(
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
) -> HostRequestGuard {
    HostRequestGuard {
        security,
        allowed_domains: normalize_allowed_domains(allowed_domains),
    }
}

#[cfg(test)]
#[path = "seams_tests.rs"]
mod tests;
