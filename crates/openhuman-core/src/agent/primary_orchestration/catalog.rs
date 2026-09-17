//! Exact-name typed capability catalog over registered OpenHuman tools.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::agent::primary_orchestration::capability::{
    CapabilityAvailability, CapabilityBackend, CapabilityModality, CapabilityOperation,
    CapabilitySideEffect, MonetaryBoundary, ToolCapability, ToolRoute,
};
use crate::tools::traits::{PermissionLevel, Tool};

/// Route classification specifying backend and monetary boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogRouteClass {
    pub backend: CapabilityBackend,
    pub monetary_boundary: MonetaryBoundary,
}

/// Dynamic inputs for capability catalog construction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityCatalogInputs {
    pub canonical_search: Option<CatalogRouteClass>,
    pub availability: BTreeMap<String, CapabilityAvailability>,
}

/// Typed capability catalog containing ordered routes and diagnostic warnings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityCatalog {
    pub routes: Vec<ToolRoute>,
    pub diagnostics: Vec<String>,
}

/// Catalog construction errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityCatalogError {
    DuplicateName(String),
}

impl std::fmt::Display for CapabilityCatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateName(name) => write!(f, "duplicate tool name '{name}'"),
        }
    }
}

impl std::error::Error for CapabilityCatalogError {}

struct ToolClassification {
    operations: Vec<CapabilityOperation>,
    modalities: Vec<CapabilityModality>,
    backend: CapabilityBackend,
    monetary_boundary: MonetaryBoundary,
}

