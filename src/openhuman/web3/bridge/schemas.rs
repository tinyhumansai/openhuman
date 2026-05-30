//! `web3_bridge` RPC controllers: prepare a cross-chain bridge quote and
//! execute it. Same-chain requests are rejected at the ops layer.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::super::store::execute_quote;
use super::super::types::{BridgeQuoteParams, ExecuteQuoteParams};
use super::super::{execute_inputs, json_result, opt_str, req_json};

pub fn schemas() -> Vec<ControllerSchema> {
    vec![schema("quote"), schema("execute")]
}

pub fn controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("quote"),
            handler: handle_quote,
        },
        RegisteredController {
            schema: schema("execute"),
            handler: handle_execute,
        },
    ]
}

pub fn schema(function: &str) -> ControllerSchema {
    match function {
        "quote" => ControllerSchema {
            namespace: "web3_bridge",
            function: "quote",
            description:
                "Prepare a cross-chain bridge via deBridge DLN. Returns a quote plus a prepared quoteId to confirm+execute; the unsigned transaction is signed+broadcast on the source chain.",
            inputs: vec![
                req_json("srcChainId", "Source deBridge chain id."),
                req_json("srcChainTokenIn", "Source token address."),
                req_json("srcChainTokenInAmount", "Source amount in the token's smallest unit."),
                req_json("dstChainId", "Destination deBridge chain id (must differ from source)."),
                req_json("dstChainTokenOut", "Destination token address."),
                opt_str("dstChainTokenOutAmount", "Destination amount, or 'auto' for market rate (default 'auto')."),
                opt_str("dstChainTokenOutRecipient", "Recipient on the destination chain. Defaults to the wallet's own address."),
                opt_str("srcChainOrderAuthorityAddress", "Source-chain order authority. Defaults to the wallet's source address."),
                opt_str("dstChainOrderAuthorityAddress", "Destination-chain order authority. Defaults to the wallet's destination address."),
            ],
            outputs: vec![json_result("Prepared web3 quote {quoteId, kind, expiresAtMs, quote}.")],
        },
        "execute" => ControllerSchema {
            namespace: "web3_bridge",
            function: "execute",
            description:
                "Confirm and execute a prepared web3_bridge quote: signs the source-chain transaction in-core and broadcasts it.",
            inputs: execute_inputs(),
            outputs: vec![json_result("ExecutionResult {quoteId, kind, transactionHash, explorerUrl?, feeRaw}.")],
        },
        _ => ControllerSchema {
            namespace: "web3_bridge",
            function: "unknown",
            description: "Unknown web3_bridge controller.",
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

fn handle_quote(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: BridgeQuoteParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        super::super::ops::quote_bridge(parsed)
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
