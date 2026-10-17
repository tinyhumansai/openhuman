//! Host calls to the TinyBox sandbox module.
//!
//! This adapter owns only OpenHuman's module loading and bus boundary. Request
//! and response types come from `tinybox-bus`; isolation, process ownership,
//! file access, forwarding, and output collection stay in TinyBox.

use std::time::Duration;

use serde::{de::DeserializeOwned, Serialize};
pub use tinybox_bus as contract;

use crate::config::Config;

use tinybus::module::{ModuleCallError, ModuleClient};

/// Registry id for the TinyBox module.
pub const MODULE_ID: &str = "tinybox";

const MODULE_CALL_TIMEOUT: Duration = Duration::from_secs(300);

// `tinybox_bus::names` carries one named constant per bus member; the host
// never indexes `METHODS` positionally nor keeps a second set of wire strings.
const DESCRIBE: &str = contract::names::DESCRIBE;
const CREATE: &str = contract::names::CREATE;
const EXEC: &str = contract::names::EXEC;
const INSPECT: &str = contract::names::INSPECT;
const CLOSE: &str = contract::names::CLOSE;
const SPAWN: &str = contract::names::SPAWN;
const IS_RUNNING: &str = contract::names::IS_RUNNING;
const CANCEL: &str = contract::names::CANCEL;
const ANALYZE_SHELL: &str = contract::names::ANALYZE_SHELL;
const RESERVE: &str = contract::names::RESERVE;
const CAPABILITIES: &str = contract::names::CAPABILITIES;
const SHUTDOWN: &str = contract::names::SHUTDOWN;
const FORWARD: &str = contract::names::FORWARD;
const CLOSE_FORWARD: &str = contract::names::CLOSE_FORWARD;
const JAIL_STATUS: &str = contract::names::JAIL_STATUS;
const ANALYZE_COMMAND: &str = contract::names::ANALYZE_COMMAND;
const BEGIN_FILE_READ: &str = contract::names::BEGIN_FILE_READ;
const READ_FILE_CHUNK: &str = contract::names::READ_FILE_CHUNK;
const FINISH_FILE_READ: &str = contract::names::FINISH_FILE_READ;
const BEGIN_FILE_WRITE: &str = contract::names::BEGIN_FILE_WRITE;
const WRITE_FILE_CHUNK: &str = contract::names::WRITE_FILE_CHUNK;
const FINISH_FILE_WRITE: &str = contract::names::FINISH_FILE_WRITE;
const ABORT_FILE_WRITE: &str = contract::names::ABORT_FILE_WRITE;
const START_EXEC: &str = contract::names::START_EXEC;
const READ_OUTPUT: &str = contract::names::READ_OUTPUT;
const RELEASE_OUTPUT: &str = contract::names::RELEASE_OUTPUT;
const START_JAILED_EXEC: &str = contract::names::START_JAILED_EXEC;
const RUN_DOCKER: &str = contract::names::RUN_DOCKER;
const DOCKER_STATUS: &str = contract::names::DOCKER_STATUS;
const CLEANUP_DOCKER: &str = contract::names::CLEANUP_DOCKER;
const RUN_DOCKER_WITHOUT_WORKSPACE: &str = contract::names::RUN_DOCKER_WITHOUT_WORKSPACE;
const SELECT_SHELL: &str = contract::names::SELECT_SHELL;
const RESOLVE_LOCAL_GRANTS: &str = contract::names::RESOLVE_LOCAL_GRANTS;
const START_NATIVE_SHELL_EXEC: &str = contract::names::START_NATIVE_SHELL_EXEC;
const VALIDATE_DOCKER_POLICY: &str = contract::names::VALIDATE_DOCKER_POLICY;
const DOCKER_CALL_GRACE: Duration = Duration::from_secs(5);

#[cfg(test)]
#[path = "tinybox_tests.rs"]
mod tests;

/// Resolve TinyBox, require the bus contract supported by this host, and
/// return its proxy.
///
/// The capability check is deliberate: the registry artifact may be older
/// than this compiled host while a release is being rolled out. A package
/// version alone does not prove which additive bus members it serves.
///
/// # Errors
///
/// Returns a safe loading, transport, or contract-version error. It does not
/// include request arguments or filesystem paths.
fn require_compatible(version: (u32, u32)) -> Result<(), ModuleCallError> {
    if contract::is_compatible(version) {
        return Ok(());
    }
    crate::modules::report::incompatible_contract(MODULE_ID);
    Err(ModuleCallError::IncompatibleContract)
}

