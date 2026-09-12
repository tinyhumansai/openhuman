use super::{
    grouped_schemas, load_dotenv_for_cli, parse_function_params, parse_input_value,
    parse_launch_options, should_auto_launch_tui,
};
use crate::core::types::HostKind;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use tempfile::tempdir;

#[test]
fn bare_cli_auto_launches_tui_only_for_interactive_non_container_hosts() {
    let none: Vec<String> = vec![];
    assert!(should_auto_launch_tui(
        &none,
        true,
        true,
        HostKind::Cli,
        true
    ));
    assert!(!should_auto_launch_tui(
        &none,
        false,
        true,
        HostKind::Cli,
        true
    ));
    assert!(!should_auto_launch_tui(
        &none,
        true,
        false,
        HostKind::Cli,
        true
    ));
    assert!(!should_auto_launch_tui(
        &none,
        true,
        true,
        HostKind::Docker,
        true
    ));
    assert!(!should_auto_launch_tui(
        &none,
        true,
        true,
        HostKind::Cli,
        false
    ));
}

#[test]
fn explicit_args_never_trigger_bare_cli_auto_launch() {
    for args in [
        vec!["--no-tui".to_string()],
        vec!["run".to_string()],
        vec!["tui".to_string()],
    ] {
        assert!(!should_auto_launch_tui(
            &args,
            true,
            true,
            HostKind::Cli,
            true
        ));
    }
}

#[test]
fn launch_options_parse_model_provider_and_no_tui_before_command() {
    let args = vec![
        "--no-tui".to_string(),
        "--model".to_string(),
        "qwen3:8b".to_string(),
        "--provider-id=ollama".to_string(),
        "run".to_string(),
        "--jsonrpc-only".to_string(),
    ];
    let parsed = parse_launch_options(&args).expect("global options");
    assert!(parsed.no_tui);
    assert_eq!(parsed.model.as_deref(), Some("qwen3:8b"));
    assert_eq!(parsed.provider.as_deref(), Some("ollama"));
    assert_eq!(parsed.args, ["run", "--jsonrpc-only"]);
}

#[test]
fn launch_options_support_short_flags_and_last_value_wins() {
    let args = ["-m", "old", "--model=new", "-p", "p_openai", "tui"].map(str::to_string);
    let parsed = parse_launch_options(&args).expect("global options");
    assert_eq!(parsed.model.as_deref(), Some("new"));
    assert_eq!(parsed.provider.as_deref(), Some("p_openai"));
    assert_eq!(parsed.args, ["tui"]);
}

#[test]
fn launch_options_stop_at_the_subcommand() {
    let args = ["inference", "list_models", "--provider", "openai"].map(str::to_string);
    let parsed = parse_launch_options(&args).expect("global options");
    assert_eq!(parsed.args, args);
    assert_eq!(parsed.provider, None);
}

#[test]
fn launch_options_reject_missing_or_empty_values() {
    for args in [
        vec!["--model".to_string()],
        vec!["--provider=".to_string()],
        vec![
            "--model".to_string(),
            "--provider".to_string(),
            "ollama".to_string(),
        ],
    ] {
        assert!(parse_launch_options(&args).is_err());
    }
}

/// Serialises env-mutating CLI tests via the crate-wide backend env lock —
/// these tests set `BACKEND_URL`, which `api::config` and `medulla::ops`
/// tests also read/remove, so a module-local lock is not enough.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::api::config::backend_env_test_lock()
}

#[test]
fn grouped_schemas_contains_migrated_namespaces() {
    let grouped = grouped_schemas();
    assert!(grouped.contains_key("health"));
    assert!(grouped.contains_key("doctor"));
    assert!(grouped.contains_key("encrypt"));
    assert!(grouped.contains_key("decrypt"));
    assert!(grouped.contains_key("config"));
    assert!(grouped.contains_key("auth"));
    assert!(grouped.contains_key("service"));
    assert!(grouped.contains_key("migrate"));
    assert!(grouped.contains_key("inference"));
}

#[test]
fn parse_function_params_rejects_unknown_param() {
    let schema = ControllerSchema {
        namespace: "test",
        function: "echo",
        description: "test schema",
        inputs: vec![FieldSchema {
            name: "message",
            ty: TypeSchema::String,
            required: true,
            comment: "message text",
        }],
        outputs: vec![FieldSchema {
            name: "result",
            ty: TypeSchema::String,
            required: true,
            comment: "echo response",
        }],
    };
    let args = vec!["--unknown".to_string(), "value".to_string()];
    let err = parse_function_params(&schema, &args).expect_err("unknown param should fail");
    assert!(err.contains("unknown param"));
}

