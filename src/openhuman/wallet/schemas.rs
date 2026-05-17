use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::execution::{
    balances, chain_status, execute_prepared, network_defaults, prepare_contract_call,
    prepare_swap, prepare_transfer, supported_assets, ExecutePreparedParams,
    PrepareContractCallParams, PrepareSwapParams, PrepareTransferParams,
};
use super::ops::{WalletAccount, WalletSetupParams, WalletSetupSource};
use super::{encode_erc20_transfer, WalletChain};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupWalletParams {
    consent_granted: bool,
    source: WalletSetupSource,
    mnemonic_word_count: u8,
    encrypted_mnemonic: String,
    accounts: Vec<WalletAccount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EncodeErc20TransferParams {
    chain: WalletChain,
    to_address: String,
    amount_raw: String,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    all_wallet_controller_schemas()
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    all_wallet_registered_controllers()
}

pub fn schemas(function: &str) -> ControllerSchema {
    wallet_schemas(function)
}

pub fn all_wallet_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        wallet_schemas("status"),
        wallet_schemas("setup"),
        wallet_schemas("balances"),
        wallet_schemas("network_defaults"),
        wallet_schemas("supported_assets"),
        wallet_schemas("encode_erc20_transfer"),
        wallet_schemas("chain_status"),
        wallet_schemas("prepare_transfer"),
        wallet_schemas("prepare_swap"),
        wallet_schemas("prepare_contract_call"),
        wallet_schemas("execute_prepared"),
    ]
}

pub fn all_wallet_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: wallet_schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: wallet_schemas("setup"),
            handler: handle_setup,
        },
        RegisteredController {
            schema: wallet_schemas("balances"),
            handler: handle_balances,
        },
        RegisteredController {
            schema: wallet_schemas("network_defaults"),
            handler: handle_network_defaults,
        },
        RegisteredController {
            schema: wallet_schemas("supported_assets"),
            handler: handle_supported_assets,
        },
        RegisteredController {
            schema: wallet_schemas("encode_erc20_transfer"),
            handler: handle_encode_erc20_transfer,
        },
        RegisteredController {
            schema: wallet_schemas("chain_status"),
            handler: handle_chain_status,
        },
        RegisteredController {
            schema: wallet_schemas("prepare_transfer"),
            handler: handle_prepare_transfer,
        },
        RegisteredController {
            schema: wallet_schemas("prepare_swap"),
            handler: handle_prepare_swap,
        },
        RegisteredController {
            schema: wallet_schemas("prepare_contract_call"),
            handler: handle_prepare_contract_call,
        },
        RegisteredController {
            schema: wallet_schemas("execute_prepared"),
            handler: handle_execute_prepared,
        },
    ]
}

