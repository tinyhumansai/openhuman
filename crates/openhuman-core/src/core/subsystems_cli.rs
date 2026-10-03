//! `openhuman subsystems` — the human-readable subsystem slot table.
//!
//! Reached through the `RegisteredCliAdapter` seam
//! ([`crate::core::all::cli_handler_for_namespace`]), which
//! `run_namespace_command` consults when the namespace is invoked with no
//! function or with `--help`. `openhuman subsystems status` bypasses this and
//! prints the raw JSON through the generic namespace dispatcher, so there is
//! no hand-written subcommand match arm anywhere — registering the controller
//! is what makes the subcommand exist.
//!
//! ```text
//! openhuman subsystems           # table
//! openhuman subsystems status    # JSON
//! ```

use anyhow::Result;

use crate::core::subsystem::{subsystems_status, SubsystemStatus};

pub fn run_subsystems_command(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }

    // A current-thread runtime is enough: a status call probes a bound driver's
    // health and touches no orchestrator, unlike the generic dispatcher's
    // multi-thread runtime with an enlarged agent worker stack.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let rows = rt.block_on(cli_subsystems_status());

    println!(
        "{:<10} {:<14} {:<9} {:<9} {:<9} CAPABILITIES",
        "SLOT", "DRIVER", "CLASS", "HEALTH", "CONTRACT"
    );
    for row in &rows {
        let driver = if row.driver.is_empty() {
            "-"
        } else {
            row.driver.as_str()
        };
        let capabilities = if row.capabilities.is_empty() {
            "-".to_string()
        } else {
            row.capabilities.join(",")
        };
        println!(
            "{:<10} {:<14} {:<9} {:<9} {:<9} {}",
            row.slot, driver, row.class, row.health, row.contract_version, capabilities
        );
        if let Some(reason) = &row.health_reason {
            println!("  health: {reason}");
        }
        if let Some(previous) = &row.fell_back_from {
            println!("  fell back from: {previous}");
        }
        if let Some(err) = &row.last_error {
            println!("  last error: {err}");
        }
    }
    Ok(())
}

/// The slot table for a standalone CLI invocation: the same rows the
/// `subsystems status` RPC returns, resolved from the on-disk config.
async fn cli_subsystems_status() -> Vec<SubsystemStatus> {
    subsystems_status().await
}

fn print_help() {
    println!("openhuman subsystems — kernel subsystem slots and their bound drivers");
    println!();
    println!("USAGE:");
    println!("  openhuman subsystems           Print the slot table");
    println!("  openhuman subsystems status    Print the same data as JSON");
}

#[cfg(test)]
#[path = "subsystems_cli_tests.rs"]
mod tests;
