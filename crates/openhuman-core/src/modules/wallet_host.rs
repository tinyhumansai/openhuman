//! OpenHuman's typed host callbacks for the loadable TinyWallet module.
//!
//! The wallet module owns network/quote/payment algorithms and signing. This
//! service supplies only current public account facts, endpoint-backed chain
//! transport, authenticated wallet-backend transport, and a confirmation
//! decision bound to an in-flight confirmed execution request.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Mutex, OnceLock};

use tinybus::ObjectPath;
use tinywallet_bus::wire::{
    AuthorizationResult, AuthorizeNativeWalletExecutionRequest, AuthorizeWeb3ExecutionRequest,
    AuthorizeX402PaymentRequest, ChainTransportRequest, ChainTransportResult, HostResponse,
    WalletAccountInfo, WalletAccountsResponse, WalletBackendRequest, WalletBackendResponse,
};
use tinywallet_crypto::rpc::{NetworkId, Transport, TransportError};

const HOST_ERROR_NAME: &str = "ai.tinyhumans.tinywallet.Host.Error.CallbackFailed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ApprovalKind {
    Native,
    Web3,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ApprovalKey {
    kind: ApprovalKind,
    quote_id: String,
}

static APPROVALS: OnceLock<Mutex<HashSet<ApprovalKey>>> = OnceLock::new();
static NATIVE_APPROVAL_QUOTES: OnceLock<
    Mutex<HashMap<String, tinywallet_bus::wire::PreparedTransaction>>,
> = OnceLock::new();
static ENDPOINT_REFERENCES: OnceLock<
    Mutex<HashMap<String, (tinywallet_bus::wire::RpcEndpointKey, String)>>,
> = OnceLock::new();

fn approvals() -> &'static Mutex<HashSet<ApprovalKey>> {
    APPROVALS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn native_approval_quotes(
) -> &'static Mutex<HashMap<String, tinywallet_bus::wire::PreparedTransaction>> {
    NATIVE_APPROVAL_QUOTES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn endpoint_references(
) -> &'static Mutex<HashMap<String, (tinywallet_bus::wire::RpcEndpointKey, String)>> {
    ENDPOINT_REFERENCES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Bind module-issued opaque endpoint aliases to the current host configuration.
/// Actual URLs remain in this process and are never serialized onto TinyBus.
pub(crate) fn bind_endpoint_references(
    configured: &tinywallet_bus::wire::NativeWalletConfigResponse,
) -> Result<(), String> {
    use tinywallet_bus::wire::RpcEndpointKey;

    let mut resolved = HashMap::new();
    for endpoint in &configured.endpoint_references {
        let url = match endpoint.key {
            RpcEndpointKey::Evm { network } => {
                crate::web3::wallet::endpoints::rpc_url_for_evm_network(network)
            }
            RpcEndpointKey::Btc => crate::web3::wallet::endpoints::rpc_url_for_chain(
                tinywallet_web3::crypto::wallet::WalletChain::Btc,
            ),
            RpcEndpointKey::Solana => crate::web3::wallet::endpoints::rpc_url_for_chain(
                tinywallet_web3::crypto::wallet::WalletChain::Solana,
            ),
            RpcEndpointKey::Tron => crate::web3::wallet::endpoints::rpc_url_for_chain(
                tinywallet_web3::crypto::wallet::WalletChain::Tron,
            ),
        };
        if endpoint.reference.trim().is_empty()
            || resolved
                .insert(endpoint.reference.clone(), (endpoint.key, url))
                .is_some()
        {
            return Err("wallet module returned an invalid endpoint reference".into());
        }
    }
    *endpoint_references()
        .lock()
        .map_err(|_| "wallet endpoint reference store is unavailable".to_string())? = resolved;
    Ok(())
}

pub(crate) fn clear_endpoint_references() {
    if let Ok(mut references) = endpoint_references().lock() {
        references.clear();
    }
}

#[cfg(test)]
pub(super) fn has_endpoint_reference_for_test(reference: &str) -> bool {
    endpoint_references()
        .lock()
        .is_ok_and(|references| references.contains_key(reference))
}

pub(crate) async fn wallet_accounts_snapshot(
) -> Result<tinywallet_bus::wire::WalletAccountsResponse, String> {
    let status = crate::web3::wallet::status()
        .await
        .map_err(|error| error.to_string())?
        .value;
    let accounts = status
        .accounts
        .into_iter()
        .map(|account| WalletAccountInfo {
            chain: account.chain.to_chain(),
            address: account.address,
            derivation_path: account.derivation_path,
        })
        .collect();
    Ok(WalletAccountsResponse {
        configured: status.configured,
        accounts,
    })
}

/// One-shot authorization permit for an execution whose public RPC request
/// already carried `confirmed: true`.
struct ApprovalPermit(ApprovalKey);

