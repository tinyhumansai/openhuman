//! The host's implementations of the `tinywallet-web3` seams, and the process
//! wide engine and service built on them.
//!
//! `tinywallet-web3` holds the wallet, swap, bridge and dapp logic and none of
//! its configuration; this file is where OpenHuman's own state is plugged in:
//!
//! | Seam | Host implementation |
//! | --- | --- |
//! | `Transport` | [`OpenHumanTransport`](super::wallet::transport::OpenHumanTransport) over the wallet RPC layer |
//! | `RpcEndpoints` | [`HostEndpoints`](super::wallet::endpoints::HostEndpoints), the `OPENHUMAN_WALLET_RPC_*` env |
//! | `WalletSigner` | [`HostSigner`]: keyring, then the loaded wallet module |
//! | `WalletAccounts` | [`HostAccounts`]: the wallet's stored state |
//! | `QuoteScope` | [`TaskLocalScope`]: the chat turn's `APPROVAL_CHAT_CONTEXT` |
//! | `Web3Backend` | [`HostBackend`]: the integrations client |
//!
//! State is instance-owned in the crate; the process holds exactly one engine
//! and one service, built lazily the first time either is needed. Every seam
//! resolves its configuration per call, so building early costs nothing.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::Value;
use tinywallet_bus::wire::{
    DerivedAccount, Scheme, SecretMaterial, Signature, SignedTransaction, TransactionSpec,
};
use tinywallet_web3::crypto::seams::{WalletAccounts, WalletSigner, Web3Backend};
use tinywallet_web3::crypto::service::Web3Service;
use tinywallet_web3::crypto::wallet::{WalletEngine, WalletSeams, WalletStatus};
use tinywallet_web3::quote::QuoteOwner;
use tinywallet_web3::seams::QuoteScope;

use crate::config::rpc as config_rpc;
use crate::config::Config;
use crate::security::approval::APPROVAL_CHAT_CONTEXT;

use super::client::CryptoClient;
use super::wallet::endpoints::HostEndpoints;
use super::wallet::transport::OpenHumanTransport;
use {super::wallet, tinywallet_web3::crypto::wallet::WalletChain};

const LOG_PREFIX: &str = "[web3:seams]";

/// The engine and the service built over it.
struct Web3Runtime {
    engine: Arc<WalletEngine>,
    service: Arc<Web3Service>,
}

static RUNTIME: OnceLock<Web3Runtime> = OnceLock::new();

fn runtime() -> &'static Web3Runtime {
    RUNTIME.get_or_init(|| {
        log::debug!("{LOG_PREFIX} building the wallet engine and web3 service");
        let engine = Arc::new(WalletEngine::new(WalletSeams {
            transport: Arc::new(OpenHumanTransport::new()),
            endpoints: Arc::new(HostEndpoints),
            signer: Arc::new(HostSigner),
            accounts: Arc::new(HostAccounts),
            scope: Arc::new(TaskLocalScope),
        }));
        let service = Arc::new(Web3Service::new(engine.clone(), Arc::new(HostBackend)));
        Web3Runtime { engine, service }
    })
}

/// The process-wide wallet engine.
pub fn engine() -> Arc<WalletEngine> {
    runtime().engine.clone()
}

/// The process-wide swap/bridge/dapp service.
pub fn service() -> Arc<Web3Service> {
    runtime().service.clone()
}

// ---------------------------------------------------------------------------
// Signing
// ---------------------------------------------------------------------------

/// The chain's name as it appears in signer error messages.
fn label(chain: WalletChain) -> &'static str {
    match chain {
        WalletChain::Evm => "EVM",
        WalletChain::Btc => "BTC",
        WalletChain::Solana => "Solana",
        WalletChain::Tron => "Tron",
    }
}

/// Wording of a failed account derivation.
fn derive_failure(chain: WalletChain, error: &impl std::fmt::Display) -> String {
    format!("failed to derive the {} account: {error}", label(chain))
}

/// Wording of a failed transaction signature.
fn sign_transaction_failure(chain: WalletChain, error: &impl std::fmt::Display) -> String {
    format!("failed to sign {} transaction: {error}", label(chain))
}

/// Wording of a failed message signature.
fn sign_message_failure(chain: WalletChain, error: &impl std::fmt::Display) -> String {
    format!("failed to sign the {} message: {error}", label(chain))
}