fn classify_tool(
    name: &str,
    canonical_search: Option<CatalogRouteClass>,
) -> Option<ToolClassification> {
    match name {
        // Local workspace read tools
        "file_read" | "grep" | "glob" | "list" | "read_diff" => Some(ToolClassification {
            operations: vec![CapabilityOperation::ReadWorkspace],
            modalities: vec![CapabilityModality::File],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        // Local workspace write tools
        "file_write" | "edit_file" | "apply_patch" | "csv_export" => Some(ToolClassification {
            operations: vec![CapabilityOperation::WriteWorkspace],
            modalities: vec![CapabilityModality::File],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        // Local system execution tools
        "shell" | "run_tests" | "run_linter" | "node_exec" | "npm_exec" | "python_exec"
        | "git_operations" => Some(ToolClassification {
            operations: vec![CapabilityOperation::ExecuteCommand],
            modalities: vec![CapabilityModality::Code],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        // Browser tools
        "browser" | "browser_open" => Some(ToolClassification {
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::LocalBrowser,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        // Free network tools
        "http_request" | "web_fetch" | "curl" => Some(ToolClassification {
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::DirectNetwork,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        // Canonical search tool
        "web_search_tool" => canonical_search.map(|class| ToolClassification {
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Text],
            backend: class.backend,
            monetary_boundary: class.monetary_boundary,
        }),
        // BYOK search: image search
        "brave_image_search" => Some(ToolClassification {
            operations: vec![CapabilityOperation::RetrieveImage],
            modalities: vec![CapabilityModality::Image],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
        }),
        // BYOK search: video search
        "brave_video_search" => Some(ToolClassification {
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Video],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
        }),
        // BYOK search: URL contents / extraction
        "exa_get_contents" | "tavily_extract" => Some(ToolClassification {
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
        }),
        // BYOK search: text web search
        "brave_news_search" | "querit_search" | "exa_search" | "exa_find_similar"
        | "tavily_search" => Some(ToolClassification {
            operations: vec![CapabilityOperation::SearchWeb],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::Byok,
            monetary_boundary: MonetaryBoundary::UserSuppliedKey,
        }),
        // Managed search: extract / fetch
        "parallel_extract" | "tinyfish_fetch" => Some(ToolClassification {
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
        }),
        // Managed search: text web search
        "parallel_search" | "parallel_enrich" | "parallel_research" | "parallel_chat"
        | "parallel_dataset" | "tinyfish_search" | "tinyfish_agent_run" => {
            Some(ToolClassification {
                operations: vec![CapabilityOperation::SearchWeb],
                modalities: vec![CapabilityModality::Text],
                backend: CapabilityBackend::Managed,
                monetary_boundary: MonetaryBoundary::ManagedMetered,
            })
        }
        // Managed media: image generation
        "media_generate_image" => Some(ToolClassification {
            operations: vec![CapabilityOperation::GenerateImage],
            modalities: vec![CapabilityModality::Image],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
        }),
        // Managed media: video generation
        "media_generate_video" => Some(ToolClassification {
            operations: vec![CapabilityOperation::GenerateImage],
            modalities: vec![CapabilityModality::Video],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
        }),
        // Managed media: model listing
        "media_list_models" => Some(ToolClassification {
            operations: vec![CapabilityOperation::GenerateImage],
            modalities: vec![CapabilityModality::Text],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
        }),
        // Managed integrations
        "google_places_search"
        | "google_places_details"
        | "stock_quote"
        | "stock_exchange_rate"
        | "stock_options"
        | "stock_crypto_series"
        | "stock_commodity"
        | "twilio_call"
        | "composio_list_toolkits"
        | "composio_list_connections"
        | "composio_authorize"
        | "composio_list_tools"
        | "composio_execute" => Some(ToolClassification {
            operations: vec![CapabilityOperation::Integration],
            modalities: vec![CapabilityModality::Integration],
            backend: CapabilityBackend::Managed,
            monetary_boundary: MonetaryBoundary::ManagedMetered,
        }),
        // Memory: recall
        "memory_recall" => Some(ToolClassification {
            operations: vec![CapabilityOperation::RecallMemory],
            modalities: vec![CapabilityModality::Memory],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        // Memory: store / forget
        "memory_store" | "memory_forget" => Some(ToolClassification {
            operations: vec![CapabilityOperation::StoreMemory],
            modalities: vec![CapabilityModality::Memory],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        // Scheduling
        "schedule" | "cron_add" | "cron_list" | "cron_remove" | "cron_update" | "cron_run"
        | "cron_runs" => Some(ToolClassification {
            operations: vec![CapabilityOperation::Schedule],
            modalities: vec![CapabilityModality::Schedule],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
        }),
        // Delegation
        "spawn_subagent" | "spawn_async_subagent" | "spawn_parallel_agents" | "delegate" => {
            Some(ToolClassification {
                operations: vec![CapabilityOperation::Delegate],
                modalities: vec![CapabilityModality::Text],
                backend: CapabilityBackend::Local,
                monetary_boundary: MonetaryBoundary::NonMetered,
            })
        }
        _ => None,
    }
}

fn derive_side_effect(
    backend: CapabilityBackend,
    permission: PermissionLevel,
) -> CapabilitySideEffect {
    match backend {
        CapabilityBackend::Local => match permission {
            PermissionLevel::None => CapabilitySideEffect::None,
            p if p <= PermissionLevel::ReadOnly => CapabilitySideEffect::LocalRead,
            _ => CapabilitySideEffect::LocalWrite,
        },
        CapabilityBackend::LocalBrowser
        | CapabilityBackend::DirectNetwork
        | CapabilityBackend::Byok
        | CapabilityBackend::Managed => match permission {
            p if p <= PermissionLevel::ReadOnly => CapabilitySideEffect::ExternalRead,
            _ => CapabilitySideEffect::ExternalWrite,
        },
    }
}

/// Build the exact-name typed capability catalog over the already-registered tools.
pub fn build_capability_catalog(
    tools: &[Box<dyn Tool>],
    inputs: &CapabilityCatalogInputs,
) -> Result<CapabilityCatalog, CapabilityCatalogError> {
    let mut seen_names = HashSet::with_capacity(tools.len());
    let mut routes = Vec::with_capacity(tools.len());
    let mut diagnostics = Vec::new();

    for (index, tool) in tools.iter().enumerate() {
        let name = tool.name().to_string();
        if !seen_names.insert(name.clone()) {
            return Err(CapabilityCatalogError::DuplicateName(name));
        }

        let (operations, modalities, backend, monetary_boundary, is_diagnostic) =
            match classify_tool(name.as_str(), inputs.canonical_search) {
                Some(classified) => (
                    classified.operations,
                    classified.modalities,
                    classified.backend,
                    classified.monetary_boundary,
                    false,
                ),
                None => {
                    diagnostics.push(name.clone());
                    (
                        vec![CapabilityOperation::Integration],
                        vec![CapabilityModality::Integration],
                        CapabilityBackend::Local,
                        MonetaryBoundary::NonMetered,
                        true,
                    )
                }
            };

        let availability = if is_diagnostic {
            CapabilityAvailability::Disabled
        } else {
            inputs
                .availability
                .get(&name)
                .copied()
                .unwrap_or(CapabilityAvailability::Available)
        };

        let side_effect = derive_side_effect(backend, tool.permission_level());

        let capability = ToolCapability {
            name,
            operations,
            modalities,
            backend,
            monetary_boundary,
            side_effect,
            availability,
            permission: tool.permission_level(),
            priority: 0,
            registration_index: index,
        };

        routes.push(ToolRoute::new(capability));
    }

    Ok(CapabilityCatalog {
        routes,
        diagnostics,
    })
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod catalog_tests;
