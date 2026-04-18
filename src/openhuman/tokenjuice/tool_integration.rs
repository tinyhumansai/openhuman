//! Glue between the agent tool loop and the tokenjuice reduction engine.
//!
//! Exposes a single entry point — [`compact_tool_output`] — that the agent
//! loop calls after a tool returns its output.  It builds a
//! [`ToolExecutionInput`] from whatever metadata the caller has (tool name,
//! JSON arguments, exit code) and runs the reduction pipeline with the
//! lazily-cached builtin rule set.
//!
//! The function is **pass-through safe**: if reduction does not meaningfully
//! shrink the payload (below [`MIN_COMPACT_RATIO`]) or if the input is already
//! under [`MIN_COMPACT_INPUT_BYTES`], the original string is returned
//! untouched.  Callers do not need to guard the call site.

use once_cell::sync::Lazy;
use serde_json::Value;

use super::reduce::reduce_execution_with_rules;
use super::rules::load_builtin_rules;
use super::types::{CompiledRule, ReduceOptions, ToolExecutionInput};

/// Skip compaction for outputs smaller than this (bytes). Tiny outputs have
/// no headroom to benefit from head/tail summarisation and risk being
/// distorted by rule matches that were designed for long logs.
const MIN_COMPACT_INPUT_BYTES: usize = 512;

/// Keep the compacted form only if it is at most this fraction of the
/// original length. Between `MIN_COMPACT_RATIO` and 1.0 the compaction is
/// considered not worthwhile and the raw output is returned.
const MIN_COMPACT_RATIO: f64 = 0.95;

static BUILTIN_RULES: Lazy<Vec<CompiledRule>> = Lazy::new(load_builtin_rules);