/// Derives accounts and signs by handing the recovery phrase to the loaded
/// wallet module over a confidential call.
///
/// The phrase is decrypted here, for the one call, and sent only to a module
/// that has proved it is an artifact this build pinned (see
/// `modules::wallet::attested_proxy`). No private key is assembled in this
/// process, and no mnemonic enters `tinywallet-web3`.
///
/// Errors from before the module call (wallet not set up, keyring, config,
/// decrypt) are returned as they are; errors from the module carry the
/// per-operation prefix the wallet has always used.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostSigner;

impl HostSigner {
    /// The config and decrypted secret material for `chain`.
    async fn secret(chain: WalletChain) -> Result<(Config, SecretMaterial), String> {
        let secret = wallet::secret_material(chain).await?;
        let config = config_rpc::load_config_with_timeout().await?;
        let mnemonic =
            crate::security::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
                .await?
                .value;
        Ok((
            config,
            SecretMaterial {
                mnemonic,
                derivation_path: secret.derivation_path,
                chain: chain.to_chain(),
            },
        ))
    }
}

#[async_trait]
impl WalletSigner for HostSigner {
    async fn derive_account(&self, chain: WalletChain) -> Result<DerivedAccount, String> {
        let (config, secret) = Self::secret(chain).await?;
        crate::modules::wallet::derive_account(&config, &secret)
            .await
            .map_err(|e| derive_failure(chain, &e))
    }

    async fn sign_transaction(
        &self,
        chain: WalletChain,
        transaction: &TransactionSpec,
    ) -> Result<SignedTransaction, String> {
        let (config, secret) = Self::secret(chain).await?;
        crate::modules::wallet::sign_transaction_in_module(&config, transaction, &secret)
            .await
            .map_err(|e| sign_transaction_failure(chain, &e))
    }

    async fn sign_message(
        &self,
        chain: WalletChain,
        message: &[u8],
        scheme: Scheme,
    ) -> Result<Signature, String> {
        let (config, secret) = Self::secret(chain).await?;
        crate::modules::wallet::sign_message(&config, &secret, message, scheme)
            .await
            .map_err(|e| sign_message_failure(chain, &e))
    }
}

// ---------------------------------------------------------------------------
// Accounts, scope, backend
// ---------------------------------------------------------------------------

/// Reports the wallet's stored set-up state.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostAccounts;

#[async_trait]
impl WalletAccounts for HostAccounts {
    async fn status(&self) -> Result<WalletStatus, String> {
        Ok(wallet::status().await?.value)
    }
}

/// Reads the chat turn's owner from `APPROVAL_CHAT_CONTEXT`.
///
/// `Some(owner)` inside an interactive chat turn (the web channel installs the
/// task-local around `run_chat_task`), `None` for non-chat callers (CLI, direct
/// JSON-RPC, background triage, cron, sub-agents), which have no shared channel
/// a quote id could leak through and so stay executable without an owner.
///
/// `tokio::task_local!` propagates across `.await` but **not** across
/// `tokio::spawn`. If the chat path ever detaches the tool loop onto a
/// freshly-spawned task without re-installing the scope, this silently starts
/// returning `None` and the owner gate becomes a no-op. The crate calls this
/// synchronously on the tool's own task, and the regression test in
/// `seams_tests.rs` pins that it reads the task-local.
#[derive(Debug, Clone, Copy, Default)]
pub struct TaskLocalScope;

impl QuoteScope for TaskLocalScope {
    fn current_owner(&self) -> Option<QuoteOwner> {
        APPROVAL_CHAT_CONTEXT
            .try_with(|ctx| QuoteOwner {
                thread_id: ctx.thread_id.clone(),
                client_id: ctx.client_id.clone(),
            })
            .ok()
    }
}

/// The hosted swap/bridge quote service, over the integrations client.
///
/// Errors when the user is not signed in (no backend auth token), the same
/// gate the composio tools use.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostBackend;

impl HostBackend {
    async fn client() -> Result<CryptoClient, String> {
        let config = config_rpc::load_config_with_timeout().await?;
        CryptoClient::from_config(&config)
    }
}

#[async_trait]
impl Web3Backend for HostBackend {
    async fn routes(&self) -> Result<Value, String> {
        Self::client().await?.routes().await
    }

    async fn swap_tx(&self, body: &Value) -> Result<Value, String> {
        Self::client().await?.swap_tx(body).await
    }

    async fn bridge_tx(&self, body: &Value) -> Result<Value, String> {
        Self::client().await?.bridge_tx(body).await
    }
}

#[cfg(test)]
#[path = "seams_tests.rs"]
mod tests;