impl ApprovalPermit {
    fn new(
        kind: ApprovalKind,
        quote_id: &str,
        expected_native_quote: Option<tinywallet_bus::wire::PreparedTransaction>,
    ) -> Self {
        let key = ApprovalKey {
            kind,
            quote_id: quote_id.to_owned(),
        };
        if let Ok(mut active) = approvals().lock() {
            active.insert(key.clone());
        }
        if kind == ApprovalKind::Native {
            if let (Some(quote), Ok(mut active)) =
                (expected_native_quote, native_approval_quotes().lock())
            {
                active.insert(quote_id.to_owned(), quote);
            }
        }
        Self(key)
    }
}

impl Drop for ApprovalPermit {
    fn drop(&mut self) {
        if let Ok(mut active) = approvals().lock() {
            active.remove(&self.0);
        }
        if self.0.kind == ApprovalKind::Native {
            if let Ok(mut active) = native_approval_quotes().lock() {
                active.remove(&self.0.quote_id);
            }
        }
    }
}

fn consume_approval(kind: ApprovalKind, quote_id: &str) -> bool {
    approvals().lock().is_ok_and(|mut active| {
        active.remove(&ApprovalKey {
            kind,
            quote_id: quote_id.to_owned(),
        })
    })
}

pub(super) fn native_approval_matches_request(
    expected_quote: &tinywallet_bus::wire::PreparedTransaction,
    request: &tinywallet_bus::wire::AuthorizeNativeWalletExecutionRequest,
) -> bool {
    let permit = consume_approval(ApprovalKind::Native, &request.quote_id);
    permit
        && expected_quote.quote_id == request.quote_id
        && expected_quote == &request.quote
        && request.operation == "transfer"
        && native_intent_matches_quote(expected_quote, &request.intent)
}

fn native_intent_matches_quote(
    quote: &tinywallet_bus::wire::PreparedTransaction,
    intent: &tinywallet_bus::wire::NativeWalletExecutionIntent,
) -> bool {
    use tinywallet_bus::wire::{
        NativeWalletExecutionIntent, PreparedKind, TransactionSpec, WalletChain,
    };

    match intent {
        NativeWalletExecutionIntent::Transaction { transaction } => match transaction {
            TransactionSpec::Btc {
                from,
                to,
                amount_sat,
                ..
            } => {
                quote.chain == WalletChain::Btc
                    && quote.kind == PreparedKind::NativeTransfer
                    && from == &quote.from_address
                    && to == &quote.to_address
                    && quote.amount_raw.parse::<u64>().ok() == Some(*amount_sat)
            }
            TransactionSpec::Evm {
                to,
                value_wei,
                data_hex,
                chain_id,
                ..
            } => {
                let network = quote
                    .evm_network
                    .unwrap_or(tinywallet_bus::wire::EvmNetwork::EthereumMainnet);
                if quote.chain != WalletChain::Evm || network.chain_id() != *chain_id {
                    return false;
                }
                match quote.kind {
                    PreparedKind::NativeTransfer => {
                        to == &quote.to_address
                            && value_wei == &quote.amount_raw
                            && data_hex.is_empty()
                    }
                    PreparedKind::TokenTransfer => {
                        let Some(contract) = quote.token_address.as_deref() else {
                            return false;
                        };
                        to.eq_ignore_ascii_case(contract)
                            && value_wei == "0"
                            && evm_transfer_data_matches(
                                data_hex,
                                &quote.to_address,
                                &quote.amount_raw,
                            )
                    }
                }
            }
            TransactionSpec::Solana {
                from, to, lamports, ..
            } => {
                quote.chain == WalletChain::Solana
                    && quote.kind == PreparedKind::NativeTransfer
                    && from == &quote.from_address
                    && to == &quote.to_address
                    && quote.amount_raw.parse::<u64>().ok() == Some(*lamports)
            }
            TransactionSpec::Tron {
                expected_to,
                transfer,
                ..
            } => {
                use tinywallet_bus::TronTransfer;
                if quote.chain != WalletChain::Tron || expected_to != &quote.to_address {
                    return false;
                }
                match (quote.kind, transfer) {
                    (PreparedKind::NativeTransfer, TronTransfer::Native { amount_sun }) => {
                        quote.amount_raw.parse::<u64>().ok() == Some(*amount_sun)
                    }
                    (PreparedKind::TokenTransfer, TronTransfer::Trc20 { parameter_hex }) => {
                        valid_hex(parameter_hex)
                    }
                    _ => false,
                }
            }
            _ => false,
        },
        NativeWalletExecutionIntent::SolanaMessage { message_hex } => {
            quote.chain == WalletChain::Solana
                && quote.kind == PreparedKind::TokenTransfer
                && valid_hex(message_hex)
        }
    }
}

