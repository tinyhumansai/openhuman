//! LaunchAgent install/start/stop/status for macOS.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::common::{run_best_effort, run_capture, run_check_silent, run_checked, xml_escape};
use super::{ServiceSpec, ServiceState, ServiceStatus};

pub(crate) fn install(spec: &ServiceSpec) -> Result<()> {
    let file = macos_service_file_for(&spec.label)?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let logs_dir = &spec.logs_dir;
    fs::create_dir_all(logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");
    let program_args_xml = std::iter::once(spec.exe.display().to_string())
        .chain(spec.args.iter().cloned())
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
        label = xml_escape(&spec.label),
        program_args = program_args_xml,
        stdout = xml_escape(&stdout.display().to_string()),
        stderr = xml_escape(&stderr.display().to_string()),
        workdir = xml_escape(&spec.work_dir.display().to_string()),
    );

    fs::write(&file, plist)?;
    Ok(())
}

pub(crate) fn start(spec: &ServiceSpec) -> Result<ServiceStatus> {
    let plist = macos_service_file_for(&spec.label)?;
    let domain = macos_gui_domain()?;
    let primary_target = macos_target(&spec.label)?;

    if !plist.exists() {
        log::info!(
            "[service] LaunchAgent plist missing, installing it before start: {}",
            plist.display()
        );
        install(spec)?;
    }

    validate_macos_plist(&plist)?;

    if !is_service_loaded_macos(spec)? {
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
        let _ = run_checked(Command::new("launchctl").arg("start").arg(&spec.label));
        let status_check = super::status(spec)?;
        if matches!(status_check.state, ServiceState::Running) {
            log::info!("[service] Service was already running - operation successful");
        } else {
            return Err(e);
        }
    }
    super::status(spec)
}

pub(crate) fn stop(spec: &ServiceSpec) -> Result<()> {
    let domain = macos_gui_domain()?;

    // Current label first, then every legacy one: boot out both the service
    // target and its plist path, since either may be what launchd holds.
    for label in all_labels(spec) {
        let plist = macos_service_file_for(label)?;
        let target = macos_target(label)?;
        run_best_effort(
            Command::new("launchctl")
                .arg("bootout")
                .arg(&domain)
                .arg(&target),
        );
        run_best_effort(
            Command::new("launchctl")
                .arg("bootout")
                .arg(&domain)
                .arg(&plist),
        );
    }

    for label in all_labels(spec) {
        run_best_effort(Command::new("launchctl").arg("stop").arg(label));
    }
    Ok(())
}

pub(crate) fn status(spec: &ServiceSpec) -> Result<ServiceStatus> {
    let running = is_service_running_macos(spec)?;
    Ok(ServiceStatus {
        state: if running {
            ServiceState::Running
        } else {
            ServiceState::Stopped
        },
        unit_path: Some(macos_service_file_for(&spec.label)?),
        label: spec.label.clone(),
        details: None,
    })
}

pub(crate) fn uninstall(spec: &ServiceSpec) -> Result<ServiceStatus> {
    let file = macos_service_file_for(&spec.label)?;
    if file.exists() {
        fs::remove_file(&file).with_context(|| format!("Failed to remove {}", file.display()))?;
    }
    for label in &spec.legacy_labels {
        let legacy_file = macos_service_file_for(label)?;
        if legacy_file.exists() {
            let _ = fs::remove_file(&legacy_file);
        }
    }
    Ok(ServiceStatus {
        state: ServiceState::NotInstalled,
        unit_path: Some(file),
        label: spec.label.clone(),
        details: None,
    })
}

/// The current label followed by every legacy one.
fn all_labels(spec: &ServiceSpec) -> impl Iterator<Item = &str> {
    std::iter::once(spec.label.as_str()).chain(spec.legacy_labels.iter().map(String::as_str))
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

fn is_service_loaded_macos(spec: &ServiceSpec) -> Result<bool> {
    for label in all_labels(spec) {
        for target in candidate_macos_targets(label)? {
            if run_check_silent(Command::new("launchctl").arg("print").arg(target)) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn is_service_running_macos(spec: &ServiceSpec) -> Result<bool> {
    if is_service_loaded_macos(spec)? {
        return Ok(true);
    }

    // Fallback for environments where `launchctl print` is restricted.
    let out = run_capture(Command::new("launchctl").arg("list"))?;
    Ok(out
        .lines()
        .any(|line| all_labels(spec).any(|label| line.contains(label))))
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