#[test]
fn parse_function_params_rejects_flag_like_missing_value() {
    let schema = ControllerSchema {
        namespace: "test",
        function: "configure",
        description: "test schema",
        inputs: vec![
            FieldSchema {
                name: "enabled",
                ty: TypeSchema::Bool,
                required: true,
                comment: "whether the feature is enabled",
            },
            FieldSchema {
                name: "name",
                ty: TypeSchema::String,
                required: true,
                comment: "feature name",
            },
        ],
        outputs: vec![],
    };
    let args = vec![
        "--enabled".to_string(),
        "--name".to_string(),
        "demo".to_string(),
    ];
    let err = parse_function_params(&schema, &args).expect_err("missing value should fail");
    assert_eq!(err, "missing value for --enabled");
}

#[test]
fn parse_input_value_rejects_invalid_bool() {
    let err =
        parse_input_value(&TypeSchema::Bool, "not-a-bool").expect_err("invalid bool should fail");
    assert!(err.contains("expected bool"));
}

#[test]
fn load_dotenv_for_cli_reads_cwd_dotenv_without_overwriting_existing_env() {
    let _guard = env_lock();
    let tmp = tempdir().expect("tempdir");
    let env_path = tmp.path().join(".env");
    std::fs::write(
        &env_path,
        "BACKEND_URL=https://staging-api.example.test\nOPENHUMAN_APP_ENV=staging\n",
    )
    .expect("write .env");

    let original_dir = std::env::current_dir().expect("current dir");
    let prior_backend = std::env::var("BACKEND_URL").ok();
    let prior_app_env = std::env::var("OPENHUMAN_APP_ENV").ok();
    let prior_dotenv_path = std::env::var("OPENHUMAN_DOTENV_PATH").ok();

    unsafe {
        std::env::remove_var("BACKEND_URL");
        std::env::set_var("OPENHUMAN_APP_ENV", "production");
        std::env::remove_var("OPENHUMAN_DOTENV_PATH");
    }
    std::env::set_current_dir(tmp.path()).expect("set current dir");

    let result = load_dotenv_for_cli();

    let loaded_backend = std::env::var("BACKEND_URL").ok();
    let loaded_app_env = std::env::var("OPENHUMAN_APP_ENV").ok();

    std::env::set_current_dir(&original_dir).expect("restore current dir");
    unsafe {
        match prior_backend {
            Some(value) => std::env::set_var("BACKEND_URL", value),
            None => std::env::remove_var("BACKEND_URL"),
        }
        match prior_app_env {
            Some(value) => std::env::set_var("OPENHUMAN_APP_ENV", value),
            None => std::env::remove_var("OPENHUMAN_APP_ENV"),
        }
        match prior_dotenv_path {
            Some(value) => std::env::set_var("OPENHUMAN_DOTENV_PATH", value),
            None => std::env::remove_var("OPENHUMAN_DOTENV_PATH"),
        }
    }

    result.expect("dotenv load should succeed");
    assert_eq!(
        loaded_backend.as_deref(),
        Some("https://staging-api.example.test")
    );
    assert_eq!(loaded_app_env.as_deref(), Some("production"));
}

// --- `mcp` compile-time gate (#4799) ------------------------------------

/// With the `mcp` feature compiled out, `openhuman mcp` must fail with a
/// diagnostic that names the BUILD as the cause — not a generic
/// "unknown namespace" error.
///
/// Why this matters enough to test: the naive way to gate the CLI is to delete
/// the `"mcp" | "mcp-server"` match arm. That is WRONG — `mcp` would fall
/// through to generic namespace resolution and die with `unknown namespace:
/// mcp`, which reads like the user typo'd a command rather than like a
/// property of this build. Instead `cli.rs` is untouched and the arm resolves
/// to `mcp::server::stub::run_stdio_from_cli`, which bails with the message
/// asserted below. An MCP host (Claude Desktop, Cursor, …) spawning
/// `openhuman mcp` therefore gets a non-zero exit + a one-line reason on
/// stderr instead of hanging on stdout that never speaks JSON-RPC.
#[test]
#[cfg(not(feature = "mcp"))]
fn mcp_subcommand_reports_disabled_build_when_gate_off() {
    let _guard = env_lock();

    let err = crate::core::cli::run_from_cli_args(&["mcp".to_string()])
        .expect_err("`openhuman mcp` must fail when the `mcp` feature is compiled out");
    let msg = err.to_string();

    assert!(
        msg.contains("mcp feature disabled"),
        "error must name the compile-time gate as the cause; got: {msg}"
    );
    assert!(
        msg.contains("--features mcp"),
        "error must tell the user how to get a working build; got: {msg}"
    );
    assert!(
        !msg.contains("unknown namespace"),
        "must NOT degrade into generic namespace resolution — that reads like a typo, \
         not a build fact; got: {msg}"
    );
}

