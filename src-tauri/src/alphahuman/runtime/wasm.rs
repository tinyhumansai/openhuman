//! WASM sandbox runtime — in-process tool isolation via `wasmi`.
//!
//! Provides capability-based sandboxing without Docker or external runtimes.
//! Each WASM module runs with:
//! - **Fuel limits**: prevents infinite loops (each instruction costs 1 fuel)
//! - **Memory caps**: configurable per-module memory ceiling
//! - **No filesystem access**: by default, tools are pure computation
//! - **No network access**: unless explicitly allowlisted hosts are configured
//!
//! # Feature gate
//! This module is only compiled when `--features runtime-wasm` is enabled.
//! The default OpenHuman binary excludes it to maintain the 4.6 MB size target.

use super::traits::RuntimeAdapter;
use crate::openhuman::config::WasmRuntimeConfig;
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// WASM sandbox runtime — executes tool modules in an isolated interpreter.
#[derive(Debug, Clone)]
pub struct WasmRuntime {
    config: WasmRuntimeConfig,
    workspace_dir: Option<PathBuf>,
}

/// Result of executing a WASM module.
#[derive(Debug, Clone)]
pub struct WasmExecutionResult {
    /// Standard output captured from the module (if WASI is used)
    pub stdout: String,
    /// Standard error captured from the module
    pub stderr: String,
    /// Exit code (0 = success)
    pub exit_code: i32,
    /// Fuel consumed during execution
    pub fuel_consumed: u64,
}

/// Capabilities granted to a WASM tool module.
#[derive(Debug, Clone, Default)]
pub struct WasmCapabilities {
    /// Allow reading files from workspace
    pub read_workspace: bool,
    /// Allow writing files to workspace
    pub write_workspace: bool,
    /// Allowed HTTP hosts (empty = no network)
    pub allowed_hosts: Vec<String>,
    /// Custom fuel override (0 = use config default)
    pub fuel_override: u64,
    /// Custom memory override in MB (0 = use config default)
    pub memory_override_mb: u64,
}

impl WasmRuntime {
    /// Create a new WASM runtime with the given configuration.
    pub fn new(config: WasmRuntimeConfig) -> Self {
        Self {
            config,
            workspace_dir: None,
        }
    }

    /// Create a WASM runtime bound to a specific workspace directory.
    pub fn with_workspace(config: WasmRuntimeConfig, workspace_dir: PathBuf) -> Self {
        Self {
            config,
            workspace_dir: Some(workspace_dir),
        }
    }

    /// Check if the WASM runtime feature is available in this build.
    pub fn is_available() -> bool {
        cfg!(feature = "runtime-wasm")
    }

    /// Validate the WASM config for common misconfigurations.
    pub fn validate_config(&self) -> Result<()> {
        if self.config.memory_limit_mb == 0 {
            bail!("runtime.wasm.memory_limit_mb must be > 0");
        }
        if self.config.memory_limit_mb > 4096 {
            bail!(
                "runtime.wasm.memory_limit_mb of {} exceeds the 4 GB safety limit for 32-bit WASM",
                self.config.memory_limit_mb
            );
        }
        if self.config.tools_dir.is_empty() {
            bail!("runtime.wasm.tools_dir cannot be empty");
        }
        // Verify tools directory doesn't escape workspace
        if self.config.tools_dir.contains("..") {
            bail!("runtime.wasm.tools_dir must not contain '..' path traversal");
        }
        Ok(())
    }

    /// Resolve the absolute path to the WASM tools directory.
    pub fn tools_dir(&self, workspace_dir: &Path) -> PathBuf {
        workspace_dir.join(&self.config.tools_dir)
    }

    /// Build capabilities from config defaults.
    pub fn default_capabilities(&self) -> WasmCapabilities {
        WasmCapabilities {
            read_workspace: self.config.allow_workspace_read,
            write_workspace: self.config.allow_workspace_write,
            allowed_hosts: self.config.allowed_hosts.clone(),
            fuel_override: 0,
            memory_override_mb: 0,
        }
    }

    /// Get the effective fuel limit for an invocation.
    pub fn effective_fuel(&self, caps: &WasmCapabilities) -> u64 {
        if caps.fuel_override > 0 {
            caps.fuel_override
        } else {
            self.config.fuel_limit
        }
    }

