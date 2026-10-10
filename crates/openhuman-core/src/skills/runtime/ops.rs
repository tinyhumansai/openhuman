//! Skill runtime availability probes and skill execution orchestration.

use serde::Serialize;

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
    _config: &crate::config::Config,
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
        runtimes.push(resolve_host_runtime("node", "node"));
    }
    if matches!(
        requirement,
        RuntimeRequirement::All | RuntimeRequirement::Python
    ) {
        runtimes.push(resolve_host_runtime("python", "python3"));
    }
    tracing::debug!(
        count = runtimes.len(),
        "[skill_runtime] resolve_runtimes: done"
    );
    ResolveRuntimesOutcome { runtimes }
}

fn resolve_host_runtime(runtime: &str, command: &str) -> ResolvedRuntimeSummary {
    let output = std::process::Command::new(command)
        .arg("--version")
        .output();
    let binary = find_host_binary(command);
    let (version, error) = match output {
        Ok(output) if output.status.success() => (
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string()),
            None,
        ),
        Ok(output) => (
            None,
            Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        ),
        Err(error) => (None, Some(error.to_string())),
    };
    ResolvedRuntimeSummary {
        runtime: runtime.to_string(),
        enabled: true,
        available: version.is_some(),
        source: version.as_ref().map(|_| "host".to_string()),
        version,
        binary,
        bin_dir: None,
        error,
    }
}

fn find_host_binary(command: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    let extensions = if cfg!(windows) {
        std::env::var_os("PATHEXT")
            .map(|extensions| {
                std::env::split_paths(&extensions)
                    .map(|extension| extension.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
            .filter(|extensions| !extensions.is_empty())
            .unwrap_or_else(|| vec![".COM".into(), ".EXE".into(), ".BAT".into(), ".CMD".into()])
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&path)
        .flat_map(|directory| {
            extensions
                .iter()
                .map(move |extension| directory.join(format!("{command}{extension}")))
        })
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.display().to_string())
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
