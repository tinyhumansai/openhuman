//! Endpoint selection and transport failure classification for wallet callbacks.

pub(super) enum TransportFailure {
    Unreachable(String),
    Rpc(String),
}

pub(super) fn classify_transport_error(message: String) -> TransportFailure {
    if ["transport failed", "read body failed"]
        .iter()
        .any(|marker| message.contains(marker))
    {
        TransportFailure::Unreachable(message)
    } else {
        TransportFailure::Rpc(message)
    }
}

pub(super) fn validate_module_endpoint_url(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw)
        .map_err(|_| "wallet module selected an invalid RPC endpoint URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("wallet module selected an unsafe RPC endpoint URL".into());
    }
    Ok(raw.to_owned())
}

pub(super) fn endpoint_key(
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

pub(super) fn join_endpoint(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}