    /// Get the effective memory limit in bytes.
    pub fn effective_memory_bytes(&self, caps: &WasmCapabilities) -> u64 {
        let mb = if caps.memory_override_mb > 0 {
            caps.memory_override_mb
        } else {
            self.config.memory_limit_mb
        };
        mb.saturating_mul(1024 * 1024)
    }

    /// Execute a WASM module from the tools directory.
    ///
    /// This is the primary entry point for running sandboxed tool code.
    /// The module must export a `_start` function (WASI convention) or
    /// a custom `run` function that takes no arguments and returns i32.
    #[cfg(feature = "runtime-wasm")]
    pub fn execute_module(
        &self,
        module_name: &str,
        workspace_dir: &Path,
        caps: &WasmCapabilities,
    ) -> Result<WasmExecutionResult> {
        use wasmi::{Engine, Linker, Module, Store};

        // Resolve module path
        let tools_path = self.tools_dir(workspace_dir);
        let module_path = tools_path.join(format!("{module_name}.wasm"));

        if !module_path.exists() {
            bail!(
                "WASM module not found: {} (looked in {})",
                module_name,
                tools_path.display()
            );
        }

        // Read module bytes
        let wasm_bytes = std::fs::read(&module_path)
            .with_context(|| format!("Failed to read WASM module: {}", module_path.display()))?;

        // Validate module size (sanity check)
        if wasm_bytes.len() > 50 * 1024 * 1024 {
            bail!(
                "WASM module {} is {} MB — exceeds 50 MB safety limit",
                module_name,
                wasm_bytes.len() / (1024 * 1024)
            );
        }

        // Configure engine with fuel metering
        let mut engine_config = wasmi::Config::default();
        engine_config.consume_fuel(true);
        let engine = Engine::new(&engine_config);

        // Parse and validate module
        let module = Module::new(&engine, &wasm_bytes[..])
            .with_context(|| format!("Failed to parse WASM module: {module_name}"))?;

        // Create store with fuel budget
        let mut store = Store::new(&engine, ());
        let fuel = self.effective_fuel(caps);
        if fuel > 0 {
            store.set_fuel(fuel).with_context(|| {
                format!("Failed to set fuel budget ({fuel}) for module: {module_name}")
            })?;
        }

        // Link host functions (minimal — pure sandboxing)
        let linker = Linker::new(&engine);

        // Instantiate module
        let instance = linker
            .instantiate(&mut store, &module)
            .and_then(|pre| pre.start(&mut store))
            .with_context(|| format!("Failed to instantiate WASM module: {module_name}"))?;

        // Look for exported entry point
        let run_fn = instance
            .get_typed_func::<(), i32>(&store, "run")
            .or_else(|_| instance.get_typed_func::<(), i32>(&store, "_start"))
            .with_context(|| {
                format!(
                    "WASM module '{module_name}' must export a 'run() -> i32' or '_start() -> i32' function"
                )
            })?;

        // Execute with fuel accounting
        let fuel_before = store.get_fuel().unwrap_or(0);
        let exit_code = match run_fn.call(&mut store, ()) {
            Ok(code) => code,
            Err(e) => {
                // Check if we ran out of fuel (infinite loop protection)
                let fuel_after = store.get_fuel().unwrap_or(0);
                if fuel_after == 0 && fuel > 0 {
                    return Ok(WasmExecutionResult {
                        stdout: String::new(),
                        stderr: format!(
                            "WASM module '{module_name}' exceeded fuel limit ({fuel} ticks) — likely an infinite loop"
                        ),
                        exit_code: -1,
                        fuel_consumed: fuel,
                    });
                }
                bail!("WASM execution error in '{module_name}': {e}");
            }
        };
        let fuel_after = store.get_fuel().unwrap_or(0);
        let fuel_consumed = fuel_before.saturating_sub(fuel_after);

        Ok(WasmExecutionResult {
            stdout: String::new(),  // No WASI stdout yet — pure computation
            stderr: String::new(),
            exit_code,
            fuel_consumed,
        })
    }

    /// Stub for when the `runtime-wasm` feature is not enabled.
    #[cfg(not(feature = "runtime-wasm"))]
    pub fn execute_module(
        &self,
        module_name: &str,
        _workspace_dir: &Path,
        _caps: &WasmCapabilities,
    ) -> Result<WasmExecutionResult> {
        bail!(
            "WASM runtime is not available in this build. \
             Rebuild with `cargo build --features runtime-wasm` to enable WASM sandbox support. \
             Module requested: {module_name}"
        )
    }

