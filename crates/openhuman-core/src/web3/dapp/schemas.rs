//! `web3_dapp` RPC controllers: prepare a generic EVM contract call from
//! caller-supplied calldata and execute it. EVM-only.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, Outcome, TypeSchema};

use super::super::{execute_inputs, json_result, opt_str, req_json};
use crate::config::rpc as config_rpc;
use crate::modules::web3_service;
use tinywallet_bus::names::methods;

pub fn schemas() -> Vec<ControllerSchema> {
    vec![schema("call"), schema("execute")]
}

pub fn controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("call"),
            handler: handle_call,
        },
        RegisteredController {
            schema: schema("execute"),
            handler: handle_execute,
        },
    ]
}

pub fn schema(function: &str) -> ControllerSchema {
    match function {
        "call" => ControllerSchema {
            namespace: "web3_dapp",
            function: "call",
            description: "Prepare a generic EVM dapp contract call from pre-encoded calldata. Returns a prepared quoteId to confirm+execute.",
            inputs: vec![
                req_json("contractAddress", "Target contract address."),
                req_json("calldata", "0x-prefixed hex calldata."),
                opt_str(
                    "valueRaw",
                    "Native value attached, smallest unit. Defaults to '0'.",
                ),
                opt_str(
                    "evmNetwork",
                    "EVM network selector. Defaults to ethereum_mainnet.",
                ),
            ],
            outputs: vec![json_result(
                "Prepared web3 quote {quoteId, kind, expiresAtMs, quote}.",
            )],
        },
        "execute" => ControllerSchema {
            namespace: "web3_dapp",
            function: "execute",
            description: "Confirm and execute a prepared web3_dapp call: signs the contract call in-core and broadcasts it.",
            inputs: execute_inputs(),
            outputs: vec![json_result(
                "ExecutionResult {quoteId, kind, transactionHash, explorerUrl?, feeRaw}.",
            )],
        },
        _ => ControllerSchema {
            namespace: "web3_dapp",
            function: "unknown",
            description: "Unknown web3_dapp controller.",
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

fn handle_call(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let quote = web3_service::quote(
            &config,
            None,
            methods::WEB3_DAPP_QUOTE,
            Value::Object(params),
        )
        .await
        .map_err(|error| error.to_string())?;
        Outcome::new(quote, vec!["web3 dapp call prepared".to_string()]).into_cli_compatible_json()
    })
}

fn handle_execute(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: web3_service::ExecuteParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let config = config_rpc::load_config_with_timeout().await?;
        let result = web3_service::execute(&config, None, parsed.quote_id, parsed.confirmed)
            .await
            .map_err(|error| error.to_string())?;
        Outcome::new(result, vec!["web3 transaction broadcast".to_string()])
            .into_cli_compatible_json()
    })
}
