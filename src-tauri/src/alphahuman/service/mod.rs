//! Service management helpers for Alphahuman daemon.

use crate::alphahuman::config::Config;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const SERVICE_LABEL: &str = "com.alphahuman.daemon";
const LEGACY_SERVICE_LABEL: &str = "com.alphahuman.app";
const WINDOWS_TASK_NAME: &str = "AlphaHuman Daemon";

fn windows_task_name() -> &'static str {
    WINDOWS_TASK_NAME
}

fn daemon_program_args(exe: &std::path::Path) -> Vec<String> {
    let file_name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if file_name.contains("alphahuman-core") {
        vec!["serve".to_string()]
    } else {
        vec!["core".to_string(), "serve".to_string()]
    }
}

fn daemon_command_line(exe: &std::path::Path) -> String {
    let args = daemon_program_args(exe);
    let exe_quoted = format!("\"{}\"", exe.display());
    if args.is_empty() {
        exe_quoted
    } else {
        format!("{} {}", exe_quoted, args.join(" "))
    }
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
        let domain = macos_gui_domain()?;
        let primary_target = macos_target(SERVICE_LABEL)?;

        // Prefer modern launchctl lifecycle commands on macOS.
        if !is_service_loaded_macos()? {
            log::info!("[service] Loading macOS LaunchAgent service");
            let bootstrap_ok = run_checked(
                Command::new("launchctl")
                    .arg("bootstrap")
                    .arg(&domain)
                    .arg(&plist),
            );
            if let Err(err) = bootstrap_ok {
                log::warn!(
                    "[service] launchctl bootstrap failed, falling back to load -w: {err}"
                );
                run_checked(Command::new("launchctl").arg("load").arg("-w").arg(&plist))?;
            }
        } else {
            log::info!("[service] LaunchAgent service already loaded, skipping load step");
        }

        // Always try to start - this is safe even if already running
        log::info!("[service] Starting macOS LaunchAgent service");
        let start_result = run_checked(
            Command::new("launchctl")
                .arg("kickstart")
                .arg("-k")
                .arg(&primary_target),
        );
        if let Err(e) = start_result {
            log::warn!("[service] launchctl kickstart failed, trying launchctl start");
            let _ = run_checked(Command::new("launchctl").arg("start").arg(SERVICE_LABEL));
            // Check if it's already running - that's not an error for us
            let status_check = status(config)?;
            if matches!(status_check.state, ServiceState::Running) {
                log::info!("[service] Service was already running - operation successful");
            } else {
                return Err(e);
            }
        }
        return status(config);
    }

    if cfg!(target_os = "linux") {
        // Check if service is enabled before trying to start
        if !is_service_enabled_linux()? {
            log::info!("[service] Enabling systemd service");
            let _ = run_checked(Command::new("systemctl").args([
                "--user",
                "enable",
                "alphahuman.service",
            ]));
        } else {
            log::info!("[service] Systemd service already enabled");
        }

        run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;

        // Try to start - systemctl start is idempotent
        log::info!("[service] Starting systemd service");
        let start_result =
            run_checked(Command::new("systemctl").args(["--user", "start", "alphahuman.service"]));
        if let Err(e) = start_result {
            // Check if it's already active - that's success for us
            let status_check = status(config)?;
            if matches!(status_check.state, ServiceState::Running) {
                log::info!("[service] Service was already running - operation successful");
            } else {
                return Err(e);
            }
        }
        return status(config);
    }

    if cfg!(target_os = "windows") {
        let task_name = windows_task_name();

        // Check if task exists before trying to run
        if !is_task_exists_windows(task_name)? {
            log::warn!("[service] Windows scheduled task does not exist, please install first");
            return Ok(ServiceStatus {
                state: ServiceState::NotInstalled,
                unit_path: None,
                label: task_name.to_string(),
                details: Some("Task not installed".to_string()),
            });
        }

        // Try to run task - this may fail if already running, which is OK
        log::info!("[service] Starting Windows scheduled task");
        let run_result = run_checked(Command::new("schtasks").args(["/Run", "/TN", task_name]));
        if let Err(e) = run_result {
            // Check if it's already running - that's success for us
            let status_check = status(config)?;
            if matches!(status_check.state, ServiceState::Running) {
                log::info!("[service] Task was already running - operation successful");
            } else {
                return Err(e);
            }
        }
        return status(config);
    }

    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn stop(config: &Config) -> Result<ServiceStatus> {
    if cfg!(target_os = "macos") {
        let plist = macos_service_file()?;
        let domain = macos_gui_domain()?;
        let primary_target = macos_target(SERVICE_LABEL)?;

        let legacy_plist = macos_service_file_for(LEGACY_SERVICE_LABEL)?;
        let legacy_target = macos_target(LEGACY_SERVICE_LABEL)?;

        // Modern lifecycle path first.
        run_best_effort(
            Command::new("launchctl")
                .arg("bootout")
                .arg(&domain)
                .arg(&primary_target),
        );
        run_best_effort(
            Command::new("launchctl")
                .arg("bootout")
                .arg(&domain)
                .arg(&plist),
        );
        run_best_effort(
            Command::new("launchctl")
                .arg("bootout")
                .arg(&domain)
                .arg(&legacy_target),
        );
        run_best_effort(
            Command::new("launchctl")
                .arg("bootout")
                .arg(&domain)
                .arg(&legacy_plist),
        );

        // Compatibility fallback.
        run_best_effort(Command::new("launchctl").arg("stop").arg(SERVICE_LABEL));
        run_best_effort(Command::new("launchctl").arg("stop").arg(LEGACY_SERVICE_LABEL));
        return status(config);
    }

    if cfg!(target_os = "linux") {
        let _ =
            run_checked(Command::new("systemctl").args(["--user", "stop", "alphahuman.service"]));
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
        let running = out
            .lines()
            .any(|line| line.contains(SERVICE_LABEL) || line.contains(LEGACY_SERVICE_LABEL));
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
        let out =
            run_capture(Command::new("schtasks").args(["/Query", "/TN", task_name, "/FO", "LIST"]));
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
    let daemon_args = daemon_program_args(&exe);
    let program_args_xml = std::iter::once(exe.display().to_string())
        .chain(daemon_args)
        .map(|arg| format!("    <string>{}</string>\n", xml_escape(&arg)))
        .collect::<String>();

    let plist = format!(
        r#"<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{program_args}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>ALPHAHUMAN_DAEMON_INTERNAL</key>
    <string>false</string>
  </dict>
  <key>WorkingDirectory</key>
  <string>{workdir}</string>
  <key>ProcessType</key>
  <string>Background</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        program_args = program_args_xml,
        stdout = xml_escape(&stdout.display().to_string()),
        stderr = xml_escape(&stderr.display().to_string()),
        workdir = xml_escape(
            &config
                .config_path
                .parent()
                .map_or_else(|| PathBuf::from("."), PathBuf::from)
                .display()
                .to_string(),
        )
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
    let exec_start = daemon_command_line(&exe);

    let unit = format!(
        "[Unit]\nDescription=Alphahuman Daemon\n\n[Service]\nExecStart={}\nRestart=always\nRestartSec=3\n\nStandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
        exec_start,
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
    let daemon_cmd = daemon_command_line(&exe);

    let cmd = format!(
        "@echo off\n{} >> \"{}\" 2>> \"{}\"\n",
        daemon_cmd,
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

fn macos_gui_domain() -> Result<String> {
    let uid = run_capture(Command::new("id").arg("-u"))?;
    Ok(format!("gui/{}", uid.trim()))
}

fn macos_target(label: &str) -> Result<String> {
    Ok(format!("{}/{}", macos_gui_domain()?, label))
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

fn run_best_effort(cmd: &mut Command) {
    match cmd.stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(status) => {
            if !status.success() {
                log::debug!("[service] best-effort command failed with status {status}");
            }
        }
        Err(err) => {
            log::debug!("[service] best-effort command failed to execute: {err}");
        }
    }
}

/// Check if the macOS LaunchAgent service is loaded (regardless of running state)
fn is_service_loaded_macos() -> Result<bool> {
    if run_checked(Command::new("launchctl").arg("print").arg(macos_target(SERVICE_LABEL)?))
        .is_ok()
    {
        return Ok(true);
    }
    if run_checked(
        Command::new("launchctl")
            .arg("print")
            .arg(macos_target(LEGACY_SERVICE_LABEL)?),
    )
    .is_ok()
    {
        return Ok(true);
    }
    Ok(false)
}

/// Check if the Linux systemd service is enabled
fn is_service_enabled_linux() -> Result<bool> {
    let result = Command::new("systemctl")
        .args(["--user", "is-enabled", "alphahuman.service"])
        .output();

    match result {
        Ok(output) => {
            let status_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(status_str == "enabled")
        }
        Err(_) => Ok(false), // Service not found or other error means not enabled
    }
}

/// Check if the Windows scheduled task exists
fn is_task_exists_windows(task_name: &str) -> Result<bool> {
    let result = Command::new("schtasks")
        .args(["/Query", "/TN", task_name])
        .output();

    match result {
        Ok(output) => Ok(output.status.success()),
        Err(_) => Ok(false), // Command failed means task doesn't exist
    }
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
