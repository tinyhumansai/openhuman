//! Read-only execution for the legacy `doctor report` and `doctor models` CLI
//! namespace methods.

use anyhow::{anyhow, Result};
use serde_json::{Map, Value};

#[derive(Debug, PartialEq)]
struct Invocation {
    method: String,
    params: Map<String, Value>,
}

/// Run one legacy doctor namespace command in the inspection runtime.
///
/// `None` leaves unrelated commands and help requests to the general CLI
/// dispatcher. A matching inspection invocation returns its raw controller
/// JSON shape, without opening the persistent session store or entering the
/// normal CLI boot path.
pub(super) fn try_run(args: &[String]) -> Option<Result<()>> {
    match parse(args) {
        Ok(Some(invocation)) => Some(invoke(invocation)),
        Ok(None) => None,
        Err(error) => Some(Err(error)),
    }
}

fn parse(args: &[String]) -> Result<Option<Invocation>> {
    if !args
        .first()
        .is_some_and(|arg| is_doctor_or_launch_option(arg))
    {
        return Ok(None);
    }

    // This pure parser is shared with the CLI dispatcher. The inspection path
    // accepts the familiar leading flags but deliberately does not apply their
    // transient model/provider overrides to the selected profile.
    let command = match crate::embed::cli::parse_cli_args_after_launch_options(args) {
        Ok(command) => command,
        Err(_) => return Ok(None),
    };
    if command.first().map(String::as_str) != Some("doctor") {
        return Ok(None);
    }

    let Some(function) = command.get(1).map(String::as_str) else {
        return Ok(None);
    };
    if !matches!(function, "report" | "models") {
        return Ok(None);
    }
    // Core's namespace dispatcher prints these function-help forms before it
    // parses parameters. Let that path retain its existing help text.
    if command
        .get(2)
        .is_some_and(|arg| matches!(arg.as_str(), "-h" | "--help" | "help"))
    {
        return Ok(None);
    }

    let method = format!("openhuman.doctor_{function}");
    let schema = crate::embed::schema_for_rpc_method(&method)
        .ok_or_else(|| anyhow!("unregistered doctor controller '{function}'"))?;
    let params = crate::embed::cli::parse_function_params(&schema, &command[2..])
        .map_err(anyhow::Error::msg)?;
    Ok(Some(Invocation { method, params }))
}

fn is_doctor_or_launch_option(arg: &str) -> bool {
    matches!(
        arg,
        "doctor" | "--model" | "--model-id" | "-m" | "--provider" | "--provider-id" | "-p"
    ) || arg.starts_with("--model=")
        || arg.starts_with("--model-id=")
        || arg.starts_with("--provider=")
        || arg.starts_with("--provider-id=")
}

fn invoke(invocation: Invocation) -> Result<()> {
    let selected_profile = invocation
        .params
        .get("profile")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let tokio = crate::embed::process::tokio_runtime()?;
    let runtime = tokio.block_on(super::doctor_runtime(selected_profile.as_deref()))?;
    let result = tokio
        .block_on(
            runtime
                .core_runtime()
                .invoke(&invocation.method, Value::Object(invocation.params)),
        )
        .map_err(anyhow::Error::msg)?;
    println!("{}", format_json(&result)?);
    Ok(())
}

fn format_json(value: &Value) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

#[cfg(test)]
#[path = "doctor_cli_tests.rs"]
mod tests;