pub fn wallet_schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "wallet",
            function: "status",
            description: "Fetch core-owned local wallet metadata and onboarding status.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Wallet onboarding status plus safe multi-chain account metadata.",
                required: true,
            }],
        },
        "setup" => ControllerSchema {
            namespace: "wallet",
            function: "setup",
            description:
                "Persist local wallet consent and derived account metadata from the recovery phrase flow.",
            inputs: vec![
                required_json("consentGranted", "Whether the user explicitly consented to wallet setup."),
                required_json("source", "Whether the recovery phrase was generated or imported."),
                required_json(
                    "mnemonicWordCount",
                    "The number of words in the validated recovery phrase.",
                ),
                FieldSchema {
                    name: "encryptedMnemonic",
                    ty: TypeSchema::String,
                    comment:
                        "Encrypted recovery phrase payload created via openhuman.encrypt_secret. Required for on-chain signing/broadcast.",
                    required: true,
                },
                required_json(
                    "accounts",
                    "Exactly one derived account for each supported chain: EVM, BTC, Solana, and Tron.",
                ),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Updated wallet status after saving the setup.",
                required: true,
            }],
        },
        "balances" => ControllerSchema {
            namespace: "wallet",
            function: "balances",
            description:
                "List native-asset balances for every derived wallet account. EVM balances are read live from the configured/default RPC; other chains remain placeholder-zero until provider support lands.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of balance rows: {chain, address, assetSymbol, decimals, raw, formatted, providerStatus}.",
                required: true,
            }],
        },
        "network_defaults" => ControllerSchema {
            namespace: "wallet",
            function: "network_defaults",
            description:
                "List default RPC URLs, explorer bases, capability flags, and asset catalogs for supported wallet chains.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of {chain, network, chainId?, rpcUrl, rpcSource, explorerTxUrlBase, supportsBroadcast, supportsTokenTransfers, supportsContractCalls, assets[]}.",
                required: true,
            }],
        },
        "supported_assets" => ControllerSchema {
            namespace: "wallet",
            function: "supported_assets",
            description:
                "Catalog of built-in asset defaults the wallet surface understands, including default ERC-20s on EVM.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of {chain, symbol, name, native, decimals, contractAddress?}.",
                required: true,
            }],
        },
        "encode_erc20_transfer" => ControllerSchema {
            namespace: "wallet",
            function: "encode_erc20_transfer",
            description:
                "Encode ERC-20 transfer(address,uint256) calldata for EVM token sends.",
            inputs: vec![
                required_json("chain", "Target chain. Must be evm."),
                required_json("toAddress", "Recipient EVM address."),
                required_json("amountRaw", "Token amount in the token's smallest unit as a decimal string."),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "0x-prefixed calldata string for transfer(address,uint256).",
                required: true,
            }],
        },
        "chain_status" => ControllerSchema {
            namespace: "wallet",
            function: "chain_status",
            description:
                "Per-chain readiness: whether a wallet account is derived plus the active RPC URL (default or env override).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Array of {chain, configured, providerStatus, rpcUrl}.",
                required: true,
            }],
        },
        "prepare_transfer" => ControllerSchema {
            namespace: "wallet",
            function: "prepare_transfer",
            description:
                "Build a native or token-transfer quote. For EVM, default ERC-20 symbols are supported and wallet.execute_prepared will sign+broadcast after explicit confirmation.",
            inputs: vec![
                required_json("chain", "Target chain (evm | btc | solana | tron)."),
                required_json("toAddress", "Recipient address on the target chain."),
                required_json("amountRaw", "Amount in the asset's smallest unit (wei/sat/lamport) as a decimal string."),
                FieldSchema {
                    name: "assetSymbol",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional. Omit / null for the chain's native asset; otherwise a token symbol from wallet.supported_assets.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "PreparedTransaction with quoteId, simulated fee, and expiry.",
                required: true,
            }],
        },
        "prepare_swap" => ControllerSchema {
            namespace: "wallet",
            function: "prepare_swap",
            description:
                "Build a swap quote against a router/aggregator. Caller selects the router; this layer enforces simulation and a minimum-out floor.",
            inputs: vec![
                required_json("chain", "Target chain (evm | btc | solana | tron)."),
                required_json("fromSymbol", "Asset symbol being sold."),
                required_json("toSymbol", "Asset symbol being bought (must differ from fromSymbol)."),
                required_json("amountInRaw", "Input amount in the from-asset's smallest unit, as a decimal string."),
                required_json("slippageBps", "Slippage tolerance in basis points (max 5000 = 50%)."),
                required_json("routerAddress", "Router / aggregator contract address."),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "PreparedTransaction with quoteId, receiveSymbol, minReceiveRaw.",
                required: true,
            }],
        },
        "prepare_contract_call" => ControllerSchema {
            namespace: "wallet",
            function: "prepare_contract_call",
            description:
                "Build a contract-call quote for EVM. Caller supplies pre-encoded calldata.",
            inputs: vec![
                required_json("chain", "Target chain. Must be evm."),
                required_json("contractAddress", "Target contract address."),
                required_json("calldata", "0x-prefixed hex calldata."),
                FieldSchema {
                    name: "valueRaw",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Native value attached, smallest unit. Defaults to '0'.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "PreparedTransaction with calldata, quoteId, and simulated fee.",
                required: true,
            }],
        },
        "execute_prepared" => ControllerSchema {
            namespace: "wallet",
            function: "execute_prepared",
            description:
                "Confirm and execute a previously prepared quote. EVM transfers and contract calls are signed in core from encrypted local secret material, then broadcast to the configured/default RPC.",
            inputs: vec![
                required_json("quoteId", "quoteId returned by a prior wallet.prepare_* call."),
                required_json("confirmed", "Must be true; explicit safety boundary between simulate and execute."),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "ExecutionResult payload: {quoteId, status, chain, transactionHash, explorerUrl?, transaction}.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "wallet",
            function: "unknown",
            description: "Unknown wallet controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        crate::openhuman::wallet::status()
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_setup(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload: SetupWalletParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        crate::openhuman::wallet::setup(WalletSetupParams {
            consent_granted: payload.consent_granted,
            source: payload.source,
            mnemonic_word_count: payload.mnemonic_word_count,
            encrypted_mnemonic: Some(payload.encrypted_mnemonic),
            accounts: payload.accounts,
        })
        .await?
        .into_cli_compatible_json()
    })
}

