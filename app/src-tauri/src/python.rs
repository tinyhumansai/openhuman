// Bundled Python interpreter (python-build-standalone) resolver.
//
// The `stage-python-sidecar.mjs` build step extracts a relocatable CPython
// distribution to `app/src-tauri/python/`, and `tauri.conf.json` ships it as
// a resource. At runtime we resolve the interpreter path relative to the
// Tauri resource directory so skills can spawn `python3` / `pip` without
// depending on anything on the end-user's machine.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

/// Path to the bundled python interpreter for the current platform.
///
/// On Unix this is `<resources>/python/bin/python3`; on Windows it is
/// `<resources>\python\python.exe`. Returns an error if the resource dir
/// cannot be located or the interpreter is missing (e.g. `python:stage`
/// was not run before the build).
pub fn interpreter_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("resource_dir unavailable: {e}"))?;
    let mut path = resource_dir.join("python");
    if cfg!(windows) {
        path.push("python.exe");
    } else {
        path.push("bin");
        path.push("python3");
    }
    if !path.exists() {
        return Err(format!("bundled python not found at {}", path.display()));
    }
    Ok(path)
}

/// Directory containing the bundled python distribution (parent of `bin/`
/// on Unix, the dist root on Windows). Useful when callers need to set
/// `PYTHONHOME` or locate `site-packages`.
pub fn home_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("resource_dir unavailable: {e}"))?;
    Ok(resource_dir.join("python"))
}

/// Diagnostic command: returns the interpreter path and `python --version`
/// output so the frontend can confirm the sidecar staged correctly.
#[tauri::command]
pub async fn python_info<R: Runtime>(app: AppHandle<R>) -> Result<serde_json::Value, String> {
    let path = interpreter_path(&app)?;
    let output = std::process::Command::new(&path)
        .arg("--version")
        .output()
        .map_err(|e| format!("failed to invoke bundled python: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let version = if !stdout.is_empty() { stdout } else { stderr };
    Ok(serde_json::json!({
        "path": path.to_string_lossy(),
        "home": home_dir(&app)?.to_string_lossy(),
        "version": version,
        "exit_code": output.status.code(),
    }))
}
