//! Service management helpers for Alphahuman daemon.

use crate::alphahuman::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SERVICE_LABEL: &str = "com.alphahuman.daemon";
const LEGACY_SERVICE_LABEL: &str = "com.alphahuman.app";
const WINDOWS_TASK_NAME: &str = "AlphaHuman Daemon";

fn windows_task_name() -> &'static str {
    WINDOWS_TASK_NAME
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceState {
    Running,
    Stopped,
    NotInstalled,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub state: ServiceState,
    pub unit_path: Option<PathBuf>,
    pub label: String,
    pub details: Option<String>,
}

pub fn install(config: &Config) -> Result<ServiceStatus> {
    if cfg!(target_os = "macos") {
        install_macos(config)?;
        status(config)
    } else if cfg!(target_os = "linux") {
        install_linux(config)?;
        status(config)
    } else if cfg!(target_os = "windows") {
        install_windows(config)?;
        status(config)
    } else {
        anyhow::bail!("Service management is supported on macOS, Linux, and Windows only");
    }
}

pub fn start(config: &Config) -> Result<ServiceStatus> {
    if cfg!(target_os = "macos") {
        let plist = macos_service_file()?;
        run_checked(Command::new("launchctl").arg("load").arg("-w").arg(&plist))?;
        run_checked(Command::new("launchctl").arg("start").arg(SERVICE_LABEL))?;
        return status(config);
    }

    if cfg!(target_os = "linux") {
        run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run_checked(Command::new("systemctl").args(["--user", "start", "alphahuman.service"]))?;
        return status(config);
    }

    if cfg!(target_os = "windows") {
        let _ = config;
        run_checked(Command::new("schtasks").args(["/Run", "/TN", windows_task_name()]))?;
        return status(config);
    }

    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn stop(config: &Config) -> Result<ServiceStatus> {
    if cfg!(target_os = "macos") {
        let plist = macos_service_file()?;
        let _ = run_checked(Command::new("launchctl").arg("stop").arg(SERVICE_LABEL));
        let _ = run_checked(Command::new("launchctl").arg("unload").arg("-w").arg(&plist));

        let legacy_plist = macos_service_file_for(LEGACY_SERVICE_LABEL)?;
        let _ = run_checked(Command::new("launchctl").arg("stop").arg(LEGACY_SERVICE_LABEL));
        let _ = run_checked(Command::new("launchctl").arg("unload").arg("-w").arg(&legacy_plist));
        return status(config);
    }

    if cfg!(target_os = "linux") {
        let _ = run_checked(Command::new("systemctl").args(["--user", "stop", "alphahuman.service"]));
        return status(config);
    }

    if cfg!(target_os = "windows") {
        let _ = config;
        let task_name = windows_task_name();
        let _ = run_checked(Command::new("schtasks").args(["/End", "/TN", task_name]));
        return status(config);
    }

    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn status(config: &Config) -> Result<ServiceStatus> {
    if cfg!(target_os = "macos") {
        let out = run_capture(Command::new("launchctl").arg("list"))?;
        let running = out.lines().any(|line| {
            line.contains(SERVICE_LABEL) || line.contains(LEGACY_SERVICE_LABEL)
        });
        return Ok(ServiceStatus {
            state: if running {
                ServiceState::Running
            } else {
                ServiceState::Stopped
            },
            unit_path: Some(macos_service_file()?),
            label: SERVICE_LABEL.to_string(),
            details: None,
        });
    }

    if cfg!(target_os = "linux") {
        let out = run_capture(Command::new("systemctl").args([
            "--user",
            "is-active",
            "alphahuman.service",
        ]))
        .unwrap_or_else(|_| "unknown".into());
        let state = match out.trim() {
            "active" => ServiceState::Running,
            "inactive" | "failed" => ServiceState::Stopped,
            other => ServiceState::Unknown(other.to_string()),
        };
        return Ok(ServiceStatus {
            state,
            unit_path: Some(linux_service_file(config)?),
            label: "alphahuman.service".to_string(),
            details: None,
        });
    }

    if cfg!(target_os = "windows") {
        let _ = config;
        let task_name = windows_task_name();
        let out = run_capture(Command::new("schtasks").args(["/Query", "/TN", task_name, "/FO", "LIST"]));
        match out {
            Ok(text) => {
                let running = text.contains("Running");
                return Ok(ServiceStatus {
                    state: if running {
                        ServiceState::Running
                    } else {
                        ServiceState::Stopped
                    },
                    unit_path: None,
                    label: task_name.to_string(),
                    details: None,
                });
            }
            Err(err) => {
                return Ok(ServiceStatus {
                    state: ServiceState::NotInstalled,
                    unit_path: None,
                    label: task_name.to_string(),
                    details: Some(err.to_string()),
                });
            }
        }
    }

    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn uninstall(config: &Config) -> Result<ServiceStatus> {
    let _ = stop(config);

    if cfg!(target_os = "macos") {
        let file = macos_service_file()?;
        if file.exists() {
            fs::remove_file(&file)
                .with_context(|| format!("Failed to remove {}", file.display()))?;
        }
        let legacy_file = macos_service_file_for(LEGACY_SERVICE_LABEL)?;
        if legacy_file.exists() {
            let _ = fs::remove_file(&legacy_file);
        }
        return Ok(ServiceStatus {
            state: ServiceState::NotInstalled,
            unit_path: Some(file),
            label: SERVICE_LABEL.to_string(),
            details: None,
        });
    }

    if cfg!(target_os = "linux") {
        let file = linux_service_file(config)?;
        if file.exists() {
            fs::remove_file(&file)
                .with_context(|| format!("Failed to remove {}", file.display()))?;
        }
        let _ = run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]));
        return Ok(ServiceStatus {
            state: ServiceState::NotInstalled,
            unit_path: Some(file),
            label: "alphahuman.service".to_string(),
            details: None,
        });
    }

    if cfg!(target_os = "windows") {
        let task_name = windows_task_name();
        let _ = run_checked(Command::new("schtasks").args(["/Delete", "/TN", task_name, "/F"]));
        // Remove the wrapper script
        let wrapper = config
            .config_path
            .parent()
            .map_or_else(|| PathBuf::from("."), PathBuf::from)
            .join("logs")
            .join("alphahuman-daemon.cmd");
        if wrapper.exists() {
            fs::remove_file(&wrapper).ok();
        }
        return Ok(ServiceStatus {
            state: ServiceState::NotInstalled,
            unit_path: None,
            label: task_name.to_string(),
            details: None,
        });
    }

    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

fn install_macos(config: &Config) -> Result<()> {
    let file = macos_service_file()?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let logs_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("logs");
    fs::create_dir_all(&logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");

    let plist = format!(
        r#"<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        exe = xml_escape(&exe.display().to_string()),
        stdout = xml_escape(&stdout.display().to_string()),
        stderr = xml_escape(&stderr.display().to_string())
    );

    fs::write(&file, plist)?;
    Ok(())
}

fn install_linux(config: &Config) -> Result<()> {
    let file = linux_service_file(config)?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let logs_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("logs");
    fs::create_dir_all(&logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");

    let unit = format!(
        "[Unit]\nDescription=Alphahuman Daemon\n\n[Service]\nExecStart={} daemon\nRestart=always\nRestartSec=3\n\nStandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
        exe.display(),
        stdout.display(),
        stderr.display(),
    );

    fs::write(&file, unit)?;
    let _ = run_checked(Command::new("systemctl").args(["--user", "enable", "alphahuman.service"]));
    Ok(())
}

fn install_windows(config: &Config) -> Result<()> {
    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let logs_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("logs");
    fs::create_dir_all(&logs_dir)?;

    let wrapper = logs_dir.join("alphahuman-daemon.cmd");
    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");

    let cmd = format!(
        "@echo off\n\"{}\" daemon >> \"{}\" 2>> \"{}\"\n",
        exe.display(),
        stdout.display(),
        stderr.display()
    );
    fs::write(&wrapper, cmd)?;

    run_checked(Command::new("schtasks").args([
        "/Create",
        "/TN",
        windows_task_name(),
        "/TR",
        &wrapper.display().to_string(),
        "/SC",
        "ONLOGON",
        "/F",
    ]))?;

    Ok(())
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn macos_service_file() -> Result<PathBuf> {
    macos_service_file_for(SERVICE_LABEL)
}

fn macos_service_file_for(label: &str) -> Result<PathBuf> {
    let home = std::env::var("HOME").context("$HOME is not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{label}.plist")))
}

fn linux_service_file(config: &Config) -> Result<PathBuf> {
    let config_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    Ok(config_dir
        .join(".config")
        .join("systemd")
        .join("user")
        .join("alphahuman.service"))
}

fn run_checked(cmd: &mut Command) -> Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("command failed with status {status}");
    }
    Ok(())
}

fn run_capture(cmd: &mut Command) -> Result<String> {
    let output = cmd.output()?;
    if !output.status.success() {
        anyhow::bail!("command failed with status {}", output.status);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_replaces_entities() {
        let raw = "<tag>\"&'";
        let escaped = xml_escape(raw);
        assert!(escaped.contains("&lt;tag&gt;"));
        assert!(escaped.contains("&quot;"));
        assert!(escaped.contains("&amp;"));
        assert!(escaped.contains("&apos;"));
    }

    #[test]
    fn linux_service_file_uses_config_dir() {
        let config = Config::default();
        let path = linux_service_file(&config).unwrap();
        assert!(path.ends_with(".config/systemd/user/alphahuman.service"));
    }
}