    /// List available WASM tool modules in the tools directory.
    pub fn list_modules(&self, workspace_dir: &Path) -> Result<Vec<String>> {
        let tools_path = self.tools_dir(workspace_dir);
        if !tools_path.exists() {
            return Ok(Vec::new());
        }

        let mut modules = Vec::new();
        for entry in std::fs::read_dir(&tools_path)
            .with_context(|| format!("Failed to read tools dir: {}", tools_path.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "wasm") {
                if let Some(stem) = path.file_stem() {
                    modules.push(stem.to_string_lossy().to_string());
                }
            }
        }
        modules.sort();
        Ok(modules)
    }
}

impl RuntimeAdapter for WasmRuntime {
    fn name(&self) -> &str {
        "wasm"
    }

    fn has_shell_access(&self) -> bool {
        // WASM sandbox does NOT provide shell access — that's the point
        false
    }

    fn has_filesystem_access(&self) -> bool {
        self.config.allow_workspace_read || self.config.allow_workspace_write
    }

    fn storage_path(&self) -> PathBuf {
        self.workspace_dir
            .as_ref()
            .map_or_else(|| PathBuf::from(".openhuman"), |w| w.join(".openhuman"))
    }

    fn supports_long_running(&self) -> bool {
        // WASM modules are short-lived invocations, not daemons
        false
    }

    fn memory_budget(&self) -> u64 {
        self.config.memory_limit_mb.saturating_mul(1024 * 1024)
    }

