//! `web3_swap` RPC controllers: prepare a single-chain swap quote, execute a
//! prepared quote, and list supported routes. Cross-chain swaps are rejected
//! at the ops layer with a pointer to `web3_bridge`.

use crate::core::runtime::DispatchCtx;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, Outcome, TypeSchema};

use super::super::{execute_inputs, json_result, opt_str, req_json};
use crate::modules::web3_service;
use tinywallet_bus::names::methods;

pub fn schemas() -> Vec<ControllerSchema> {
    vec![schema("quote"), schema("execute"), schema("routes")]
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
        RegisteredController {
            schema: schema("routes"),
            handler: handle_routes,
        },
    ]
}

pub fn schema(function: &str) -> ControllerSchema {
    match function {
        "quote" => ControllerSchema {
            namespace: "web3_swap",
            function: "quote",
            description: "Prepare a single-chain token swap via deBridge. Returns a quote plus a prepared quoteId to confirm+execute. For cross-chain swaps use web3_bridge.",
            inputs: vec![
                req_json(
                    "chainId",
                    "deBridge chain id the swap executes on (e.g. 1 ETH, 56 BNB, 7565164 Solana).",
                ),
                req_json(
                    "tokenIn",
                    "Input token address (use the zero address for native).",
                ),
                req_json(
                    "tokenInAmount",
                    "Input amount in the token's smallest unit.",
                ),
                req_json("tokenOut", "Output token address."),
                opt_str(
                    "tokenOutRecipient",
                    "Address receiving output. Defaults to the wallet's own address on this chain.",
                ),
                opt_str(
                    "senderAddress",
                    "Sender. Defaults to the wallet's own address on this chain.",
                ),
                opt_str("slippage", "Slippage percent or 'auto' (default 'auto')."),
            ],
            outputs: vec![json_result(
                "Prepared web3 quote {quoteId, kind, expiresAtMs, quote}.",
            )],
        },
        "execute" => ControllerSchema {
            namespace: "web3_swap",
            function: "execute",
            description: "Confirm and execute a prepared web3_swap quote: signs the unsigned transaction in-core and broadcasts it.",
            inputs: execute_inputs(),
            outputs: vec![json_result(
                "ExecutionResult {quoteId, kind, transactionHash, explorerUrl?, feeRaw}.",
            )],
        },
        "routes" => ControllerSchema {
            namespace: "web3_swap",
            function: "routes",
            description: "List the chains deBridge can swap/bridge between.",
            inputs: vec![],
            outputs: vec![json_result("deBridge supported-chains payload.")],
        },
        _ => unknown_schema(),
    }
}

fn unknown_schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "web3_swap",
        function: "unknown",
        description: "Unknown web3_swap controller.",
        inputs: vec![],
        outputs: vec![FieldSchema {
            name: "error",
            ty: TypeSchema::String,
            comment: "Lookup error details.",
            required: true,
        }],
    }
}

fn handle_quote(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let quote = web3_service::quote(
            &ctx,
            &config,
            None,
            methods::WEB3_SWAP_QUOTE,
            Value::Object(params),
        )
        .await
        .map_err(|error| error.to_string())?;
        Outcome::new(quote, vec!["web3 swap prepared".to_string()]).into_cli_compatible_json()
    })
}

fn handle_execute(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let parsed: web3_service::ExecuteParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let config = ctx.load_config().await?;
        let result = web3_service::execute(&ctx, &config, None, parsed.quote_id, parsed.confirmed)
            .await
            .map_err(|error| error.to_string())?;
        Outcome::new(result, vec!["web3 transaction broadcast".to_string()])
            .into_cli_compatible_json()
    })
}

fn handle_routes(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let routes = web3_service::routes(&ctx, &config)
            .await
            .map_err(|error| error.to_string())?;
        Outcome::new(routes, vec!["web3 supported routes listed".to_string()])
            .into_cli_compatible_json()
    })
}