/// Describe the loaded TinyBox module.
pub async fn describe(config: &Config) -> Result<String, String> {
    call(config, DESCRIBE, (), false).await
}

/// Read module capabilities after enforcing contract compatibility.
pub async fn capabilities(config: &Config) -> Result<contract::ModuleCapabilities, String> {
    let client = module_client(config);
    let capabilities: contract::ModuleCapabilities = client
        .call(MODULE_ID, CAPABILITIES, ())
        .await
        .map_err(message)?;
    require_compatible(capabilities.contract_version).map_err(message)?;
    Ok(capabilities)
}

/// Read the native jail facts; the caller applies its own policy to them.
pub async fn jail_status(config: &Config) -> Result<contract::JailStatus, String> {
    call(config, JAIL_STATUS, (), false).await
}

/// Reserve an opaque identifier before native startup.
pub async fn reserve(
    config: &Config,
    request: contract::ReserveRequest,
) -> Result<contract::ResourceId, String> {
    call(config, RESERVE, (request,), false).await
}

/// Create a sandbox using its caller-known reservation.
pub async fn create(
    config: &Config,
    request: contract::CreateRequest,
) -> Result<contract::ResourceInfo, String> {
    call(config, CREATE, (request,), true).await
}

/// Execute a collected command in a resource.
pub async fn exec(
    config: &Config,
    request: contract::ExecRequest,
) -> Result<contract::ExecOutput, String> {
    call(config, EXEC, (request,), true).await
}

/// Inspect a module-owned resource.
pub async fn inspect(
    config: &Config,
    resource: contract::ResourceId,
) -> Result<contract::ResourceInfo, String> {
    call(config, INSPECT, (resource,), false).await
}

/// Close a resource and wait for the module's native cleanup acknowledgement.
pub async fn close(config: &Config, resource: contract::ResourceId) -> Result<(), String> {
    call(config, CLOSE, (resource,), false).await
}

/// Shut down the module-owned resources.
pub async fn shutdown(config: &Config) -> Result<(), String> {
    call(config, SHUTDOWN, (), false).await
}

/// Start a detached process using a pre-reserved process id.
pub async fn spawn(
    config: &Config,
    request: contract::SpawnRequest,
) -> Result<contract::ProcessRef, String> {
    call(config, SPAWN, (request,), true).await
}

/// Start a supervised process with live output collection.
pub async fn start_exec(
    config: &Config,
    request: contract::SpawnRequest,
) -> Result<contract::ProcessRef, String> {
    call(config, START_EXEC, (request,), true).await
}

/// Start a process in the explicitly requested native directory jail.
pub async fn start_jailed_exec(
    config: &Config,
    request: contract::SpawnRequest,
    jail: contract::NativeJailConfig,
) -> Result<contract::ProcessRef, String> {
    call(config, START_JAILED_EXEC, (request, jail), true).await
}

/// Ask TinyBox which shell it would spawn for a command on its machine.
///
/// The reply is discovery only: the host never spawns the plan itself, it
/// executes through [`start_native_shell_exec`].
pub async fn select_shell(
    config: &Config,
    request: contract::SelectShellRequest,
) -> Result<contract::ShellInvocation, String> {
    call(config, SELECT_SHELL, (request,), false).await
}

/// Start a local shell under module-owned supervision and output replay.
///
/// The caller has already authorized the command; the module owns shell
/// selection, the safe child environment and cleanup.
pub async fn start_native_shell_exec(
    config: &Config,
    request: contract::ShellExecRequest,
) -> Result<contract::ProcessRef, String> {
    call(config, START_NATIVE_SHELL_EXEC, (request,), true).await
}

/// Ask TinyBox whether a Docker run would weaken isolation, without starting
/// one. This is the same check `RunDocker` enforces before creating a container.
pub async fn validate_docker_policy(
    config: &Config,
    request: contract::DockerPolicyRequest,
) -> Result<contract::DockerPolicyReport, String> {
    call(config, VALIDATE_DOCKER_POLICY, (request,), false).await
}

/// Ask TinyBox to resolve the existing toolchain and configured grants for the
/// local jail.
///
/// The request carries the host's protected-path floor as input; the reply is
/// a candidate set the host must still pass through its own floor.
pub async fn resolve_local_grants(
    config: &Config,
    request: contract::ResolveLocalGrantsRequest,
) -> Result<contract::LocalGrants, String> {
    call(config, RESOLVE_LOCAL_GRANTS, (request,), false).await
}

