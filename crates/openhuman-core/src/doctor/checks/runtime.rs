//! Static runtime prerequisites and the bounded Claude Agent SDK version probe.

use std::ffi::OsStr;
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;

use crate::doctor::{DoctorCheck, DoctorCtx, DoctorTier, Finding, FindingSeverity, ValueSource};

const ENVIRONMENT_ID: &str = "runtime/environment";
const AGENT_SDK_ID: &str = "runtime/agent-sdk";

pub(super) fn checks() -> Vec<Arc<dyn DoctorCheck>> {
    vec![Arc::new(EnvironmentCheck), Arc::new(AgentSdkCheck)]
}

struct EnvironmentCheck;

#[async_trait]
impl DoctorCheck for EnvironmentCheck {
    fn id(&self) -> &str {
        ENVIRONMENT_ID
    }

    fn category(&self) -> &str {
        "runtime environment"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, _ctx: &DoctorCtx) -> Vec<Finding> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        let findings = environment_findings(
            std::env::var_os("PATH").as_deref(),
            std::env::var_os("SHELL").as_deref(),
            home.as_deref(),
            std::env::var_os("PATHEXT").as_deref(),
        );
        for finding in &findings {
            tracing::debug!(
                target: "doctor",
                check = ENVIRONMENT_ID,
                config_path = finding.config_path.as_deref().unwrap_or(""),
                severity = ?finding.severity,
                "runtime environment prerequisite checked"
            );
        }
        findings
    }
}

fn environment_findings(
    path: Option<&OsStr>,
    shell: Option<&OsStr>,
    home: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> Vec<Finding> {
    let mut findings = Vec::with_capacity(4);
    for command in ["git", "curl"] {
        let available = command_on_path(command, path, pathext);
        findings.push(environment_finding(
            if available {
                FindingSeverity::Pass
            } else {
                FindingSeverity::Warn
            },
            if available {
                format!("The `{command}` executable is available on PATH.")
            } else {
                format!("The `{command}` executable is not available on PATH.")
            },
            if available {
                "No action required."
            } else {
                "Install the named local prerequisite and make it available on PATH."
            },
            &format!("PATH/{command}"),
        ));
    }

    let shell_present = shell.is_some_and(|value| !value.is_empty());
    findings.push(environment_finding(
        if shell_present {
            FindingSeverity::Pass
        } else {
            FindingSeverity::Warn
        },
        if shell_present {
            "A shell is configured in the process environment."
        } else {
            "The SHELL environment variable is not configured."
        },
        if shell_present {
            "No action required."
        } else {
            "Configure the SHELL environment variable for commands that need a shell."
        },
        "SHELL",
    ));

    let home_present = home.is_some_and(|value| !value.is_empty());
    findings.push(environment_finding(
        if home_present {
            FindingSeverity::Pass
        } else {
            FindingSeverity::Error
        },
        if home_present {
            "A home directory is configured in the process environment."
        } else {
            "Neither HOME nor USERPROFILE is configured."
        },
        if home_present {
            "No action required."
        } else {
            "Set HOME or USERPROFILE to the current user's home directory."
        },
        "HOME",
    ));
    findings
}

fn environment_finding(
    severity: FindingSeverity,
    message: impl Into<String>,
    fix_hint: impl Into<String>,
    config_path: &str,
) -> Finding {
    let mut finding = Finding::new(ENVIRONMENT_ID, severity, message, fix_hint);
    finding.config_path = Some(config_path.to_owned());
    finding.value_source = Some(ValueSource::Env);
    finding
}

/// Checks command availability with filesystem metadata only. Static doctor
/// checks never execute tools or shell code.
fn command_on_path(command: &str, path: Option<&OsStr>, pathext: Option<&OsStr>) -> bool {
    let Some(path) = path else {
        return false;
    };

    #[cfg(windows)]
    let extensions = pathext
        .and_then(OsStr::to_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(".COM;.EXE;.BAT;.CMD")
        .split(';')
        .filter(|extension| !extension.is_empty())
        .collect::<Vec<_>>();

    #[cfg(not(windows))]
    let _ = pathext;

    std::env::split_paths(path).any(|directory| {
        #[cfg(windows)]
        {
            let direct = directory.join(command);
            direct.is_file()
                || extensions
                    .iter()
                    .any(|extension| directory.join(format!("{command}{extension}")).is_file())
        }

        #[cfg(unix)]
        {
            let executable = directory.join(command);
            std::fs::metadata(executable)
                .map(|metadata| {
                    metadata.is_file() && {
                        use std::os::unix::fs::PermissionsExt;
                        metadata.permissions().mode() & 0o111 != 0
                    }
                })
                .unwrap_or(false)
        }

        #[cfg(not(any(windows, unix)))]
        {
            directory.join(command).is_file()
        }
    })
}

struct AgentSdkCheck;

#[async_trait]
impl DoctorCheck for AgentSdkCheck {
    fn id(&self) -> &str {
        AGENT_SDK_ID
    }

    fn category(&self) -> &str {
        "Claude Agent SDK"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let sdk = &ctx.config().claude_agent_sdk;
        if !sdk.enabled {
            tracing::debug!(target: "doctor", check = AGENT_SDK_ID, outcome = "disabled");
            return vec![Finding::skipped(
                self.id(),
                "Claude Agent SDK is disabled in the selected configuration.",
                "Enable Claude Agent SDK in the selected configuration to check its CLI.",
            )];
        }
        if sdk.binary.trim().is_empty() {
            return vec![agent_sdk_warning()];
        }

        tracing::debug!(target: "doctor", check = AGENT_SDK_ID, outcome = "probe_started");
        let mut command = tokio::process::Command::new(&sdk.binary);
        command
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.as_std_mut().creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        match command.status().await {
            Ok(status) if status.success() => {
                tracing::debug!(target: "doctor", check = AGENT_SDK_ID, outcome = "available");
                vec![Finding::pass(
                    self.id(),
                    "The configured Claude Agent SDK CLI responded to `--version`.",
                )]
            }
            _ => {
                tracing::debug!(target: "doctor", check = AGENT_SDK_ID, outcome = "unavailable");
                vec![agent_sdk_warning()]
            }
        }
    }
}

fn agent_sdk_warning() -> Finding {
    let mut finding = Finding::new(
        AGENT_SDK_ID,
        FindingSeverity::Warn,
        "The configured Claude Agent SDK CLI is unavailable or did not respond successfully to `--version`.",
        "Install Claude Agent SDK or configure `claude_agent_sdk.binary`, then rerun doctor.",
    );
    finding.config_path = Some("claude_agent_sdk.binary".to_owned());
    finding
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
