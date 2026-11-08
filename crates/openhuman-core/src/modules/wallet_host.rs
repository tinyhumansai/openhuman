//! OpenHuman's typed host callbacks for the loadable TinyWallet module.
//!
//! The wallet module owns network/quote/payment algorithms and signing. This
//! service supplies only current public account facts, endpoint-backed chain
//! transport, authenticated wallet-backend transport, and a confirmation
//! decision bound to an in-flight confirmed execution request.

use crate::core::runtime::DispatchCtx;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Mutex, OnceLock};

use tinybus::ObjectPath;
use tinywallet_bus::intent::{native_intent_matches_quote, web3_approval_matches_request};
use tinywallet_bus::wire::HostEndpointReference;
use tinywallet_bus::wire::{
    AuthorizationResult, AuthorizeNativeWalletExecutionRequest, AuthorizeWeb3ExecutionRequest,
    AuthorizeX402PaymentRequest, ChainTransportRequest, ChainTransportResult, HostResponse,
    RpcEndpointKey, WalletAccountInfo, WalletAccountsResponse, WalletBackendRequest,
    WalletBackendResponse,
};

#[path = "wallet_host_transport.rs"]
mod transport;

use transport::{
    classify_transport_error, endpoint_key, join_endpoint, validate_module_endpoint_url,
    TransportFailure,
};

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
static WEB3_APPROVAL_QUOTES: OnceLock<
    Mutex<HashMap<String, tinywallet_bus::wire::Web3QuoteResponse>>,
> = OnceLock::new();
static ENDPOINT_REFERENCES: OnceLock<Mutex<HashMap<String, EndpointReference>>> = OnceLock::new();

#[derive(Debug, Clone)]
struct EndpointReference {
    service: String,
    key: RpcEndpointKey,
}

