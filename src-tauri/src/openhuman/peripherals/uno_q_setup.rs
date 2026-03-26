//! Deploy OpenHuman Bridge app to Arduino Uno Q.

use anyhow::{Context, Result};
use std::process::Command;

const BRIDGE_APP_NAME: &str = "openhuman-uno-q-bridge";

/// Deploy the Bridge app. If host is Some, scp from repo and ssh to start.
/// If host is None, assume we're ON the Uno Q — use embedded files and start.
pub fn setup_uno_q_bridge(host: Option<&str>) -> Result<()> {
    let bridge_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("firmware")
        .join("openhuman-uno-q-bridge");

    if let Some(h) = host {
        if bridge_dir.exists() {
            deploy_remote(h, &bridge_dir)?;
        } else {
            anyhow::bail!(
                "Bridge app not found at {}. Run from openhuman repo root.",
                bridge_dir.display()
            );
        }
    } else {
        deploy_local(if bridge_dir.exists() {
            Some(&bridge_dir)
        } else {
            None
        })?;
    }
    Ok(())
}

fn deploy_remote(host: &str, bridge_dir: &std::path::Path) -> Result<()> {
    let ssh_target = if host.contains('@') {
        host.to_string()
    } else {
        format!("arduino@{}", host)
    };

    println!("Copying Bridge app to {}...", host);
    let status = Command::new("ssh")
        .args([&ssh_target, "mkdir", "-p", "~/ArduinoApps"])
        .status()
        .context("ssh mkdir failed")?;
    if !status.success() {
        anyhow::bail!("Failed to create ArduinoApps dir on Uno Q");
    }

    let status = Command::new("scp")
        .args([
            "-r",
            bridge_dir.to_str().unwrap(),
            &format!("{}:~/ArduinoApps/", ssh_target),
        ])
        .status()
        .context("scp failed")?;
    if !status.success() {
        anyhow::bail!("Failed to copy Bridge app");
    }

    println!("Starting Bridge app on Uno Q...");
    let status = Command::new("ssh")
        .args([
            &ssh_target,
            "arduino-app-cli",
            "app",
            "start",
            "~/ArduinoApps/openhuman-uno-q-bridge",
        ])
        .status()
        .context("arduino-app-cli start failed")?;
    if !status.success() {
        anyhow::bail!("Failed to start Bridge app. Ensure arduino-app-cli is installed on Uno Q.");
    }

    println!("OpenHuman Bridge app started. Add to config.toml:");
    println!("  [[peripherals.boards]]");
    println!("  board = \"arduino-uno-q\"");
    println!("  transport = \"bridge\"");
    Ok(())
}

fn deploy_local(bridge_dir: Option<&std::path::Path>) -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/arduino".into());
    let apps_dir = std::path::Path::new(&home).join("ArduinoApps");
    let dest_dir = apps_dir.join(BRIDGE_APP_NAME);

    std::fs::create_dir_all(&dest_dir).context("create dest dir")?;

    if let Some(src) = bridge_dir {
        println!("Copying Bridge app from repo...");
        copy_dir(src, &dest_dir)?;
    } else {
        println!("Writing embedded Bridge app...");
        write_embedded_bridge(&dest_dir)?;
    }

    println!("Starting Bridge app...");
    let status = Command::new("arduino-app-cli")
        .args(["app", "start", dest_dir.to_str().unwrap()])
        .status()
        .context("arduino-app-cli start failed")?;
    if !status.success() {
        anyhow::bail!("Failed to start Bridge app. Ensure arduino-app-cli is installed on Uno Q.");
    }

    println!("OpenHuman Bridge app started.");
    Ok(())
}

fn write_embedded_bridge(dest: &std::path::Path) -> Result<()> {
    let app_yaml = include_str!("../../firmware/openhuman-uno-q-bridge/app.yaml");
    let sketch_ino = include_str!("../../firmware/openhuman-uno-q-bridge/sketch/sketch.ino");
    let sketch_yaml = include_str!("../../firmware/openhuman-uno-q-bridge/sketch/sketch.yaml");
    let main_py = include_str!("../../firmware/openhuman-uno-q-bridge/python/main.py");
    let requirements = include_str!("../../firmware/openhuman-uno-q-bridge/python/requirements.txt");

    std::fs::write(dest.join("app.yaml"), app_yaml)?;
    std::fs::create_dir_all(dest.join("sketch"))?;
    std::fs::write(dest.join("sketch").join("sketch.ino"), sketch_ino)?;
    std::fs::write(dest.join("sketch").join("sketch.yaml"), sketch_yaml)?;
    std::fs::create_dir_all(dest.join("python"))?;
    std::fs::write(dest.join("python").join("main.py"), main_py)?;
    std::fs::write(dest.join("python").join("requirements.txt"), requirements)?;
    Ok(())
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    for entry in std::fs::read_dir(src)? {
        let e = entry?;
        let name = e.file_name();
        let src_path = src.join(&name);
        let dst_path = dst.join(&name);
        if e.file_type()?.is_dir() {
            std::fs::create_dir_all(&dst_path)?;
            copy_dir(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
