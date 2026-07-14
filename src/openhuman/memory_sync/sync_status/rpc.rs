//! OpenHuman RPC shell for tinycortex synchronization status.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::types::StatusListResponse;

pub async fn status_list_rpc(config: &Config) -> Result<RpcOutcome<StatusListResponse>, String> {
    tracing::debug!("[memory_sync_status][rpc] status_list via tinycortex");
    let memory_config =
        crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone());
    let statuses = match tokio::task::spawn_blocking(move || {
        tinycortex::memory::sync::list_sync_statuses(&memory_config)
    })
    .await
    {
        Ok(Ok(statuses)) => statuses,
        Ok(Err(error)) => {
            tracing::warn!(%error, "[memory_sync_status][rpc] tinycortex status query failed");
            Vec::new()
        }
        Err(error) => {
            tracing::warn!(%error, "[memory_sync_status][rpc] status task join failed");
            Vec::new()
        }
    };
    Ok(RpcOutcome::new(StatusListResponse { statuses }, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_keeps_top_level_statuses_array() {
        let value = serde_json::to_value(StatusListResponse {
            statuses: Vec::new(),
        })
        .unwrap();
        assert!(value
            .get("statuses")
            .is_some_and(serde_json::Value::is_array));
    }
}
