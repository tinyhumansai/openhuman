//! Glue between the agent tool loop and the TokenJuice content router.
//!
//! Exposes the entry points the agent loop calls after a tool returns output:
//!
//! - [`compact_tool_output`] — full version with the tool's JSON arguments and
//!   exit code; derives a command/argv and content hint, routes through the
//!   content router, and returns `(text, CompactionStats)`.
//! - [`compact_output`] — minimal version (content + tool name + enable flag)
//!   for call sites that only have those, returning just the text.
//!
//! Both are **pass-through safe**: if compression doesn't meaningfully shrink
//! the payload, or the input is under the byte floor, or the router/CCR is
//! disabled, the original string is returned untouched.
//!
//! Runtime options (the `[tokenjuice]` config block) are installed once at
//! startup via [`configure`]; callers don't thread `Config` through.

use once_cell::sync::OnceCell;
use serde_json::Value;
use std::sync::RwLock;

use super::compress::route;
use super::types::{AgentTokenjuiceCompression, CompressInput, CompressOptions, ContentHint};

/// Skip compaction for outputs smaller than this (bytes) by default. Tiny
/// outputs have no headroom and risk distortion. Overridable per the config's
/// `min_bytes_to_compress` once [`configure`] runs.
const DEFAULT_MIN_COMPACT_INPUT_BYTES: usize = 512;

/// Process-global runtime options, installed from config at startup.
fn options_cell() -> &'static RwLock<CompressOptions> {
    static OPTS: OnceCell<RwLock<CompressOptions>> = OnceCell::new();
    OPTS.get_or_init(|| {
        RwLock::new(CompressOptions {
            min_bytes_to_compress: DEFAULT_MIN_COMPACT_INPUT_BYTES,
            ..Default::default()
        })
    })
}

/// Install the runtime [`CompressOptions`] (called once from config at startup).
/// Also configures the CCR cache limits/disk tier indirectly via the caller.
pub fn configure(opts: CompressOptions) {
    *options_cell().write().unwrap_or_else(|p| p.into_inner()) = opts;
}

