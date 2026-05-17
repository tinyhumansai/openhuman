//! RPC adapters for the `http_host` domain.

use crate::openhuman::http_host::ops;
use crate::openhuman::http_host::types::{
    HostedDirGetResult, HostedDirListResult, HostedDirLookupParams, HostedDirStartResult,
    HostedDirStopResult, StartHostedDirParams,
};
use crate::rpc::RpcOutcome;

pub async fn start(
    params: StartHostedDirParams,
) -> Result<RpcOutcome<HostedDirStartResult>, String> {
    let server = ops::start_hosted_dir_server(params).await?;
    Ok(RpcOutcome::single_log(
        HostedDirStartResult { server },
        "started hosted directory HTTP server",
    ))
}

pub async fn stop(
    params: HostedDirLookupParams,
) -> Result<RpcOutcome<HostedDirStopResult>, String> {
    let server = ops::stop_hosted_dir_server(&params.server_id).await?;
    Ok(RpcOutcome::single_log(
        HostedDirStopResult {
            stopped: true,
            server,
        },
        "stopped hosted directory HTTP server",
    ))
}

pub async fn get(params: HostedDirLookupParams) -> Result<RpcOutcome<HostedDirGetResult>, String> {
    let server = ops::get_hosted_dir_server(&params.server_id)?;
    Ok(RpcOutcome::new(HostedDirGetResult { server }, vec![]))
}

pub async fn list() -> Result<RpcOutcome<HostedDirListResult>, String> {
    let servers = ops::list_hosted_dir_servers()?;
    Ok(RpcOutcome::new(HostedDirListResult { servers }, vec![]))
}
