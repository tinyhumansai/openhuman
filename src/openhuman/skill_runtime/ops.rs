//! Skill runtime operations that coordinate reusable language runtimes.

use serde::Serialize;

use crate::openhuman::config::Config;
use crate::openhuman::runtime_node::{NodeBootstrap, NodeSource};
use crate::openhuman::runtime_python::{PythonBootstrap, PythonSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRequirement {
    All,
    Node,
    Python,
}

impl RuntimeRequirement {
    pub fn from_optional(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("all").trim().to_ascii_lowercase().as_str() {
            "" | "all" => Ok(Self::All),
            "node" | "nodejs" | "javascript" => Ok(Self::Node),
            "python" | "python3" => Ok(Self::Python),
            other => Err(format!(
                "unknown runtime '{other}' (expected all, node, or python)"
            )),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ResolvedRuntimeSummary {
    pub runtime: String,
    pub enabled: bool,
    pub available: bool,
    pub source: Option<String>,
    pub version: Option<String>,
    pub binary: Option<String>,
    pub bin_dir: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResolveRuntimesOutcome {
    pub runtimes: Vec<ResolvedRuntimeSummary>,
}

pub async fn resolve_runtimes(
    config: &Config,
    requirement: RuntimeRequirement,
) -> ResolveRuntimesOutcome {
    tracing::debug!(
        requirement = ?requirement,
        "[skill_runtime] resolve_runtimes: start"
    );
    let mut runtimes = Vec::new();
    if matches!(
        requirement,
        RuntimeRequirement::All | RuntimeRequirement::Node
    ) {
        runtimes.push(resolve_node(config).await);
    }
    if matches!(
        requirement,
        RuntimeRequirement::All | RuntimeRequirement::Python
    ) {
        runtimes.push(resolve_python(config).await);
    }
    tracing::debug!(
        count = runtimes.len(),
        "[skill_runtime] resolve_runtimes: done"
    );
    ResolveRuntimesOutcome { runtimes }
}

async fn resolve_node(config: &Config) -> ResolvedRuntimeSummary {
    if !config.node.enabled {
        return ResolvedRuntimeSummary {
            runtime: "node".to_string(),
            enabled: false,
            available: false,
            source: None,
            version: None,
            binary: None,
            bin_dir: None,
            error: Some("node runtime disabled".to_string()),
        };
    }
    let bootstrap = NodeBootstrap::new(
        config.node.clone(),
        config.workspace_dir.clone(),
        reqwest::Client::new(),
    );
    match bootstrap.resolve().await {
        Ok(resolved) => ResolvedRuntimeSummary {
            runtime: "node".to_string(),
            enabled: true,
            available: true,
            source: Some(
                match resolved.source {
                    NodeSource::System => "system",
                    NodeSource::Managed => "managed",
                }
                .to_string(),
            ),
            version: Some(resolved.version),
            binary: Some(resolved.node_bin.display().to_string()),
            bin_dir: Some(resolved.bin_dir.display().to_string()),
            error: None,
        },
        Err(error) => ResolvedRuntimeSummary {
            runtime: "node".to_string(),
            enabled: true,
            available: false,
            source: None,
            version: None,
            binary: None,
            bin_dir: None,
            error: Some(error.to_string()),
        },
    }
}

async fn resolve_python(config: &Config) -> ResolvedRuntimeSummary {
    if !config.runtime_python.enabled {
        return ResolvedRuntimeSummary {
            runtime: "python".to_string(),
            enabled: false,
            available: false,
            source: None,
            version: None,
            binary: None,
            bin_dir: None,
            error: Some("python runtime disabled".to_string()),
        };
    }
    let bootstrap = PythonBootstrap::new(config.runtime_python.clone());
    match bootstrap.resolve().await {
        Ok(resolved) => ResolvedRuntimeSummary {
            runtime: "python".to_string(),
            enabled: true,
            available: true,
            source: Some(
                match resolved.source {
                    PythonSource::System => "system",
                    PythonSource::Managed => "managed",
                }
                .to_string(),
            ),
            version: Some(resolved.version),
            binary: Some(resolved.python_bin.display().to_string()),
            bin_dir: resolved
                .python_bin
                .parent()
                .map(|path| path.display().to_string()),
            error: None,
        },
        Err(error) => ResolvedRuntimeSummary {
            runtime: "python".to_string(),
            enabled: true,
            available: false,
            source: None,
            version: None,
            binary: None,
            bin_dir: None,
            error: Some(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_requirement_parses_aliases() {
        assert_eq!(
            RuntimeRequirement::from_optional(None).unwrap(),
            RuntimeRequirement::All
        );
        assert_eq!(
            RuntimeRequirement::from_optional(Some("nodejs")).unwrap(),
            RuntimeRequirement::Node
        );
        assert_eq!(
            RuntimeRequirement::from_optional(Some("python3")).unwrap(),
            RuntimeRequirement::Python
        );
        assert!(RuntimeRequirement::from_optional(Some("ruby")).is_err());
    }
}
