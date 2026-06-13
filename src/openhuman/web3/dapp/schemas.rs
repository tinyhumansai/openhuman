//! `web3_dapp` RPC controllers: prepare a generic EVM contract call from
//! caller-supplied calldata and execute it. EVM-only.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::store::execute_quote;
use super::super::types::{DappCallParams, ExecuteQuoteParams};
use super::super::{execute_inputs, json_result, opt_str, req_json};

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
            description:
                "Prepare a generic EVM dapp contract call from pre-encoded calldata. Returns a prepared quoteId to confirm+execute.",
            inputs: vec![
                req_json("contractAddress", "Target contract address."),
                req_json("calldata", "0x-prefixed hex calldata."),
                opt_str("valueRaw", "Native value attached, smallest unit. Defaults to '0'."),
                opt_str("evmNetwork", "EVM network selector. Defaults to ethereum_mainnet."),
            ],
            outputs: vec![json_result("Prepared web3 quote {quoteId, kind, expiresAtMs, quote}.")],
        },
        "execute" => ControllerSchema {
            namespace: "web3_dapp",
            function: "execute",
            description:
                "Confirm and execute a prepared web3_dapp call: signs the contract call in-core and broadcasts it.",
            inputs: execute_inputs(),
            outputs: vec![json_result("ExecutionResult {quoteId, kind, transactionHash, explorerUrl?, feeRaw}.")],
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
        let parsed: DappCallParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        super::super::ops::prepare_dapp_call(parsed)
            .await?
            .into_cli_compatible_json()
    })
}

fn handle_execute(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: ExecuteQuoteParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        execute_quote(parsed).await?.into_cli_compatible_json()
    })
}
