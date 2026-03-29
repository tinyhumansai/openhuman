//! JSON-RPC / CLI controller surface for data migration.

use std::path::PathBuf;

use crate::openhuman::config::Config;
use crate::openhuman::migration::{self, MigrationReport};
use crate::rpc::RpcOutcome;

pub async fn migrate_openclaw(
    config: &Config,
    source_workspace: Option<PathBuf>,
    dry_run: bool,
) -> Result<RpcOutcome<MigrationReport>, String> {
    let report = migration::migrate_openclaw_memory(config, source_workspace, dry_run)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(report, "migration completed"))
}