/// Snapshot the current runtime options.
pub fn current_options() -> CompressOptions {
    options_cell()
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

fn options_for_agent(profile: AgentTokenjuiceCompression) -> Option<CompressOptions> {
    match profile {
        AgentTokenjuiceCompression::Off => None,
        AgentTokenjuiceCompression::Auto | AgentTokenjuiceCompression::Full => {
            Some(current_options())
        }
        AgentTokenjuiceCompression::Light => {
            let mut opts = current_options();
            // Coding agents need raw, exact tool text more than aggressive token
            // savings. Disabling CCR makes every lossy compressor decline in
            // route(), while still allowing any truly lossless reduction.
            opts.ccr_enabled = false;
            opts.ml_text_enabled = false;
            Some(opts)
        }
    }
}

/// Install the full TokenJuice runtime configuration in one call at startup:
/// router/compressor options, CCR cache limits, and the optional on-disk tier.
/// Kept free of the config-schema type so `tokenjuice` stays decoupled — the
/// caller maps `Config.tokenjuice` into these primitives.
#[allow(clippy::too_many_arguments)]
pub fn install_config(
    options: CompressOptions,
    max_cache_entries: usize,
    max_cache_bytes: usize,
    ccr_ttl_secs: Option<u64>,
    disk_tier_root: Option<std::path::PathBuf>,
) {
    configure(options);
    super::cache::configure(max_cache_entries, max_cache_bytes, ccr_ttl_secs);
    // Enable or disable the disk tier to match the setting — a `None` here means
    // the user turned it off, so clear any previously-installed disk root rather
    // than leaving the process writing originals to disk until restart.
    match disk_tier_root {
        Some(root) => super::cache::enable_disk_tier(root),
        None => super::cache::disable_disk_tier(),
    }
    log::debug!("[tokenjuice] runtime config installed");
}

/// Statistics for a single compaction call (back-compat shape).
#[derive(Debug, Clone)]
pub struct CompactionStats {
    pub tool_name: String,
    pub original_bytes: usize,
    pub compacted_bytes: usize,
    /// The compressor kind (or `none/...`) that handled the output.
    pub rule_id: String,
    pub applied: bool,
}

impl CompactionStats {
    pub fn ratio(&self) -> f64 {
        if self.original_bytes == 0 {
            1.0
        } else {
            self.compacted_bytes as f64 / self.original_bytes as f64
        }
    }
}

/// Compact a tool call's output through the content router.
///
/// * `tool_name` — the agent-level tool name (`shell`, `grep`, `browser_navigate`).
/// * `arguments` — the raw JSON arguments; used to derive command/argv (for the
///   log/command rule path) and a file extension (for code/JSON/HTML hints).
/// * `output` — the captured tool output (already credential-scrubbed).
/// * `exit_code` — enables failure-preserving behaviour in the log compressor.
///
/// Returns `(text, stats)`. When `stats.applied == false` the text is the
/// untouched original.
pub async fn compact_tool_output(
    tool_name: &str,
    arguments: Option<&Value>,
    output: &str,
    exit_code: Option<i32>,
) -> (String, CompactionStats) {
    compact_tool_output_with_policy(
        tool_name,
        arguments,
        output,
        exit_code,
        AgentTokenjuiceCompression::Full,
    )
    .await
}

/// Compact a tool call's output using an agent-level TokenJuice profile.
pub async fn compact_tool_output_with_policy(
    tool_name: &str,
    arguments: Option<&Value>,
    output: &str,
    exit_code: Option<i32>,
    profile: AgentTokenjuiceCompression,
) -> (String, CompactionStats) {
    let original_bytes = output.len();

    let Some(opts) = options_for_agent(profile) else {
        log::debug!(
            "[tokenjuice] agent profile disabled compaction tool={} bytes={}",
            tool_name,
            original_bytes
        );
        return (
            output.to_string(),
            CompactionStats {
                tool_name: tool_name.to_string(),
                original_bytes,
                compacted_bytes: original_bytes,
                rule_id: "none/agent-profile-off".to_string(),
                applied: false,
            },
        );
    };

    // A recovery tool's output is the original we previously offloaded — never
    // re-compact it, or the agent could never see the full data it asked for.
    if super::cache::is_recovery_tool(tool_name) {
        return (
            output.to_string(),
            CompactionStats {
                tool_name: tool_name.to_string(),
                original_bytes,
                compacted_bytes: original_bytes,
                rule_id: "none/recovery-tool".to_string(),
                applied: false,
            },
        );
    }

    let (command, argv) = extract_command_argv(arguments);
    let hint = ContentHint {
        source_tool: Some(tool_name.to_string()),
        extension: extract_extension(arguments),
        query: extract_query(arguments),
        ..Default::default()
    };

    let input = CompressInput {
        content: output,
        kind: super::types::ContentKind::PlainText,
        hint: &hint,
        exit_code,
        command,
        argv,
        original_bytes,
    };

    let res = route(input, &opts).await;
    let stats = CompactionStats {
        tool_name: tool_name.to_string(),
        original_bytes,
        compacted_bytes: res.compacted_bytes,
        rule_id: if res.applied {
            res.compressor.as_str().to_string()
        } else {
            format!("none/{}", res.content_kind.as_str())
        },
        applied: res.applied,
    };
    (res.text, stats)
}

/// Minimal compaction for call sites that only have content + tool name. The
/// `enabled` flag is an explicit kill-switch on top of the configured options.
pub async fn compact_output(content: String, tool_name: &str, enabled: bool) -> String {
    compact_output_with_policy(
        content,
        tool_name,
        enabled,
        AgentTokenjuiceCompression::Full,
    )
    .await
}

/// Minimal compaction with an agent-level TokenJuice profile.
pub async fn compact_output_with_policy(
    content: String,
    tool_name: &str,
    enabled: bool,
    profile: AgentTokenjuiceCompression,
) -> String {
    // The call-site `enabled` flag and the configured router switch are both
    // hard off-switches; either one short-circuits to the untouched original.
    if !enabled || !current_options().router_enabled {
        return content;
    }
    let (text, _stats) =
        compact_tool_output_with_policy(tool_name, None, &content, None, profile).await;
    text
}

/// Derive `(command, argv)` from a tool's JSON arguments.
fn extract_command_argv(arguments: Option<&Value>) -> (Option<String>, Option<Vec<String>>) {
    let Some(Value::Object(map)) = arguments else {
        return (None, None);
    };

    if let Some(Value::Array(arr)) = map.get("argv") {
        let argv: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_owned()))
            .collect();
        if !argv.is_empty() {
            let command = argv.join(" ");
            return (Some(command), Some(argv));
        }
    }

    let cmd_str = map
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| map.get("cmd").and_then(Value::as_str));

    if let Some(cmd) = cmd_str {
        if let Some(Value::Array(args)) = map.get("args") {
            let mut argv = vec![cmd.to_owned()];
            argv.extend(args.iter().filter_map(|v| v.as_str().map(|s| s.to_owned())));
            return (Some(format!("{cmd} {}", argv[1..].join(" "))), Some(argv));
        }
        let argv: Vec<String> = cmd.split_whitespace().map(|s| s.to_owned()).collect();
        return (Some(cmd.to_owned()), (!argv.is_empty()).then_some(argv));
    }

    (None, None)
}