/// Run one bounded Docker-isolated command through TinyBox.
///
/// The caller applies authorization and Docker policy before sending this
/// confidential request. The bus deadline includes a short reply grace after
/// TinyBox's own command deadline so the timeout result can be delivered.
pub async fn run_docker(
    config: &Config,
    request: contract::DockerRunRequest,
) -> Result<contract::DockerRunOutput, String> {
    let timeout = Duration::from_millis(request.timeout_ms).saturating_add(DOCKER_CALL_GRACE);
    let client = crate::modules::client(config.clone()).with_timeout(timeout);
    call_run_docker_with_client(&client, request).await
}

/// Run one bounded Docker-isolated command without mounting a workspace.
///
/// This is used by the configured Docker runtime, whose existing behavior
/// intentionally does not expose host workspace files to the container.
pub async fn run_docker_without_workspace(
    config: &Config,
    request: contract::DockerRunRequest,
) -> Result<contract::DockerRunOutput, String> {
    let timeout = Duration::from_millis(request.timeout_ms).saturating_add(DOCKER_CALL_GRACE);
    let client = crate::modules::client(config.clone()).with_timeout(timeout);
    call_run_docker_without_workspace_with_client(&client, request).await
}

/// Probe the Docker daemon owned by the TinyBox module host.
pub async fn docker_status(
    config: &Config,
    request: contract::DockerStatusRequest,
) -> Result<contract::DockerStatus, String> {
    call_docker_status_with_client(&module_client(config), request).await
}

/// Stop TinyBox-owned containers matching the supplied label.
pub async fn cleanup_docker(
    config: &Config,
    request: contract::CleanupDockerRequest,
) -> Result<contract::CleanupDockerOutput, String> {
    call_cleanup_docker_with_client(&module_client(config), request).await
}

fn docker_control_deadline(timeout_ms: u64, grace: Duration) -> Duration {
    Duration::from_millis(timeout_ms.min(contract::MAX_DOCKER_CONTROL_TIMEOUT_MS))
        .saturating_add(grace)
}

fn docker_control_client(client: &ModuleClient, timeout_ms: u64, grace: Duration) -> ModuleClient {
    client
        .clone()
        .with_timeout(docker_control_deadline(timeout_ms, grace))
}

async fn call_docker_status_with_client(
    client: &ModuleClient,
    request: contract::DockerStatusRequest,
) -> Result<contract::DockerStatus, String> {
    call_docker_status_with_grace(client, request, DOCKER_CALL_GRACE).await
}

async fn call_docker_status_with_grace(
    client: &ModuleClient,
    request: contract::DockerStatusRequest,
    grace: Duration,
) -> Result<contract::DockerStatus, String> {
    let client = docker_control_client(client, request.timeout_ms, grace);
    call_with_client(&client, DOCKER_STATUS, (request,), false).await
}

async fn call_cleanup_docker_with_client(
    client: &ModuleClient,
    request: contract::CleanupDockerRequest,
) -> Result<contract::CleanupDockerOutput, String> {
    call_cleanup_docker_with_grace(client, request, DOCKER_CALL_GRACE).await
}

async fn call_cleanup_docker_with_grace(
    client: &ModuleClient,
    request: contract::CleanupDockerRequest,
    grace: Duration,
) -> Result<contract::CleanupDockerOutput, String> {
    let client = docker_control_client(client, request.timeout_ms, grace);
    call_with_client(&client, CLEANUP_DOCKER, (request,), false).await
}

async fn call_run_docker_with_client(
    client: &ModuleClient,
    request: contract::DockerRunRequest,
) -> Result<contract::DockerRunOutput, String> {
    call_with_client(client, RUN_DOCKER, (request,), true).await
}

async fn call_run_docker_without_workspace_with_client(
    client: &ModuleClient,
    request: contract::DockerRunRequest,
) -> Result<contract::DockerRunOutput, String> {
    call_with_client(client, RUN_DOCKER_WITHOUT_WORKSPACE, (request,), true).await
}

/// Read a bounded output batch from a live or completed process.
pub async fn read_output(
    config: &Config,
    process: contract::ProcessRef,
    cursor: u64,
) -> Result<contract::OutputBatch, String> {
    call(config, READ_OUTPUT, (process, cursor), false).await
}

/// Release retained output and process ownership after terminal observation.
pub async fn release_output(config: &Config, process: contract::ProcessRef) -> Result<(), String> {
    call(config, RELEASE_OUTPUT, (process,), false).await
}