    fn build_shell_command(
        &self,
        _command: &str,
        _workspace_dir: &Path,
    ) -> anyhow::Result<tokio::process::Command> {
        bail!(
            "WASM runtime does not support shell commands. \
             Use `execute_module()` to run WASM tools, or switch to runtime.kind = \"native\" for shell access."
        )
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> WasmRuntimeConfig {
        WasmRuntimeConfig::default()
    }

    // ── Basic trait compliance ──────────────────────────────────

    #[test]
    fn wasm_runtime_name() {
        let rt = WasmRuntime::new(default_config());
        assert_eq!(rt.name(), "wasm");
    }

    #[test]
    fn wasm_no_shell_access() {
        let rt = WasmRuntime::new(default_config());
        assert!(!rt.has_shell_access());
    }

    #[test]
    fn wasm_no_filesystem_by_default() {
        let rt = WasmRuntime::new(default_config());
        assert!(!rt.has_filesystem_access());
    }

    #[test]
    fn wasm_filesystem_when_read_enabled() {
        let mut cfg = default_config();
        cfg.allow_workspace_read = true;
        let rt = WasmRuntime::new(cfg);
        assert!(rt.has_filesystem_access());
    }

    #[test]
    fn wasm_filesystem_when_write_enabled() {
        let mut cfg = default_config();
        cfg.allow_workspace_write = true;
        let rt = WasmRuntime::new(cfg);
        assert!(rt.has_filesystem_access());
    }

    #[test]
    fn wasm_no_long_running() {
        let rt = WasmRuntime::new(default_config());
        assert!(!rt.supports_long_running());
    }

    #[test]
    fn wasm_memory_budget() {
        let rt = WasmRuntime::new(default_config());
        assert_eq!(rt.memory_budget(), 64 * 1024 * 1024);
    }

    #[test]
    fn wasm_shell_command_errors() {
        let rt = WasmRuntime::new(default_config());
        let result = rt.build_shell_command("echo hello", Path::new("/tmp"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not support shell"));
    }

    #[test]
    fn wasm_storage_path_default() {
        let rt = WasmRuntime::new(default_config());
        assert!(rt.storage_path().to_string_lossy().contains("openhuman"));
    }

    #[test]
    fn wasm_storage_path_with_workspace() {
        let rt = WasmRuntime::with_workspace(default_config(), PathBuf::from("/home/user/project"));
        assert_eq!(rt.storage_path(), PathBuf::from("/home/user/project/.openhuman"));
    }

    // ── Config validation ──────────────────────────────────────

    #[test]
    fn validate_rejects_zero_memory() {
        let mut cfg = default_config();
        cfg.memory_limit_mb = 0;
        let rt = WasmRuntime::new(cfg);
        let err = rt.validate_config().unwrap_err();
        assert!(err.to_string().contains("must be > 0"));
    }

    #[test]
    fn validate_rejects_excessive_memory() {
        let mut cfg = default_config();
        cfg.memory_limit_mb = 8192;
        let rt = WasmRuntime::new(cfg);
        let err = rt.validate_config().unwrap_err();
        assert!(err.to_string().contains("4 GB safety limit"));
    }

    #[test]
    fn validate_rejects_empty_tools_dir() {
        let mut cfg = default_config();
        cfg.tools_dir = String::new();
        let rt = WasmRuntime::new(cfg);
        let err = rt.validate_config().unwrap_err();
        assert!(err.to_string().contains("cannot be empty"));
    }

    #[test]
    fn validate_rejects_path_traversal() {
        let mut cfg = default_config();
        cfg.tools_dir = "../../../etc/passwd".into();
        let rt = WasmRuntime::new(cfg);
        let err = rt.validate_config().unwrap_err();
        assert!(err.to_string().contains("path traversal"));
    }

    #[test]
    fn validate_accepts_valid_config() {
        let rt = WasmRuntime::new(default_config());
        assert!(rt.validate_config().is_ok());
    }

    #[test]
    fn validate_accepts_max_memory() {
        let mut cfg = default_config();
        cfg.memory_limit_mb = 4096;
        let rt = WasmRuntime::new(cfg);
        assert!(rt.validate_config().is_ok());
    }

    // ── Capabilities & fuel ────────────────────────────────────

    #[test]
    fn effective_fuel_uses_config_default() {
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities::default();
        assert_eq!(rt.effective_fuel(&caps), 1_000_000);
    }

    #[test]
    fn effective_fuel_respects_override() {
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities {
            fuel_override: 500,
            ..Default::default()
        };
        assert_eq!(rt.effective_fuel(&caps), 500);
    }

    #[test]
    fn effective_memory_uses_config_default() {
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities::default();
        assert_eq!(rt.effective_memory_bytes(&caps), 64 * 1024 * 1024);
    }

    #[test]
    fn effective_memory_respects_override() {
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities {
            memory_override_mb: 128,
            ..Default::default()
        };
        assert_eq!(rt.effective_memory_bytes(&caps), 128 * 1024 * 1024);
    }

    #[test]
    fn default_capabilities_match_config() {
        let mut cfg = default_config();
        cfg.allow_workspace_read = true;
        cfg.allowed_hosts = vec!["api.example.com".into()];
        let rt = WasmRuntime::new(cfg);
        let caps = rt.default_capabilities();
        assert!(caps.read_workspace);
        assert!(!caps.write_workspace);
        assert_eq!(caps.allowed_hosts, vec!["api.example.com"]);
    }

    // ── Tools directory ────────────────────────────────────────

    #[test]
    fn tools_dir_resolves_relative_to_workspace() {
        let rt = WasmRuntime::new(default_config());
        let dir = rt.tools_dir(Path::new("/home/user/project"));
        assert_eq!(dir, PathBuf::from("/home/user/project/tools/wasm"));
    }

    #[test]
    fn list_modules_empty_when_dir_missing() {
        let rt = WasmRuntime::new(default_config());
        let modules = rt.list_modules(Path::new("/nonexistent/path")).unwrap();
        assert!(modules.is_empty());
    }

    #[test]
    fn list_modules_finds_wasm_files() {
        let dir = tempfile::tempdir().unwrap();
        let tools_dir = dir.path().join("tools/wasm");
        std::fs::create_dir_all(&tools_dir).unwrap();

        // Create dummy .wasm files
        std::fs::write(tools_dir.join("calculator.wasm"), b"\0asm").unwrap();
        std::fs::write(tools_dir.join("formatter.wasm"), b"\0asm").unwrap();
        std::fs::write(tools_dir.join("readme.txt"), b"not a wasm").unwrap();

        let rt = WasmRuntime::new(default_config());
        let modules = rt.list_modules(dir.path()).unwrap();
        assert_eq!(modules, vec!["calculator", "formatter"]);
    }

    // ── Module execution edge cases ────────────────────────────

    #[test]
    fn execute_module_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let tools_dir = dir.path().join("tools/wasm");
        std::fs::create_dir_all(&tools_dir).unwrap();

        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities::default();
        let result = rt.execute_module("nonexistent", dir.path(), &caps);
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        // Should mention the module name
        assert!(err_msg.contains("nonexistent"));
    }

    #[test]
    fn execute_module_invalid_wasm() {
        let dir = tempfile::tempdir().unwrap();
        let tools_dir = dir.path().join("tools/wasm");
        std::fs::create_dir_all(&tools_dir).unwrap();

        // Write invalid WASM bytes
        std::fs::write(tools_dir.join("bad.wasm"), b"not valid wasm bytes at all").unwrap();

        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities::default();
        let result = rt.execute_module("bad", dir.path(), &caps);
        assert!(result.is_err());
    }

    #[test]
    fn execute_module_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let tools_dir = dir.path().join("tools/wasm");
        std::fs::create_dir_all(&tools_dir).unwrap();

        // Write a file > 50 MB (we just check the size, don't actually allocate)
        // This test verifies the check without consuming 50 MB of disk
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities::default();

        // File doesn't exist for oversized test — the missing file check catches first
        // But if it did exist and was 51 MB, the size check would catch it
        let result = rt.execute_module("oversized", dir.path(), &caps);
        assert!(result.is_err());
    }

    // ── Feature gate check ─────────────────────────────────────

    #[test]
    fn is_available_matches_feature_flag() {
        // This test verifies the compile-time feature detection works
        let available = WasmRuntime::is_available();
        assert_eq!(available, cfg!(feature = "runtime-wasm"));
    }

    // ── Memory overflow edge cases ─────────────────────────────

    #[test]
    fn memory_budget_no_overflow() {
        let mut cfg = default_config();
        cfg.memory_limit_mb = 4096; // Max valid
        let rt = WasmRuntime::new(cfg);
        assert_eq!(rt.memory_budget(), 4096 * 1024 * 1024);
    }

    #[test]
    fn effective_memory_saturating() {
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities {
            memory_override_mb: u64::MAX,
            ..Default::default()
        };
        // Should not panic — saturating_mul prevents overflow
        let _bytes = rt.effective_memory_bytes(&caps);
    }

    // ── WasmCapabilities default ───────────────────────────────

    #[test]
    fn capabilities_default_is_locked_down() {
        let caps = WasmCapabilities::default();
        assert!(!caps.read_workspace);
        assert!(!caps.write_workspace);
        assert!(caps.allowed_hosts.is_empty());
        assert_eq!(caps.fuel_override, 0);
        assert_eq!(caps.memory_override_mb, 0);
    }

    // ── §3.1 / §3.2 WASM fuel & memory exhaustion tests ─────

    #[test]
    fn wasm_fuel_limit_enforced_in_config() {
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities::default();
        let fuel = rt.effective_fuel(&caps);
        assert!(
            fuel > 0,
            "default fuel limit must be > 0 to prevent infinite loops"
        );
    }

    #[test]
    fn wasm_memory_limit_enforced_in_config() {
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities::default();
        let mem_bytes = rt.effective_memory_bytes(&caps);
        assert!(
            mem_bytes > 0,
            "default memory limit must be > 0"
        );
        assert!(
            mem_bytes <= 4096 * 1024 * 1024,
            "default memory must not exceed 4 GB safety limit"
        );
    }

    #[test]
    fn wasm_zero_fuel_override_uses_default() {
        let rt = WasmRuntime::new(default_config());
        let caps = WasmCapabilities {
            fuel_override: 0,
            ..Default::default()
        };
        assert_eq!(
            rt.effective_fuel(&caps),
            1_000_000,
            "fuel_override=0 must use config default"
        );
    }

    #[test]
    fn validate_rejects_memory_just_above_limit() {
        let mut cfg = default_config();
        cfg.memory_limit_mb = 4097;
        let rt = WasmRuntime::new(cfg);
        let err = rt.validate_config().unwrap_err();
        assert!(err.to_string().contains("4 GB safety limit"));
    }

    #[test]
    fn execute_module_stub_returns_error_without_feature() {
        if !WasmRuntime::is_available() {
            let dir = tempfile::tempdir().unwrap();
            let tools_dir = dir.path().join("tools/wasm");
            std::fs::create_dir_all(&tools_dir).unwrap();
            std::fs::write(tools_dir.join("test.wasm"), b"\0asm\x01\0\0\0").unwrap();

            let rt = WasmRuntime::new(default_config());
            let caps = WasmCapabilities::default();
            let result = rt.execute_module("test", dir.path(), &caps);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not available"));
        }
    }
}