fn approvals() -> &'static Mutex<HashSet<ApprovalKey>> {
    APPROVALS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn native_approval_quotes(
) -> &'static Mutex<HashMap<String, tinywallet_bus::wire::PreparedTransaction>> {
    NATIVE_APPROVAL_QUOTES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn web3_approval_quotes() -> &'static Mutex<HashMap<String, tinywallet_bus::wire::Web3QuoteResponse>>
{
    WEB3_APPROVAL_QUOTES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn endpoint_references() -> &'static Mutex<HashMap<String, EndpointReference>> {
    ENDPOINT_REFERENCES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether the payment was quoted for the chat the acting turn is in.
///
/// An x402 payment is approved with a forced approval, which only a `WebChat`
/// turn can raise, and that origin carries the same thread and client the turn's
/// chat context does. The origin is therefore the chat to compare against; the
/// callback arrives over the bus with no run context of its own.
fn x402_request_owner_matches(
    request: &AuthorizeX402PaymentRequest,
    origin: Option<&crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> bool {
    let Some(crate::core::runtime::turn_origin::AgentTurnOrigin::WebChat {
        thread_id,
        client_id,
        ..
    }) = origin
    else {
        return false;
    };
    request
        .owner
        .as_ref()
        .is_some_and(|owner| &owner.thread_id == thread_id && &owner.client_id == client_id)
}

fn x402_approval_facts(request: &AuthorizeX402PaymentRequest) -> serde_json::Value {
    serde_json::json!({
        "url": request.url,
        "network": request.network,
        "asset": request.asset,
        "recipient": request.recipient,
        "amount_atomic": request.amount_atomic,
    })
}

/// Bind module-issued opaque endpoint aliases to the current host configuration.
/// Actual URLs remain in this process and are never serialized onto TinyBus.
pub(crate) fn bind_endpoint_references(
    service: &str,
    references: &[HostEndpointReference],
) -> Result<(), String> {
    let mut resolved = HashMap::new();
    for endpoint in references {
        // Validate admission here, but resolve the URL again on each callback:
        // the host may rotate an endpoint override without changing its opaque
        // module reference.
        configured_endpoint_url(endpoint.key)
            .map_err(|_| "wallet module referenced an endpoint without a host override")?;
        if endpoint.reference.trim().is_empty()
            || resolved
                .insert(
                    endpoint.reference.clone(),
                    EndpointReference {
                        service: service.to_owned(),
                        key: endpoint.key,
                    },
                )
                .is_some()
        {
            return Err("wallet module returned an invalid endpoint reference".into());
        }
    }
    endpoint_references()
        .lock()
        .map_err(|_| "wallet endpoint reference store is unavailable".to_string())?
        .extend(resolved);
    Ok(())
}

fn configured_endpoint_url(key: RpcEndpointKey) -> Result<String, String> {
    let url = crate::web3::wallet::endpoints::override_for_endpoint(key)
        .ok_or_else(|| "wallet endpoint override is no longer configured".to_string())?;
    validate_module_endpoint_url(&url)
}

pub(crate) fn clear_endpoint_references(service: &str) {
    if let Ok(mut references) = endpoint_references().lock() {
        references.retain(|_, endpoint| endpoint.service != service);
    }
}

#[cfg(test)]
pub(super) fn has_endpoint_reference_for_test(reference: &str) -> bool {
    endpoint_references()
        .lock()
        .is_ok_and(|references| references.contains_key(reference))
}

pub(crate) async fn wallet_accounts_snapshot(
    ctx: &DispatchCtx,
) -> Result<tinywallet_bus::wire::WalletAccountsResponse, String> {
    let status = crate::web3::wallet::status(ctx)
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
        expected_web3_quote: Option<tinywallet_bus::wire::Web3QuoteResponse>,
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
        if kind == ApprovalKind::Web3 {
            if let (Some(quote), Ok(mut active)) =
                (expected_web3_quote, web3_approval_quotes().lock())
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
        if self.0.kind == ApprovalKind::Web3 {
            if let Ok(mut active) = web3_approval_quotes().lock() {
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

/// Keep the confirmation permit alive only while the corresponding module
/// execution is in flight. The callback consumes it once and is correlated by
/// the immutable module-issued quote id.
pub(crate) async fn with_confirmed_execution<F: Future>(
    kind: ApprovalKind,
    quote_id: &str,
    expected_native_quote: Option<tinywallet_bus::wire::PreparedTransaction>,
    expected_web3_quote: Option<tinywallet_bus::wire::Web3QuoteResponse>,
    future: F,
) -> F::Output {
    let _permit = ApprovalPermit::new(kind, quote_id, expected_native_quote, expected_web3_quote);
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
            wallet_accounts_snapshot(&DispatchCtx::host_default())
                .await
                .map_err(callback_error)?,
        ))
    }

    /// Run the chain operation through OpenHuman's configured RPC transport.
    async fn chain_transport(
        &self,
        request: ChainTransportRequest,
    ) -> tinybus::Result<HostResponse> {
        let (endpoint_ref, module_endpoint_url, chain, evm_chain_id) = match &request {
            ChainTransportRequest::JsonRpc { network, .. }
            | ChainTransportRequest::RestGet { network, .. }
            | ChainTransportRequest::RestPost { network, .. } => (
                network.endpoint_ref.as_deref(),
                network.endpoint_url.as_deref(),
                network.chain,
                network.evm_chain_id,
            ),
        };
        let endpoint_url = match (endpoint_ref, module_endpoint_url) {
            (Some(_), Some(_)) => {
                return Ok(HostResponse::ChainTransport(
                    ChainTransportResult::RpcError {
                        message: "wallet request cannot combine an endpoint reference and URL"
                            .into(),
                    },
                ));
            }
            (Some(reference), None) => {
                let Some(endpoint) = endpoint_references()
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
                if Some(endpoint.key) != endpoint_key(chain, evm_chain_id) {
                    return Ok(HostResponse::ChainTransport(
                        ChainTransportResult::RpcError {
                            message:
                                "the wallet endpoint reference does not match the requested network"
                                    .into(),
                        },
                    ));
                }
                match configured_endpoint_url(endpoint.key) {
                    Ok(url) => url,
                    Err(message) => {
                        return Ok(HostResponse::ChainTransport(
                            ChainTransportResult::RpcError { message },
                        ));
                    }
                }
            }
            (None, Some(url)) => match validate_module_endpoint_url(url) {
                Ok(url) => url,
                Err(message) => {
                    return Ok(HostResponse::ChainTransport(
                        ChainTransportResult::RpcError { message },
                    ));
                }
            },
            (None, None) => {
                return Ok(HostResponse::ChainTransport(
                    ChainTransportResult::RpcError {
                        message: "wallet module did not select an RPC endpoint".into(),
                    },
                ));
            }
        };
        let result = match request {
            ChainTransportRequest::JsonRpc { method, params, .. } => {
                crate::web3::wallet::rpc::rpc_call_to::<serde_json::Value>(
                    &endpoint_url,
                    &method,
                    params,
                )
                .await
                .map_err(classify_transport_error)
            }
            ChainTransportRequest::RestGet { path, .. } => {
                crate::web3::wallet::rpc::rest_get_text(&join_endpoint(&endpoint_url, &path))
                    .await
                    .map(serde_json::Value::String)
                    .map_err(classify_transport_error)
            }
            ChainTransportRequest::RestPost {
                path,
                body,
                content_type,
                ..
            } => crate::web3::wallet::rpc::rest_post_text(
                &join_endpoint(&endpoint_url, &path),
                &body,
                &content_type,
            )
            .await
            .map(serde_json::Value::String)
            .map_err(classify_transport_error),
        };
        let response = match result {
            Ok(body) => ChainTransportResult::Ok { body },
            Err(TransportFailure::Unreachable(message)) => {
                ChainTransportResult::Unreachable { message }
            }
            Err(TransportFailure::Rpc(message)) => ChainTransportResult::RpcError { message },
        };
        Ok(HostResponse::ChainTransport(response))
    }

    /// Perform only the named authenticated wallet-backend request.
    async fn wallet_backend(&self, request: WalletBackendRequest) -> tinybus::Result<HostResponse> {
        let config = DispatchCtx::host_default()
            .load_config()
            .await
            .map_err(callback_error)?;
        let data = super::wallet_backend::call_wallet_backend(config, request.into())
            .await
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
        let quote = web3_approval_quotes()
            .lock()
            .ok()
            .and_then(|mut quotes| quotes.remove(&request.quote_id));
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved: quote
                .as_ref()
                .is_some_and(|quote| web3_approval_matches_request(quote, &request))
                && consume_approval(ApprovalKind::Web3, &request.quote_id),
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
        request: AuthorizeX402PaymentRequest,
    ) -> tinybus::Result<HostResponse> {
        let origin = crate::core::runtime::DispatchCtx::ambient_turn().turn_origin();
        let owner_matches = x402_request_owner_matches(&request, origin.as_ref());
        let approved = if owner_matches {
            if let Some(gate) = crate::security::approval::ApprovalGate::try_global() {
                matches!(
                    gate.intercept_forced(
                        "x402_payment",
                        "Approve this exact x402 payment",
                        x402_approval_facts(&request),
                    )
                    .await,
                    crate::security::approval::GateOutcome::Allow
                )
            } else {
                false
            }
        } else {
            false
        };
        Ok(HostResponse::Authorization(AuthorizationResult {
            approved,
            reason: (!approved)
                .then(|| "x402 payment was not explicitly approved for this chat".into()),
        }))
    }
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
            super::callback_owner::OwnedBy::new(
                WalletHost,
                connection,
                tinywallet_bus::names::BUS_NAME,
            ),
        )
        .await?;
    connection
        .request_name(tinywallet_bus::HOST_INTERFACE)
        .await
}

#[cfg(test)]
#[path = "wallet_host_tests.rs"]
mod tests;
