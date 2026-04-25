//! Controller schemas for the learning domain.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

pub fn all_learning_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        learning_schemas("learning_linkedin_enrichment"),
        learning_schemas("learning_save_profile"),
    ]
}

pub fn all_learning_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: learning_schemas("learning_linkedin_enrichment"),
            handler: handle_linkedin_enrichment,
        },
        RegisteredController {
            schema: learning_schemas("learning_save_profile"),
            handler: handle_save_profile,
        },
    ]
}

pub fn learning_schemas(function: &str) -> ControllerSchema {
    match function {
        "learning_linkedin_enrichment" => ControllerSchema {
            namespace: "learning",
            function: "linkedin_enrichment",
            description: "Search Gmail for LinkedIn profile URLs, scrape the profile via Apify, \
                          and persist the result to memory. Runs the full enrichment pipeline.",
            inputs: vec![FieldSchema {
                name: "profile_url",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Pre-found LinkedIn profile URL (skips the Gmail-search stage). \
                          The frontend supplies this when it has already located the URL via \
                          the webview-driven `gmail_find_linkedin_profile_url` Tauri command.",
                required: false,
            }],
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
        "learning_save_profile" => ControllerSchema {
            namespace: "learning",
            function: "save_profile",
            description: "Persist a markdown profile to `{workspace_dir}/PROFILE.md`. \
                          When `summarize=true`, runs the body through the LLM compressor \
                          first (same prompt as the LinkedIn-enrichment pipeline) so callers \
                          can hand in raw scraped material and get the same end-state.",
            inputs: vec![
                FieldSchema {
                    name: "markdown",
                    ty: TypeSchema::String,
                    comment: "Markdown body to persist (or to summarize first).",
                    required: true,
                },
                FieldSchema {
                    name: "summarize",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Compress through LLM before writing (default false).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "path",
                    ty: TypeSchema::String,
                    comment: "Absolute path of the written PROFILE.md.",
                    required: true,
                },
                FieldSchema {
                    name: "bytes",
                    ty: TypeSchema::U64,
                    comment: "Bytes written.",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_schemas_returns_two() {
        assert_eq!(all_learning_controller_schemas().len(), 2);
    }

    #[test]
    fn all_controllers_returns_two() {
        assert_eq!(all_learning_registered_controllers().len(), 2);
    }

    #[test]
    fn save_profile_schema_shape() {
        let s = learning_schemas("learning_save_profile");
        assert_eq!(s.namespace, "learning");
        assert_eq!(s.function, "save_profile");
        assert!(s.inputs.iter().any(|f| f.name == "markdown" && f.required));
    }

    #[test]
    fn linkedin_enrichment_schema() {
        let s = learning_schemas("learning_linkedin_enrichment");
        assert_eq!(s.namespace, "learning");
        assert_eq!(s.function, "linkedin_enrichment");
        // Optional `profile_url` input: the frontend supplies one when it
        // has already discovered the URL via the webview-driven Gmail
        // helper, letting the pipeline skip its Composio-only stage 1.
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "profile_url");
        assert!(!s.inputs[0].required);
        assert!(!s.outputs.is_empty());
    }

    #[test]
    fn unknown_function_returns_unknown() {
        let s = learning_schemas("nonexistent");
        assert_eq!(s.function, "unknown");
    }

    #[test]
    fn schemas_and_controllers_match() {
        let s = all_learning_controller_schemas();
        let c = all_learning_registered_controllers();
        assert_eq!(s[0].function, c[0].schema.function);
    }
}

fn handle_linkedin_enrichment(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let preset_profile_url = params
            .get("profile_url")
            .and_then(Value::as_str)
            .map(str::to_string);
        let config = config_rpc::load_config_with_timeout().await?;
        let result =
            super::linkedin_enrichment::run_linkedin_enrichment(&config, preset_profile_url)
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

fn handle_save_profile(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let markdown = params
            .get("markdown")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "missing required `markdown`".to_string())?;
        let summarize = params
            .get("summarize")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let config = config_rpc::load_config_with_timeout().await?;

        let body = if summarize {
            super::linkedin_enrichment::summarise_profile_with_llm(&config, &markdown)
                .await
                .map_err(|e| format!("LLM summarisation failed: {e:#}"))?
        } else {
            markdown
        };

        let path = config.workspace_dir.join("PROFILE.md");
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create workspace dir failed: {e}"))?;
        }
        tokio::fs::write(&path, &body)
            .await
            .map_err(|e| format!("write PROFILE.md failed: {e}"))?;

        let bytes = body.len();
        let path_display = path.display().to_string();
        let payload = serde_json::json!({
            "path": path_display,
            "bytes": bytes,
        });
        let log = vec![format!(
            "learning.save_profile: wrote {bytes} bytes to {path_display} (summarize={summarize})"
        )];
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}