fn evm_transfer_data_matches(data: &str, recipient: &str, amount: &str) -> bool {
    let Some(address) = recipient
        .strip_prefix("0x")
        .or_else(|| recipient.strip_prefix("0X"))
    else {
        return false;
    };
    if address.len() != 40 || !address.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let Some(amount_word) = decimal_u256_word(amount) else {
        return false;
    };
    let expected = format!("a9059cbb{:0>64}{amount_word}", address.to_ascii_lowercase());
    let actual = data
        .strip_prefix("0x")
        .or_else(|| data.strip_prefix("0X"))
        .unwrap_or(data);
    actual.eq_ignore_ascii_case(&expected)
}

fn decimal_u256_word(decimal: &str) -> Option<String> {
    if decimal.is_empty() || !decimal.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut bytes = [0u8; 32];
    for digit in decimal.bytes().map(|byte| byte - b'0') {
        let mut carry = u16::from(digit);
        for byte in bytes.iter_mut().rev() {
            let value = u16::from(*byte) * 10 + carry;
            *byte = (value & 0xff) as u8;
            carry = value >> 8;
        }
        if carry != 0 {
            return None;
        }
    }
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn valid_hex(value: &str) -> bool {
    !value.is_empty()
        && value.len().is_multiple_of(2)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Keep the confirmation permit alive only while the corresponding module
/// execution is in flight. The callback consumes it once and is correlated by
/// the immutable module-issued quote id.
pub(crate) async fn with_confirmed_execution<F: Future>(
    kind: ApprovalKind,
    quote_id: &str,
    expected_native_quote: Option<tinywallet_bus::wire::PreparedTransaction>,
    future: F,
) -> F::Output {
    let _permit = ApprovalPermit::new(kind, quote_id, expected_native_quote);
    future.await
}

#[derive(Clone, Copy)]
struct WalletHost;

#[tinybus::interface(name = "ai.tinyhumans.tinywallet.Host")]
impl WalletHost {
    /// Public wallet setup state and addresses; encrypted secrets never leave
    /// the host's custody store through this callback.
    async fn wallet_accounts(&self) -> tinybus::Result<HostResponse> {
        Ok(HostResponse::WalletAccounts(
            wallet_accounts_snapshot().await.map_err(callback_error)?,
        ))
    }

    /// Run the chain operation through OpenHuman's configured RPC transport.
    async fn chain_transport(
        &self,
        request: ChainTransportRequest,
    ) -> tinybus::Result<HostResponse> {
        let (network, endpoint_ref, chain, evm_chain_id) = match &request {
            ChainTransportRequest::JsonRpc { network, .. }
            | ChainTransportRequest::RestGet { network, .. }
            | ChainTransportRequest::RestPost { network, .. } => (
                network_id(network.chain, network.evm_chain_id),
                network.endpoint_ref.as_deref(),
                network.chain,
                network.evm_chain_id,
            ),
        };
        let endpoint_url = match endpoint_ref {
            Some(reference) => {
                let Some((key, url)) = endpoint_references()
                    .lock()
                    .ok()
                    .and_then(|references| references.get(reference).cloned())
                else {
                    return Ok(HostResponse::ChainTransport(
                        ChainTransportResult::RpcError {
                            message:
                                "the requested wallet endpoint reference is not active in this host"
                                    .into(),
                        },
                    ));
                };
                if Some(key) != endpoint_key(chain, evm_chain_id) {
                    return Ok(HostResponse::ChainTransport(
                        ChainTransportResult::RpcError {
                            message:
                                "the wallet endpoint reference does not match the requested network"
                                    .into(),
                        },
                    ));
                }
                Some(url)
            }
            None => None,
        };
        let transport = crate::web3::wallet::transport::OpenHumanTransport::new();
        let result = match request {
            ChainTransportRequest::JsonRpc { method, params, .. } => match endpoint_url {
                Some(url) => crate::web3::wallet::rpc::rpc_call_to::<serde_json::Value>(
                    &url, &method, params,
                )
                .await
                .map_err(|message| crate::web3::wallet::transport::classify(network, message)),
                None => transport.json_rpc(network, &method, params).await,
            },
            ChainTransportRequest::RestGet { path, .. } => match endpoint_url {
                Some(url) => crate::web3::wallet::rpc::rest_get_text(&join_endpoint(&url, &path))
                    .await
                    .map(serde_json::Value::String)
                    .map_err(|message| crate::web3::wallet::transport::classify(network, message)),
                None => transport
                    .rest_get(network, &path)
                    .await
                    .map(serde_json::Value::String),
            },
            ChainTransportRequest::RestPost {
                path,
                body,
                content_type,
                ..
            } => match endpoint_url {
                Some(url) => crate::web3::wallet::rpc::rest_post_text(
                    &join_endpoint(&url, &path),
                    &body,
                    &content_type,
                )
                .await
                .map(serde_json::Value::String)
                .map_err(|message| crate::web3::wallet::transport::classify(network, message)),
                None => transport
                    .rest_post(network, &path, body, &content_type)
                    .await
                    .map(serde_json::Value::String),
            },
        };
        let response = match result {
            Ok(body) => ChainTransportResult::Ok { body },
            Err(TransportError::Unreachable { message, .. }) => {
                ChainTransportResult::Unreachable { message }
            }
            Err(TransportError::Rpc { message, .. }) => ChainTransportResult::RpcError { message },
            Err(error) => ChainTransportResult::RpcError {
                message: error.to_string(),
            },
        };
        Ok(HostResponse::ChainTransport(response))
    }

    /// Perform only the named authenticated wallet-backend request.
    async fn wallet_backend(&self, request: WalletBackendRequest) -> tinybus::Result<HostResponse> {
        let config = crate::config::rpc::load_config_with_timeout()
            .await
            .map_err(callback_error)?;
        let client =
            crate::web3::client::CryptoClient::from_config(&config).map_err(callback_error)?;
        let data = match request {
            WalletBackendRequest::Routes => client.routes().await,
            WalletBackendRequest::SwapTransaction { body } => client.swap_tx(&body).await,
            WalletBackendRequest::BridgeTransaction { body } => client.bridge_tx(&body).await,
        }
        .map_err(callback_error)?;
        Ok(HostResponse::WalletBackend(WalletBackendResponse { data }))
    }

    /// A module callback is permitted only while the host is servicing the
    /// exact confirmed quote id. The module supplies the transaction intent
    /// built from its stored quote; the host never constructs or signs it.
    async fn authorize_web3_execution(
        &self,
        request: AuthorizeWeb3ExecutionRequest,
    ) -> tinybus::Result<HostResponse> {
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved: consume_approval(ApprovalKind::Web3, &request.quote_id),
            reason: None,
        }))
    }

    /// Approve the exact native quote facts only inside the corresponding
    /// confirmed host operation.
    async fn authorize_native_wallet_execution(
        &self,
        request: AuthorizeNativeWalletExecutionRequest,
    ) -> tinybus::Result<HostResponse> {
        let expected = native_approval_quotes()
            .lock()
            .ok()
            .and_then(|mut quotes| quotes.remove(&request.quote_id));
        let approved = expected
            .as_ref()
            .is_some_and(|quote| native_approval_matches_request(quote, &request));
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved,
            reason: None,
        }))
    }

    /// x402 authorization is supplied by the host's guarded HTTP request path;
    /// absent an active guard scope, payment signing fails closed.
    async fn authorize_x402_payment(
        &self,
        _request: AuthorizeX402PaymentRequest,
    ) -> tinybus::Result<HostResponse> {
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved: false,
            reason: Some("x402 payment has no active host authorization".into()),
        }))
    }
}