fn handle_balances(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { balances().await?.into_cli_compatible_json() })
}

fn handle_network_defaults(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { network_defaults().await?.into_cli_compatible_json() })
}

fn handle_supported_assets(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { supported_assets().await?.into_cli_compatible_json() })
}

fn handle_encode_erc20_transfer(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: EncodeErc20TransferParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        if parsed.chain != WalletChain::Evm {
            return Err("encode_erc20_transfer only supports the evm chain".to_string());
        }
        serde_json::to_value(encode_erc20_transfer(
            &parsed.to_address,
            &parsed.amount_raw,
        )?)
        .map_err(|e| format!("failed to encode ERC20 transfer output: {e}"))
    })
}

fn handle_chain_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { chain_status().await?.into_cli_compatible_json() })
}

fn handle_prepare_transfer(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: PrepareTransferParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        prepare_transfer(parsed).await?.into_cli_compatible_json()
    })
}

fn handle_prepare_swap(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: PrepareSwapParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        prepare_swap(parsed).await?.into_cli_compatible_json()
    })
}

fn handle_prepare_contract_call(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: PrepareContractCallParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        prepare_contract_call(parsed)
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_execute_prepared(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: ExecutePreparedParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        execute_prepared(parsed).await?.into_cli_compatible_json()
    })
}

fn required_json(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_lists_every_controller() {
        assert_eq!(all_wallet_controller_schemas().len(), 11);
    }

    #[test]
    fn all_controllers_lists_every_handler() {
        assert_eq!(all_wallet_registered_controllers().len(), 11);
    }

    #[test]
    fn status_schema_is_empty_input() {
        let schema = wallet_schemas("status");
        assert_eq!(schema.namespace, "wallet");
        assert_eq!(schema.function, "status");
        assert!(schema.inputs.is_empty());
    }

    #[test]
    fn setup_schema_requires_all_inputs() {
        let schema = wallet_schemas("setup");
        assert_eq!(schema.inputs.len(), 5);
        let encrypted = schema
            .inputs
            .iter()
            .find(|field| field.name == "encryptedMnemonic")
            .expect("encryptedMnemonic input present");
        assert!(encrypted.required);
    }

    #[test]
    fn execute_prepared_schema_takes_quote_id_and_confirmed() {
        let schema = wallet_schemas("execute_prepared");
        let names: Vec<&str> = schema.inputs.iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["quoteId", "confirmed"]);
    }

    #[test]
    fn prepare_transfer_schema_marks_asset_symbol_optional() {
        let schema = wallet_schemas("prepare_transfer");
        let asset = schema
            .inputs
            .iter()
            .find(|f| f.name == "assetSymbol")
            .expect("assetSymbol input present");
        assert!(!asset.required);
    }

    #[test]
    fn unknown_schema_maps_to_unknown() {
        let schema = wallet_schemas("wat");
        assert_eq!(schema.function, "unknown");
    }
}
