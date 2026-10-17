//! RPC adapters for the `http_host` domain.

use crate::core_host::core::Outcome;
use crate::http_host::ops;
use crate::http_host::types::{
    HostedDirGetResult, HostedDirListResult, HostedDirLookupParams, HostedDirStartResult,
    HostedDirStopResult, StartHostedDirParams,
};

pub async fn start(
    ctx: &crate::core_host::core::runtime::DispatchCtx,
    params: StartHostedDirParams,
) -> Result<Outcome<HostedDirStartResult>, String> {
    let server = ops::start_hosted_dir_server(ctx, params).await?;
    Ok(Outcome::single_log(
        HostedDirStartResult { server },
        "started hosted directory HTTP server",
    ))
}

pub async fn stop(params: HostedDirLookupParams) -> Result<Outcome<HostedDirStopResult>, String> {
    let server = ops::stop_hosted_dir_server(&params.server_id).await?;
    Ok(Outcome::single_log(
        HostedDirStopResult {
            stopped: true,
            server,
        },
        "stopped hosted directory HTTP server",
    ))
}

pub async fn get(params: HostedDirLookupParams) -> Result<Outcome<HostedDirGetResult>, String> {
    let server = ops::get_hosted_dir_server(&params.server_id)?;
    Ok(Outcome::new(HostedDirGetResult { server }, vec![]))
}

pub async fn list() -> Result<Outcome<HostedDirListResult>, String> {
    let servers = ops::list_hosted_dir_servers()?;
    Ok(Outcome::new(HostedDirListResult { servers }, vec![]))
}