fn network_id(chain: tinywallet_bus::Chain, evm_chain_id: Option<u64>) -> NetworkId {
    if chain == tinywallet_bus::Chain::Evm {
        NetworkId {
            chain,
            evm_chain_id,
        }
    } else {
        NetworkId::chain(chain)
    }
}

fn endpoint_key(
    chain: tinywallet_bus::Chain,
    evm_chain_id: Option<u64>,
) -> Option<tinywallet_bus::wire::RpcEndpointKey> {
    use tinywallet_bus::wire::{EvmNetwork, RpcEndpointKey};
    match chain {
        tinywallet_bus::Chain::Evm => EvmNetwork::ALL
            .into_iter()
            .find(|network| Some(network.chain_id()) == evm_chain_id)
            .map(|network| RpcEndpointKey::Evm { network }),
        tinywallet_bus::Chain::Btc => Some(RpcEndpointKey::Btc),
        tinywallet_bus::Chain::Solana => Some(RpcEndpointKey::Solana),
        tinywallet_bus::Chain::Tron => Some(RpcEndpointKey::Tron),
        _ => None,
    }
}

fn join_endpoint(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn callback_error(error: impl std::fmt::Display) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: HOST_ERROR_NAME.to_string(),
        message: error.to_string(),
    }
}

/// Install the host callback interface before any TinyWallet module can load.
pub(super) async fn install(connection: &tinybus::Connection) -> tinybus::Result<()> {
    connection
        .serve_at(
            ObjectPath::new(tinywallet_bus::HOST_OBJECT_PATH)?,
            WalletHost,
        )
        .await?;
    connection
        .request_name(tinywallet_bus::HOST_INTERFACE)
        .await
}

#[cfg(test)]
#[path = "wallet_host_tests.rs"]
mod tests;
