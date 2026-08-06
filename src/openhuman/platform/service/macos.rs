//! LaunchAgent install/start/stop/status for macOS.

use crate::openhuman::config::Config;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::common::{
    self, run_best_effort, run_capture, run_check_silent, run_checked, xml_escape,
    LEGACY_APP_LABEL, LEGACY_SERVICE_LABEL, SERVICE_LABEL,
};
use super::{ServiceState, ServiceStatus};

pub(crate) fn install(config: &Config) -> Result<()> {
    let file = macos_service_file()?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = common::resolve_daemon_executable()?;
    let logs_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("logs");
    fs::create_dir_all(&logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");
    let daemon_args = common::daemon_program_args(&exe);
    let program_args_xml = std::iter::once(exe.display().to_string())
        .chain(daemon_args)
        .map(|arg| format!("    <string>{}</string>\n", xml_escape(&arg)))
        .collect::<String>();

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
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
    <key>OPENHUMAN_DAEMON_INTERNAL</key>
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

pub(crate) fn start(config: &Config) -> Result<ServiceStatus> {
    let plist = macos_service_file()?;
    let domain = macos_gui_domain()?;
    let primary_target = macos_target(SERVICE_LABEL)?;

    if !plist.exists() {
        log::info!(
            "[service] LaunchAgent plist missing, installing it before start: {}",
            plist.display()
        );
        install(config)?;
    }

    validate_macos_plist(&plist)?;

    if !is_service_loaded_macos()? {
        log::info!("[service] Loading macOS LaunchAgent service");
        run_best_effort(
            Command::new("launchctl")
                .arg("bootout")
                .arg(&domain)
                .arg(&primary_target),
        );
        let bootstrap_ok = run_checked(
            Command::new("launchctl")
                .arg("bootstrap")
                .arg(&domain)
                .arg(&plist),
        );
        if let Err(err) = bootstrap_ok {
            log::warn!("[service] launchctl bootstrap failed, falling back to load -w: {err}");
            run_checked(Command::new("launchctl").arg("load").arg("-w").arg(&plist))?;
        }
    } else {
        log::info!("[service] LaunchAgent service already loaded, skipping load step");
    }

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
        let status_check = super::status(config)?;
        if matches!(status_check.state, ServiceState::Running) {
            log::info!("[service] Service was already running - operation successful");
        } else {
            return Err(e);
        }
    }
    super::status(config)
}

pub(crate) fn stop(_config: &Config) -> Result<()> {
    let plist = macos_service_file()?;
    let domain = macos_gui_domain()?;
    let primary_target = macos_target(SERVICE_LABEL)?;

    let legacy_plist = macos_service_file_for(LEGACY_SERVICE_LABEL)?;
    let legacy_target = macos_target(LEGACY_SERVICE_LABEL)?;
    let legacy_app_plist = macos_service_file_for(LEGACY_APP_LABEL)?;
    let legacy_app_target = macos_target(LEGACY_APP_LABEL)?;

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
    run_best_effort(
        Command::new("launchctl")
            .arg("bootout")
            .arg(&domain)
            .arg(&legacy_app_target),
    );
    run_best_effort(
        Command::new("launchctl")
            .arg("bootout")
            .arg(&domain)
            .arg(&legacy_app_plist),
    );

    run_best_effort(Command::new("launchctl").arg("stop").arg(SERVICE_LABEL));
    run_best_effort(
        Command::new("launchctl")
            .arg("stop")
            .arg(LEGACY_SERVICE_LABEL),
    );
    run_best_effort(Command::new("launchctl").arg("stop").arg(LEGACY_APP_LABEL));
    Ok(())
}

pub(crate) fn status(_config: &Config) -> Result<ServiceStatus> {
    let running = is_service_running_macos()?;
    Ok(ServiceStatus {
        state: if running {
            ServiceState::Running
        } else {
            ServiceState::Stopped
        },
        unit_path: Some(macos_service_file()?),
        label: SERVICE_LABEL.to_string(),
        details: None,
    })
}

pub(crate) fn uninstall(_config: &Config) -> Result<ServiceStatus> {
    let file = macos_service_file()?;
    if file.exists() {
        fs::remove_file(&file).with_context(|| format!("Failed to remove {}", file.display()))?;
    }
    let legacy_file = macos_service_file_for(LEGACY_SERVICE_LABEL)?;
    if legacy_file.exists() {
        let _ = fs::remove_file(&legacy_file);
    }
    let legacy_app_file = macos_service_file_for(LEGACY_APP_LABEL)?;
    if legacy_app_file.exists() {
        let _ = fs::remove_file(&legacy_app_file);
    }
    Ok(ServiceStatus {
        state: ServiceState::NotInstalled,
        unit_path: Some(file),
        label: SERVICE_LABEL.to_string(),
        details: None,
    })
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

fn validate_macos_plist(path: &std::path::Path) -> Result<()> {
    run_checked(Command::new("plutil").arg("-lint").arg(path))
        .with_context(|| format!("Invalid launch agent plist: {}", path.display()))
}

fn is_service_loaded_macos() -> Result<bool> {
    for target in candidate_macos_targets(SERVICE_LABEL)? {
        if run_check_silent(Command::new("launchctl").arg("print").arg(target)) {
            return Ok(true);
        }
    }
    for target in candidate_macos_targets(LEGACY_SERVICE_LABEL)? {
        if run_check_silent(Command::new("launchctl").arg("print").arg(target)) {
            return Ok(true);
        }
    }
    for target in candidate_macos_targets(LEGACY_APP_LABEL)? {
        if run_check_silent(Command::new("launchctl").arg("print").arg(target)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_service_running_macos() -> Result<bool> {
    if is_service_loaded_macos()? {
        return Ok(true);
    }

    // Fallback for environments where `launchctl print` is restricted.
    let out = run_capture(Command::new("launchctl").arg("list"))?;
    Ok(out.lines().any(|line| {
        line.contains(SERVICE_LABEL)
            || line.contains(LEGACY_SERVICE_LABEL)
            || line.contains(LEGACY_APP_LABEL)
    }))
}

fn candidate_macos_targets(label: &str) -> Result<Vec<String>> {
    let uid = run_capture(Command::new("id").arg("-u"))?;
    let uid = uid.trim();
    Ok(vec![
        format!("gui/{uid}/{label}"),
        format!("user/{uid}/{label}"),
        format!("system/{label}"),
    ])
}
