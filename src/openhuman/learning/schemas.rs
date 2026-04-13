//! Controller schemas for the learning domain.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_learning_controller_schemas() -> Vec<ControllerSchema> {
    vec![learning_schemas("learning_linkedin_enrichment")]
}

pub fn all_learning_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: learning_schemas("learning_linkedin_enrichment"),
        handler: handle_linkedin_enrichment,
    }]
}

pub fn learning_schemas(function: &str) -> ControllerSchema {
    match function {
        "learning_linkedin_enrichment" => ControllerSchema {
            namespace: "learning",
            function: "linkedin_enrichment",
            description: "Search Gmail for LinkedIn profile URLs, scrape the profile via Apify, \
                          and persist the result to memory. Runs the full enrichment pipeline.",
            inputs: vec![],
            outputs: vec![
                FieldSchema {
                    name: "profile_url",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "LinkedIn profile URL found in Gmail, if any.",
                    required: false,
                },
                FieldSchema {
                    name: "profile_data",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Scraped LinkedIn profile JSON from Apify, if successful.",
                    required: false,
                },
                FieldSchema {
                    name: "log",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Human-readable log of each pipeline stage.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "learning",
            function: "unknown",
            description: "Unknown learning controller.",
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

fn handle_linkedin_enrichment(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let result = super::linkedin_enrichment::run_linkedin_enrichment(&config)
            .await
            .map_err(|e| format!("linkedin enrichment failed: {e:#}"))?;

        let payload = serde_json::json!({
            "profile_url": result.profile_url,
            "profile_data": result.profile_data,
            "stages": result.stages,
            "log": result.log,
        });

        RpcOutcome::new(payload, result.log.clone()).into_cli_compatible_json()
    })
}