/// Derive a file extension hint from common path-bearing argument shapes.
fn extract_extension(arguments: Option<&Value>) -> Option<String> {
    let Some(Value::Object(map)) = arguments else {
        return None;
    };
    let path = ["path", "file_path", "file", "filename"]
        .iter()
        .find_map(|k| map.get(*k).and_then(Value::as_str))?;
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?;
    Some(ext.to_ascii_lowercase())
}

/// Derive a search-query hint from common query-bearing argument shapes.
fn extract_query(arguments: Option<&Value>) -> Option<String> {
    let Some(Value::Object(map)) = arguments else {
        return None;
    };
    ["query", "pattern", "search", "q", "regex"]
        .iter()
        .find_map(|k| map.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn skips_short_output() {
        let (out, stats) = compact_tool_output("shell", None, "hello world", Some(0)).await;
        assert_eq!(out, "hello world");
        assert!(!stats.applied);
        assert_eq!(stats.original_bytes, 11);
    }

    #[tokio::test]
    async fn compacts_long_git_status_via_argv() {
        let mut lines = vec!["On branch main".to_owned()];
        for i in 0..200 {
            lines.push(format!("\tmodified:   src/file_{i}.rs"));
        }
        let output = lines.join("\n");
        let args = json!({"command": "git status"});
        let (compacted, stats) = compact_tool_output("shell", Some(&args), &output, Some(0)).await;
        assert!(stats.applied, "expected compaction, got {:?}", stats);
        assert!(compacted.len() < output.len());
    }

    #[tokio::test]
    async fn passes_through_incompressible_output() {
        let unique_lines: Vec<String> = (0..200)
            .map(|i| format!("unique-payload-chunk-{i}-{}", "x".repeat(30)))
            .collect();
        let output = unique_lines.join("\n");
        let (returned, stats) = compact_tool_output("unknown_tool", None, &output, Some(0)).await;
        if !stats.applied {
            assert_eq!(returned, output);
        }
    }

    #[tokio::test]
    async fn disabled_flag_is_passthrough() {
        let big = "x".repeat(5000);
        assert_eq!(compact_output(big.clone(), "grep", false).await, big);
    }

    #[tokio::test]
    async fn light_agent_profile_declines_lossy_ccr_compaction() {
        let mut lines = vec!["On branch main".to_owned()];
        for i in 0..200 {
            lines.push(format!("\tmodified:   src/file_{i}.rs"));
        }
        let output = lines.join("\n");
        let args = json!({"command": "git status"});
        let (returned, stats) = compact_tool_output_with_policy(
            "shell",
            Some(&args),
            &output,
            Some(0),
            AgentTokenjuiceCompression::Light,
        )
        .await;
        assert_eq!(returned, output);
        assert!(!stats.applied);
    }

    #[tokio::test]
    async fn off_agent_profile_bypasses_router() {
        let big = "x".repeat(5000);
        let returned =
            compact_output_with_policy(big.clone(), "grep", true, AgentTokenjuiceCompression::Off)
                .await;
        assert_eq!(returned, big);
    }

    #[test]
    fn extract_argv_handles_common_shapes() {
        let (cmd, argv) = extract_command_argv(Some(&json!({"command": "git status"})));
        assert_eq!(cmd.as_deref(), Some("git status"));
        assert_eq!(argv.unwrap(), vec!["git", "status"]);

        let (cmd, _) = extract_command_argv(Some(&json!({"command": "cargo", "args": ["test"]})));
        assert_eq!(cmd.as_deref(), Some("cargo test"));
    }

    #[test]
    fn extract_extension_and_query() {
        assert_eq!(
            extract_extension(Some(&json!({"path": "src/lib.rs"}))).as_deref(),
            Some("rs")
        );
        assert_eq!(
            extract_query(Some(&json!({"pattern": "foo bar"}))).as_deref(),
            Some("foo bar")
        );
    }
}