/// Check whether a detached process remains active.
pub async fn is_running(config: &Config, process: contract::ProcessRef) -> Result<bool, String> {
    call(config, IS_RUNNING, (process,), false).await
}

/// Cancel a detached process and wait for acknowledged cleanup.
pub async fn cancel(config: &Config, process: contract::ProcessRef) -> Result<(), String> {
    call(config, CANCEL, (process,), false).await
}

/// Open a module-owned port forward.
pub async fn forward(
    config: &Config,
    request: contract::ForwardRequest,
) -> Result<contract::ForwardInfo, String> {
    call(config, FORWARD, (request,), false).await
}

/// Close a module-owned port forward.
pub async fn close_forward(
    config: &Config,
    request: contract::CloseForwardRequest,
) -> Result<(), String> {
    call(config, CLOSE_FORWARD, (request,), false).await
}

/// Ask TinyBox for structural shell facts; this is not an authorization check.
pub async fn analyze_shell(
    config: &Config,
    command: String,
) -> Result<contract::ShellAnalysis, String> {
    call(config, ANALYZE_SHELL, (command,), false).await
}

/// Ask TinyBox for generic command facts; the caller still decides policy.
pub async fn analyze_command(
    config: &Config,
    command: String,
) -> Result<contract::CommandAnalysis, String> {
    call(config, ANALYZE_COMMAND, (command,), false).await
}

/// Open a bounded workspace file reader.
pub async fn begin_file_read(
    config: &Config,
    request: contract::BeginFileReadRequest,
) -> Result<contract::FileReadInfo, String> {
    call(config, BEGIN_FILE_READ, (request,), false).await
}

/// Read one bounded range from an open file reader.
pub async fn read_file_chunk(
    config: &Config,
    request: contract::ReadFileChunkRequest,
) -> Result<contract::FileChunk, String> {
    call(config, READ_FILE_CHUNK, (request,), false).await
}

/// Close a workspace file reader.
pub async fn finish_file_read(
    config: &Config,
    request: contract::FinishFileReadRequest,
) -> Result<(), String> {
    call(config, FINISH_FILE_READ, (request,), false).await
}

/// Open an atomic staged workspace file writer.
pub async fn begin_file_write(
    config: &Config,
    request: contract::BeginFileWriteRequest,
) -> Result<contract::FileWriteInfo, String> {
    call(config, BEGIN_FILE_WRITE, (request,), false).await
}

/// Write one bounded sequential chunk.
pub async fn write_file_chunk(
    config: &Config,
    request: contract::WriteFileChunkRequest,
) -> Result<contract::FileWriteProgress, String> {
    call(config, WRITE_FILE_CHUNK, (request,), true).await
}

/// Atomically publish the staged workspace file.
pub async fn finish_file_write(
    config: &Config,
    request: contract::FinishFileWriteRequest,
) -> Result<contract::FileWriteProgress, String> {
    call(config, FINISH_FILE_WRITE, (request,), false).await
}

/// Abort a staged workspace write without publishing it.
pub async fn abort_file_write(
    config: &Config,
    request: contract::AbortFileWriteRequest,
) -> Result<(), String> {
    call(config, ABORT_FILE_WRITE, (request,), false).await
}

async fn call<Request, Reply>(
    config: &Config,
    member: &'static str,
    request: Request,
    confidential: bool,
) -> Result<Reply, String>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    // A registry index from before the storage contract moves into the
    // module's storage before its first call. Once per workspace, best effort.
    super::import_legacy_tinybox(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config.workspace_dir,
    )
    .await;
    call_with_client(&module_client(config), member, request, confidential).await
}

fn module_client(config: &Config) -> ModuleClient {
    crate::modules::client(config.clone()).with_timeout(MODULE_CALL_TIMEOUT)
}

fn message(error: ModuleCallError) -> String {
    error.to_string()
}

async fn call_with_client<Request, Reply>(
    client: &ModuleClient,
    member: &'static str,
    request: Request,
    confidential: bool,
) -> Result<Reply, String>
where
    Request: Serialize + Send,
    Reply: DeserializeOwned,
{
    let capabilities: contract::ModuleCapabilities = client
        .call(MODULE_ID, CAPABILITIES, ())
        .await
        .map_err(message)?;
    require_compatible(capabilities.contract_version).map_err(message)?;
    if confidential {
        client
            .call_confidential(MODULE_ID, member, request)
            .await
            .map_err(message)
    } else {
        client
            .call(MODULE_ID, member, request)
            .await
            .map_err(message)
    }
}
