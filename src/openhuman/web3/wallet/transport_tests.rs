use super::*;

#[test]
fn join_does_not_double_the_separator() {
    assert_eq!(join("https://api.test/", "/tx"), "https://api.test/tx");
    assert_eq!(join("https://api.test", "tx"), "https://api.test/tx");
}

#[test]
fn an_unknown_evm_chain_id_is_authoritative_not_retryable() {
    // No endpoint is configured for it, so retrying elsewhere cannot help.
    let network = NetworkId::evm(999_999);
    let err = resolve(network).unwrap_err();
    assert!(!err.is_retryable(), "{err}");
}

#[test]
fn an_evm_request_without_a_chain_id_is_authoritative_not_retryable() {
    let err = resolve(NetworkId::chain(tinywallet_bus::Chain::Evm)).unwrap_err();
    assert!(!err.is_retryable(), "{err}");
}

#[test]
fn every_supported_evm_network_resolves() {
    for evm in EvmNetwork::ALL {
        let network = NetworkId::evm(evm.chain_id());
        assert!(resolve(network).is_ok(), "{} did not resolve", evm.as_str());
    }
}

#[test]
fn every_non_evm_chain_resolves() {
    for chain in [
        tinywallet_bus::Chain::Btc,
        tinywallet_bus::Chain::Solana,
        tinywallet_bus::Chain::Tron,
    ] {
        assert!(resolve(NetworkId::chain(chain)).is_ok(), "{chain}");
    }
}

#[test]
fn transport_failures_are_retryable_and_everything_else_is_not() {
    // The conservative direction: only what this layer knows to be a
    // transport failure may drive a failover.
    let network = NetworkId::chain(tinywallet_bus::Chain::Btc);
    assert!(classify(network, "wallet RPC transport failed for x: refused".into()).is_retryable());
    assert!(classify(network, "wallet RPC read body failed for x: eof".into()).is_retryable());
    assert!(
        !classify(network, "insufficient funds".into()).is_retryable(),
        "an authoritative answer must not be retried"
    );
    assert!(
        !classify(network, "something unrecognised".into()).is_retryable(),
        "an unclassifiable error must stop a failover, not drive it"
    );
}
