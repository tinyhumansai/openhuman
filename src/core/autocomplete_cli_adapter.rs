//! Autocomplete-specific CLI adapter.
//!
//! Keeps autocomplete-only argument handling out of the generic core CLI.

use anyhow::Result;

use crate::core::logging::CliLogDefault;
use crate::openhuman::autocomplete::ops::{autocomplete_start_cli, AutocompleteStartCliOptions};

pub struct NamespacePreparse {
    pub args: Vec<String>,
    pub init_logging: Option<(bool, CliLogDefault)>,
}

/// Extract only *leading* global verbose flags so parameter values remain intact.
/// Returns `(verbose, remaining_args)`.
fn extract_leading_verbose_flags(args: &[String]) -> (bool, Vec<String>) {
    let mut verbose = false;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "-v" | "--verbose" => {
                verbose = true;
                index += 1;
            }
            _ => break,
        }
    }
    (verbose, args[index..].to_vec())
}

pub fn preparse_namespace(namespace: &str, args: &[String]) -> NamespacePreparse {
    if namespace != "autocomplete" {
        return NamespacePreparse {
            args: args.to_vec(),
            init_logging: None,
        };
    }

    let (verbose, remaining) = extract_leading_verbose_flags(args);
    NamespacePreparse {
        args: remaining,
        init_logging: Some((verbose, CliLogDefault::AutocompleteOnly)),
    }
}

pub fn parse_run_scope_flag(flag: &str) -> Option<CliLogDefault> {
    if flag == "--autocomplete-logs" {
        Some(CliLogDefault::AutocompleteOnly)
    } else {
        None
    }
}

pub fn print_run_scope_help_line() {
    println!(
        "  --autocomplete-logs  When RUST_LOG is unset: stderr shows only inline-autocomplete logs"
    );
}

pub fn maybe_print_namespace_help_footer(namespace: &str) {
    if namespace == "autocomplete" {
        println!(
            "Logging: stderr is autocomplete-only by default (unless RUST_LOG is set); add -v for trace."
        );
    }
}

pub fn maybe_print_start_help(namespace: &str, function: &str) -> bool {
    if namespace == "autocomplete" && function == "start" {
        print_autocomplete_start_help();
        true
    } else {
        false
    }
}

pub fn maybe_handle_namespace_start(
    namespace: &str,
    function: &str,
    args: &[String],
) -> Result<Option<serde_json::Value>> {
    if namespace != "autocomplete" || function != "start" {
        return Ok(None);
    }

    let cli_options = parse_autocomplete_start_cli_options(args)?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let value = rt
        .block_on(async { autocomplete_start_cli(cli_options).await })
        .map_err(anyhow::Error::msg)?;
    Ok(Some(value))
}

/// Parses CLI options specific to the `autocomplete start` command.
fn parse_autocomplete_start_cli_options(args: &[String]) -> Result<AutocompleteStartCliOptions> {
    let mut debounce_ms: Option<u64> = None;
    let mut serve = false;
    let mut spawn = false;
    let mut i = 0usize;

    while i < args.len() {
        match args[i].as_str() {
            "--debounce-ms" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --debounce-ms"))?;
                debounce_ms = Some(
                    raw.parse::<u64>()
                        .map_err(|e| anyhow::anyhow!("invalid --debounce-ms: {e}"))?,
                );
                i += 2;
            }
            "--serve" => {
                serve = true;
                i += 1;
            }
            "--spawn" => {
                spawn = true;
                i += 1;
            }
            other => return Err(anyhow::anyhow!("unknown autocomplete start arg: {other}")),
        }
    }

    if serve && spawn {
        return Err(anyhow::anyhow!(
            "--serve and --spawn are mutually exclusive"
        ));
    }

    Ok(AutocompleteStartCliOptions {
        debounce_ms,
        serve,
        spawn,
    })
}

/// Prints help information for the `autocomplete start` command.
fn print_autocomplete_start_help() {
    println!("Usage: openhuman autocomplete start [--debounce-ms <u64>] [--serve|--spawn]");
    println!();
    println!("  --debounce-ms <u64>  Override debounce in milliseconds.");
    println!("  --serve              Run autocomplete loop in the current foreground process.");
    println!("  --spawn              Spawn autocomplete loop as a background process.");
}

#[cfg(test)]
mod tests {
    use super::parse_autocomplete_start_cli_options;

    #[test]
    fn parse_autocomplete_start_cli_options_rejects_serve_and_spawn() {
        let args = vec!["--serve".to_string(), "--spawn".to_string()];
        let err = parse_autocomplete_start_cli_options(&args)
            .expect_err("must reject mutually exclusive flags");
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn extract_leading_verbose_flags_preserves_param_like_values() {
        let args = vec![
            "-v".to_string(),
            "set_style".to_string(),
            "--style-instructions".to_string(),
            "--verbose".to_string(),
        ];
        let (verbose, remaining) = super::extract_leading_verbose_flags(&args);
        assert!(verbose);
        assert_eq!(
            remaining,
            vec![
                "set_style".to_string(),
                "--style-instructions".to_string(),
                "--verbose".to_string()
            ]
        );
    }
}