/// The `mcp-server` alias must behave identically to `mcp` — both arms route
/// to the same stub, so neither can silently regress into the fall-through.
#[test]
#[cfg(not(feature = "mcp"))]
fn mcp_server_alias_reports_disabled_build_when_gate_off() {
    let _guard = env_lock();

    let err = crate::core::cli::run_from_cli_args(&["mcp-server".to_string()])
        .expect_err("`openhuman mcp-server` must fail when the `mcp` feature is compiled out");

    assert!(
        err.to_string().contains("mcp feature disabled"),
        "the `mcp-server` alias must give the same build-fact diagnostic as `mcp`"
    );
}

// --- `tui` compile-time gate --------------------------------------------

/// With the `tui` feature compiled out, `openhuman tui` must fail with a
/// diagnostic that names the BUILD as the cause — not a generic
/// "unknown namespace" error.
///
/// Same reasoning as the `mcp` gate test above: the naive way to gate the CLI
/// is to delete the `"tui" | "chat"` match arm, which is WRONG — `tui` would
/// fall through to generic namespace resolution and die with `unknown
/// namespace: tui`, reading like a user typo. Instead `cli.rs` is untouched and
/// the arm resolves to the CLI-local disabled-feature dispatcher, which bails
/// with the message asserted below.
#[test]
#[cfg(not(feature = "tui"))]
fn tui_subcommand_reports_disabled_build_when_gate_off() {
    let _guard = env_lock();

    let err = crate::core::cli::run_from_cli_args(&["tui".to_string()])
        .expect_err("`openhuman tui` must fail when the `tui` feature is compiled out");
    let msg = err.to_string();

    assert!(
        msg.contains("tui feature disabled"),
        "error must name the compile-time gate as the cause; got: {msg}"
    );
    assert!(
        msg.contains("--features tui"),
        "error must tell the user how to get a working build; got: {msg}"
    );
    assert!(
        !msg.contains("unknown namespace"),
        "must NOT degrade into generic namespace resolution — that reads like a typo, \
         not a build fact; got: {msg}"
    );
}

/// The `chat` alias must behave identically to `tui` — both names route to the
/// same dispatcher, so neither can silently regress into the fall-through.
#[test]
#[cfg(not(feature = "tui"))]
fn chat_alias_reports_disabled_build_when_gate_off() {
    let _guard = env_lock();

    let err = crate::core::cli::run_from_cli_args(&["chat".to_string()])
        .expect_err("`openhuman chat` must fail when the `tui` feature is compiled out");

    assert!(
        err.to_string().contains("tui feature disabled"),
        "the `chat` alias must give the same build-fact diagnostic as `tui`"
    );
}

// --- the capability gate on the generic namespace path -----------------------
//
// Driven through the pure helpers plus a directly-resolved capability set,
// rather than `run_from_cli_args`: reaching a narrowed set end-to-end needs
// `driver = "null"` in a real `config.toml` under a process-global
// `OPENHUMAN_WORKSPACE`, i.e. env mutation plus disk writes. Same reasoning
// recorded in the M5.4 block of `all_tests.rs`.

use crate::core::all::{
    capability_for_parts, capability_for_rpc_method, sole_capability_for_namespace,
};
use crate::core::cli_capability::capability_verdict;
use tinymemory_api::capabilities::Capabilities;

#[test]
fn capability_gated_namespace_reports_a_config_fact_not_a_typo() {
    let required = sole_capability_for_namespace("memory_tree");
    assert!(required.is_some(), "memory_tree must be a gated namespace");
    let err = capability_verdict(
        "null",
        Capabilities::mandatory(),
        required,
        "openhuman memory_tree",
    )
    .expect_err("the null driver does not advertise `tree`");
    let msg = err.to_string();
    assert!(msg.contains("null"), "{msg}");
    assert!(msg.contains("tree"), "{msg}");
    assert!(!msg.contains("unknown namespace"), "{msg}");
}

#[test]
fn capability_gated_function_reports_a_config_fact_not_a_typo() {
    let required = capability_for_parts("memory", "doc_ingest").flatten();
    let err = capability_verdict(
        "null",
        Capabilities::mandatory(),
        required,
        "openhuman memory doc_ingest",
    )
    .expect_err("the null driver does not advertise `ingest`");
    let msg = err.to_string();
    assert!(msg.contains("ingest"), "{msg}");
    assert!(!msg.contains("unknown function"), "{msg}");
}

