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

/// The slot table for a standalone CLI invocation.
///
/// [`subsystems_status`] resolves memory through [`memory_subsystem_status`],
/// which now handles the no-`CoreContext` standalone case itself by reading the
/// on-disk config and binding the configured workspace's driver (see
/// `memory::ops::provider::standalone_status`) — the same way
/// `cli_capability::bound_memory_driver_for` does. So the bare table and the
/// `subsystems status` JSON path both render the same resolved row, on the same
/// code, and neither reports an unresolved `driver = ""` row on a healthy
/// install.
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
mod tests {
    use super::*;

    #[test]
    fn help_flag_short_circuits_without_probing_a_driver() {
        run_subsystems_command(&["--help".to_string()]).expect("help succeeds");
        run_subsystems_command(&["-h".to_string()]).expect("short help succeeds");
    }

    #[test]
    fn bare_invocation_renders_the_table() {
        run_subsystems_command(&[]).expect("table renders");
    }

    #[test]
    fn namespace_has_a_registered_cli_adapter() {
        assert!(
            crate::core::all::cli_handler_for_namespace("subsystems").is_some(),
            "bare `openhuman subsystems` must reach the table, not the generic help"
        );
    }
}