/// Statistics for a single compaction call.
#[derive(Debug, Clone)]
pub struct CompactionStats {
    pub tool_name: String,
    pub original_bytes: usize,
    pub compacted_bytes: usize,
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

/// Compact a tool call's output using tokenjuice's builtin rule set.
///
/// * `tool_name` — the agent-level tool name (e.g. `"shell"`,
///   `"browser_navigate"`). When the tool is a shell wrapper, callers should
///   pass the *underlying* tool name (e.g. `"git"`) by extracting it from
///   `arguments`, but passing the agent tool name also works — rules also
///   match on `commandIncludes` / `argvIncludes`.
/// * `arguments` — the raw JSON arguments the agent passed to the tool.
///   Used to heuristically derive `command` / `argv` for shell-style tools.
/// * `output` — the captured tool output (already credential-scrubbed).
/// * `exit_code` — if known; enables failure-preserving behaviour (rules
///   with a `failure` block use `failure.head`/`failure.tail` instead of the
///   default summarise window when this is non-zero).
///
/// Returns `(compacted_text, stats)`. When `stats.applied == false` the
/// returned string is the untouched original.
pub fn compact_tool_output(
    tool_name: &str,
    arguments: Option<&Value>,
    output: &str,
    exit_code: Option<i32>,
) -> (String, CompactionStats) {
    let original_bytes = output.len();

    if original_bytes < MIN_COMPACT_INPUT_BYTES {
        log::debug!(
            "[tokenjuice] skipping tool={} bytes={} reason=too-small",
            tool_name,
            original_bytes
        );
        return (
            output.to_owned(),
            CompactionStats {
                tool_name: tool_name.to_owned(),
                original_bytes,
                compacted_bytes: original_bytes,
                rule_id: "none/too-small".to_owned(),
                applied: false,
            },
        );
    }

    let (command, argv) = extract_command_argv(arguments);

    let input = ToolExecutionInput {
        tool_name: tool_name.to_owned(),
        command,
        argv,
        stdout: Some(output.to_owned()),
        exit_code,
        ..Default::default()
    };

    let result = reduce_execution_with_rules(input, &BUILTIN_RULES, &ReduceOptions::default());
    let compacted_bytes = result.inline_text.len();
    let rule_id = result
        .classification
        .matched_reducer
        .clone()
        .unwrap_or_else(|| result.classification.family.clone());

    let ratio = if original_bytes == 0 {
        1.0
    } else {
        compacted_bytes as f64 / original_bytes as f64
    };

    let applied = ratio <= MIN_COMPACT_RATIO && compacted_bytes < original_bytes;

    if applied {
        log::info!(
            "[tokenjuice] compacted tool={} rule={} {}->{} bytes (ratio={:.2})",
            tool_name,
            rule_id,
            original_bytes,
            compacted_bytes,
            ratio
        );
        (
            result.inline_text,
            CompactionStats {
                tool_name: tool_name.to_owned(),
                original_bytes,
                compacted_bytes,
                rule_id,
                applied: true,
            },
        )
    } else {
        log::debug!(
            "[tokenjuice] pass-through tool={} rule={} {}->{} bytes (ratio={:.2} > {})",
            tool_name,
            rule_id,
            original_bytes,
            compacted_bytes,
            ratio,
            MIN_COMPACT_RATIO
        );
        (
            output.to_owned(),
            CompactionStats {
                tool_name: tool_name.to_owned(),
                original_bytes,
                compacted_bytes: original_bytes,
                rule_id,
                applied: false,
            },
        )
    }
}

/// Derive `(command, argv)` from a tool's JSON arguments.
///
/// Handles the common shapes:
/// * `{"command": "git status"}` — string command (whitespace-split into argv).
/// * `{"command": "git", "args": ["status"]}` — explicit split.
/// * `{"argv": ["git", "status"]}` — pre-built argv.
/// * `{"cmd": "..."}` — alternate field name.
///
/// Returns `(None, None)` when the arguments don't look shell-like.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn skips_short_output() {
        let (out, stats) = compact_tool_output("shell", None, "hello world", Some(0));
        assert_eq!(out, "hello world");
        assert!(!stats.applied);
        assert_eq!(stats.rule_id, "none/too-small");
        assert_eq!(stats.original_bytes, 11);
    }

    #[test]
    fn compacts_long_git_status_via_argv() {
        let mut lines = vec!["On branch main".to_owned()];
        for i in 0..200 {
            lines.push(format!("\tmodified:   src/file_{i}.rs"));
        }
        let output = lines.join("\n");
        let args = json!({"command": "git status"});
        let (compacted, stats) = compact_tool_output("shell", Some(&args), &output, Some(0));
        assert!(stats.applied, "expected compaction, got {:?}", stats);
        assert!(compacted.len() < output.len());
        assert!(stats.rule_id.starts_with("git/"));
    }

    #[test]
    fn passes_through_incompressible_output() {
        let unique_lines: Vec<String> = (0..200)
            .map(|i| format!("unique-payload-chunk-{i}-{}", "x".repeat(30)))
            .collect();
        let output = unique_lines.join("\n");
        let (returned, stats) = compact_tool_output("unknown_tool", None, &output, Some(0));
        // Either the fallback rule compacted it (applied == true) or it
        // passed through because ratio > threshold. Both are valid; we only
        // assert the function never loses data silently.
        if stats.applied {
            assert_ne!(returned, output);
            assert!(stats.compacted_bytes < stats.original_bytes);
        } else {
            assert_eq!(returned, output);
        }
    }

    #[test]
    fn extract_argv_handles_common_shapes() {
        let (cmd, argv) = extract_command_argv(Some(&json!({"command": "git status"})));
        assert_eq!(cmd.as_deref(), Some("git status"));
        assert_eq!(argv.unwrap(), vec!["git", "status"]);

        let (cmd, argv) = extract_command_argv(Some(&json!({
            "command": "cargo",
            "args": ["test", "--lib"],
        })));
        assert_eq!(cmd.as_deref(), Some("cargo test --lib"));
        assert_eq!(argv.unwrap(), vec!["cargo", "test", "--lib"]);

        let (cmd, argv) = extract_command_argv(Some(&json!({
            "argv": ["npm", "install"],
        })));
        assert_eq!(cmd.as_deref(), Some("npm install"));
        assert_eq!(argv.unwrap(), vec!["npm", "install"]);

        let (cmd, argv) = extract_command_argv(Some(&json!({"unrelated": 1})));
        assert!(cmd.is_none());
        assert!(argv.is_none());

        let (cmd, argv) = extract_command_argv(None);
        assert!(cmd.is_none());
        assert!(argv.is_none());
    }

    #[test]
    fn ratio_computation() {
        let stats = CompactionStats {
            tool_name: "x".into(),
            original_bytes: 1000,
            compacted_bytes: 250,
            rule_id: "r".into(),
            applied: true,
        };
        assert!((stats.ratio() - 0.25).abs() < 1e-9);

        let empty = CompactionStats {
            tool_name: "x".into(),
            original_bytes: 0,
            compacted_bytes: 0,
            rule_id: "r".into(),
            applied: false,
        };
        assert!((empty.ratio() - 1.0).abs() < 1e-9);
    }
}