#[test]
fn capability_gated_rpc_method_reports_its_family_unfiltered() {
    assert_eq!(
        capability_for_rpc_method("openhuman.memory_tree_wipe_all"),
        Some(Some(tinymemory_api::capabilities::Capability::Tree))
    );
}

/// A real typo must stay a typo — the gate never fires for it, because the
/// unfiltered lookup finds no controller to name a family for.
#[test]
fn unknown_namespace_still_reports_unknown_namespace() {
    let err = super::run_namespace_command(
        "definitely_not_a_namespace",
        &["x".to_string()],
        &grouped_schemas(),
    )
    .expect_err("an unknown namespace must error");
    assert!(err.to_string().contains("unknown namespace"), "{err}");
}

#[test]
fn unknown_function_in_a_live_namespace_still_reports_unknown_function() {
    let grouped = grouped_schemas();
    let namespace = grouped
        .keys()
        .next()
        .expect("at least one namespace is registered")
        .clone();
    let err = super::run_namespace_command(
        &namespace,
        &["definitely_not_a_function".to_string()],
        &grouped,
    )
    .expect_err("an unknown function must error");
    assert!(err.to_string().contains("unknown function"), "{err}");
}

/// With no ambient context nothing is filtered, so the CLI's namespace list is
/// exactly what it was before the gate existed.
#[test]
fn default_build_leaves_the_generic_namespace_path_unchanged() {
    let grouped = grouped_schemas();
    for ns in ["memory", "memory_tree", "memory_goals"] {
        assert!(grouped.contains_key(ns), "`{ns}` must still be listed");
    }
    // `memory_diff` was removed with the `memory-git` gate; it must not come
    // back as a listed namespace.
    assert!(
        !grouped.contains_key("memory_diff"),
        "`memory_diff` was removed and must not be listed"
    );
}

/// The gate must fire on the path a user actually takes.
///
/// This drives `run_namespace_command` itself rather than the pure
/// `capability_verdict` helper, because the two disagreed once: the check
/// originally sat in the not-found arm, which is unreachable on a plain CLI
/// invocation (no ambient `CoreContext` ⇒ `grouped_schemas()` is unfiltered ⇒
/// the gated function is still *found*). Every helper-level test passed while
/// the real command ran to completion under a driver that does not advertise
/// the family. Assert through the entry point or this regresses silently.
#[test]
fn generic_namespace_path_reports_the_config_fact_under_a_driver_without_the_family() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace = tempdir().expect("temp workspace");

    // SAFETY: serialised by TEST_ENV_LOCK, and both vars are restored below.
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace.path());
    std::env::set_var("OPENHUMAN_MEMORY_DRIVER", "null");

    let err = super::run_namespace_command(
        "memory_tree",
        &["list_chunks".to_string()],
        &grouped_schemas(),
    )
    .expect_err("`tree` is not advertised by the null driver, so this must not run");

    std::env::remove_var("OPENHUMAN_MEMORY_DRIVER");
    std::env::remove_var("OPENHUMAN_WORKSPACE");

    let message = err.to_string();
    assert!(
        message.starts_with(crate::core::cli_capability::CAPABILITY_UNAVAILABLE_PREFIX),
        "must read as a configuration fact, not an unknown-command error: {message}"
    );
    assert!(
        message.contains("null") && message.contains("tree"),
        "must name the bound driver and the missing family: {message}"
    );
    assert!(
        !message.contains("unknown"),
        "a gated command is not a typo and must not read like one: {message}"
    );
}

#[test]
fn raw_call_path_rejects_a_method_the_bound_driver_does_not_advertise() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let workspace = tempdir().expect("temp workspace");

    // SAFETY: serialised by TEST_ENV_LOCK, and both vars are restored below.
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace.path());
    std::env::set_var("OPENHUMAN_MEMORY_DRIVER", "null");

    let err = super::run_call_command(&[
        "--method".to_string(),
        "openhuman.memory_tree_wipe_all".to_string(),
    ])
    .expect_err("the null driver must not dispatch a tree wipe");

    std::env::remove_var("OPENHUMAN_MEMORY_DRIVER");
    std::env::remove_var("OPENHUMAN_WORKSPACE");

    let message = err.to_string();
    assert!(
        message.starts_with(crate::core::cli_capability::CAPABILITY_UNAVAILABLE_PREFIX),
        "must reject before dispatching: {message}"
    );
    assert!(
        message.contains("null") && message.contains("tree"),
        "{message}"
    );
}
