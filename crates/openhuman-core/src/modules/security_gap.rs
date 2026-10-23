//! Host client for the module's policy and egress members of contract 1.22:
//! `CheckPaths2`, `ProtectedPath2`, `ProtectedFloor2`, `SpendAction2`,
//! `PeekBudget2`, `ClassifyHostAction2` and `PrivacyCheck2`.
//!
//! The module decides every one of these questions; this file owns transport, the
//! scope registration caches the neighbouring clients share, the bridge for
//! synchronous callers and the failure rule: a module that is missing, faulted or
//! slow is `Err`, and every caller turns `Err` into a refusal. There is no local
//! answer to fall back on.
//!
//! # The port
//!
//! Callers reach the module through [`GapPort`]. Production always uses
//! [`ModulePort`]. Unit tests of code that has no module runtime install a scripted
//! port for the current thread (`install_port`, test builds only) and so exercise
//! exactly the call path production takes; the port is not a second policy.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tinysecurity_bus::{
    names, BudgetDecision2, BudgetRequest2, CommandClass, CommandPolicy, Decision,
    HostActionClassRequest2, HostActionClassResponse2, PathBatchRequest2, PathBatchResponse2,
    PathCheckKind, PathPolicy, PrivacyCheckRequest2, PrivacyCheckResponse2, ProtectedFloor2,
    ProtectedPathRequest2, ProtectedPathResponse2, MAX_PATH_BATCH,
};

use super::{config, lru_insert, lru_lookup, require_enabled, state, CallError};

/// A synchronous caller waits this long for the module before it is refused.
const BLOCKING_TIMEOUT: Duration = Duration::from_secs(3);

/// Whether `SpendAction2` or `PeekBudget2` is asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BudgetOp {
    /// Reserve one action from the hourly ledger.
    Spend,
    /// Read the ledger without reserving.
    Peek,
}

/// The module questions of contract 1.22, as one seam.
#[async_trait]
pub(crate) trait GapPort: std::fmt::Debug + Send + Sync {
    /// `CheckPaths2` under `scope`: one answer per path, in order.
    async fn check_paths(
        &self,
        scope: PathPolicy,
        kind: PathCheckKind,
        paths: Vec<PathBuf>,
    ) -> Result<Vec<bool>, String>;
    /// `ProtectedPath2`: one answer per path, in order.
    async fn protected_paths(&self, paths: Vec<PathBuf>) -> Result<Vec<bool>, String>;
    /// `ProtectedFloor2`.
    async fn protected_floor(&self) -> Result<ProtectedFloor2, String>;
    /// `SpendAction2` / `PeekBudget2` against the scope `policy` registers.
    async fn budget(
        &self,
        workspace_dir: PathBuf,
        policy: CommandPolicy,
        op: BudgetOp,
    ) -> Result<BudgetDecision2, String>;
    /// `PrivacyCheck2`.
    async fn privacy(&self, request: PrivacyCheckRequest2) -> Result<Decision, String>;
    /// `ClassifyHostAction2`.
    async fn host_action_class(
        &self,
        request: HostActionClassRequest2,
    ) -> Result<CommandClass, String>;
}

/// The production port: the attested module over the bus.
#[derive(Debug)]
pub(crate) struct ModulePort;

thread_local! {
    static OVERRIDE: std::cell::RefCell<Option<Arc<dyn GapPort>>> =
        const { std::cell::RefCell::new(None) };
}

fn overridden() -> Option<Arc<dyn GapPort>> {
    OVERRIDE.with(|slot| slot.borrow().clone())
}

fn port() -> Arc<dyn GapPort> {
    overridden().unwrap_or_else(|| Arc::new(ModulePort))
}

/// Restores the previous port of the thread when dropped.
#[cfg(test)]
pub(crate) struct PortGuard(Option<Arc<dyn GapPort>>);

#[cfg(test)]
impl Drop for PortGuard {
    fn drop(&mut self) {
        OVERRIDE.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// Answer this thread's gap questions with `port` until the guard drops.
#[cfg(test)]
pub(crate) fn install_port(port: Arc<dyn GapPort>) -> PortGuard {
    PortGuard(OVERRIDE.with(|slot| slot.borrow_mut().replace(port)))
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// `CheckPaths2` for any number of paths, split into module-sized batches. A batch
/// the module cannot answer fails the whole call.
pub(crate) async fn check_paths(
    scope: PathPolicy,
    kind: PathCheckKind,
    paths: &[PathBuf],
) -> Result<Vec<bool>, String> {
    let port = port();
    let mut allowed = Vec::with_capacity(paths.len());
    for chunk in paths.chunks(MAX_PATH_BATCH) {
        let answers = port
            .check_paths(scope.clone(), kind, chunk.to_vec())
            .await?;
        if answers.len() != chunk.len() {
            return Err("TinySecurity CheckPaths2 answer length mismatch".into());
        }
        allowed.extend(answers);
    }
    Ok(allowed)
}

/// `ProtectedPath2` for any number of paths. `Err` is a refusal: callers treat a
/// path they could not ask about as protected.
pub(crate) async fn protected_paths(paths: &[PathBuf]) -> Result<Vec<bool>, String> {
    let port = port();
    let mut protected = Vec::with_capacity(paths.len());
    for chunk in paths.chunks(MAX_PATH_BATCH) {
        let answers = port.protected_paths(chunk.to_vec()).await?;
        if answers.len() != chunk.len() {
            return Err("TinySecurity ProtectedPath2 answer length mismatch".into());
        }
        protected.extend(answers);
    }
    Ok(protected)
}

/// Whether `path` is inside the module's unconditional floor. Fails closed: a path
/// the module could not be asked about is protected.
pub(crate) async fn is_protected(path: &Path) -> bool {
    match protected_paths(&[path.to_path_buf()]).await {
        Ok(answers) => answers.first().copied().unwrap_or(true),
        Err(error) => {
            log::warn!("[security][floor] module unavailable; path treated as protected: {error}");
            true
        }
    }
}

/// Whether any of `paths` is inside the floor, in one call. Fails closed.
pub(crate) async fn any_protected(paths: &[&Path]) -> bool {
    let owned: Vec<PathBuf> = paths.iter().map(|path| path.to_path_buf()).collect();
    match protected_paths(&owned).await {
        Ok(answers) => answers.len() != owned.len() || answers.iter().any(|hit| *hit),
        Err(error) => {
            log::warn!("[security][floor] module unavailable; paths treated as protected: {error}");
            true
        }
    }
}

/// [`is_protected`] for a synchronous caller, through the shared seam. Fails closed.
pub(crate) fn is_protected_blocking(path: &Path) -> bool {
    let path = path.to_path_buf();
    match blocking(async move { protected_paths(&[path]).await }) {
        Ok(answers) => answers.first().copied().unwrap_or(true),
        Err(error) => {
            log::warn!("[security][floor] module unavailable; path treated as protected: {error}");
            true
        }
    }
}

/// The floor's vocabulary, for components that filter by it themselves.
pub(crate) async fn protected_floor() -> Result<ProtectedFloor2, String> {
    port().protected_floor().await
}

/// `SpendAction2`: reserve one action of the scope `policy` registers.
pub(crate) async fn spend_action(
    workspace_dir: &Path,
    policy: CommandPolicy,
) -> Result<BudgetDecision2, String> {
    port()
        .budget(workspace_dir.to_path_buf(), policy, BudgetOp::Spend)
        .await
}

/// `PeekBudget2`: read the ledger of the scope `policy` registers.
pub(crate) async fn peek_budget(
    workspace_dir: &Path,
    policy: CommandPolicy,
) -> Result<BudgetDecision2, String> {
    port()
        .budget(workspace_dir.to_path_buf(), policy, BudgetOp::Peek)
        .await
}

/// [`spend_action`] / [`peek_budget`] for a synchronous caller, through the seam.
pub(crate) fn budget_blocking(
    workspace_dir: &Path,
    policy: CommandPolicy,
    op: BudgetOp,
) -> Result<BudgetDecision2, String> {
    let workspace_dir = workspace_dir.to_path_buf();
    blocking(async move {
        match op {
            BudgetOp::Spend => spend_action(&workspace_dir, policy).await,
            BudgetOp::Peek => peek_budget(&workspace_dir, policy).await,
        }
    })
}

/// `PrivacyCheck2`. `Err` is a refusal.
pub(crate) async fn privacy_check(request: PrivacyCheckRequest2) -> Result<Decision, String> {
    port().privacy(request).await
}

/// `ClassifyHostAction2`. `Err` is a refusal.
pub(crate) async fn host_action_class(
    request: HostActionClassRequest2,
) -> Result<CommandClass, String> {
    port().host_action_class(request).await
}

/// Drive `future` from a synchronous caller. A scripted port answers on the calling
/// thread; the module is reached through the shared sync-to-async seam with the
/// caller's dispatch scope re-entered on the module runtime.
pub(crate) fn blocking<T, F>(future: F) -> Result<T, String>
where
    T: Send + 'static,
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
{
    if overridden().is_some() {
        return futures::executor::block_on(future);
    }
    super::sync_seam::arm_module_runtime();
    let scope = crate::core::runtime::DispatchCtx::ambient().core().cloned();
    super::sync_seam::run(BLOCKING_TIMEOUT, async move {
        match scope {
            Some(scope) => crate::core::runtime::CoreContext::scope(scope, future).await,
            None => future.await,
        }
    })
    .map_err(String::from)
}

// ---------------------------------------------------------------------------
// The module port
// ---------------------------------------------------------------------------

/// A process-wide config for questions that name no workspace (the floor, the
/// privacy decision, the class of a host action).
async fn ambient_config() -> Result<crate::config::Config, String> {
    crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await
}

#[async_trait]
impl GapPort for ModulePort {
    async fn check_paths(
        &self,
        scope: PathPolicy,
        kind: PathCheckKind,
        paths: Vec<PathBuf>,
    ) -> Result<Vec<bool>, String> {
        let config =
            config::host_config(&scope.workspace_dir, crate::core::runtime::is_saas()).await?;
        require_enabled(&config)?;
        let mut state = state().lock().await;
        let result = async {
            let proxy = super::proxy(&config, &mut state)
                .await
                .map_err(CallError::faulted)?;
            let policy_id = if let Some(id) = lru_lookup(&mut state.scopes, &scope) {
                id
            } else {
                let id = super::call_register_path_policy(&proxy, scope.clone())
                    .await
                    .map_err(CallError::faulted)?;
                lru_insert(&mut state.scopes, scope, id.clone());
                id
            };
            let response: PathBatchResponse2 = proxy
                .call_confidential(
                    names::methods::CHECK_PATHS2,
                    (PathBatchRequest2 {
                        policy_id,
                        kind,
                        paths,
                    },),
                )
                .await
                .map_err(|error| CallError::from_bus("TinySecurity CheckPaths2 failed", &error))?;
            Ok(response.allowed)
        }
        .await;
        settle(&mut state, "CheckPaths2", result)
    }

    async fn protected_paths(&self, paths: Vec<PathBuf>) -> Result<Vec<bool>, String> {
        let config = ambient_config().await?;
        let proxy = super::scoped_client::module_proxy(&config).await?;
        let response: ProtectedPathResponse2 = proxy
            .call_confidential(
                names::methods::PROTECTED_PATH2,
                (ProtectedPathRequest2 { paths },),
            )
            .await
            .map_err(|_| "TinySecurity ProtectedPath2 failed")?;
        Ok(response.protected)
    }

    async fn protected_floor(&self) -> Result<ProtectedFloor2, String> {
        let config = ambient_config().await?;
        let proxy = super::scoped_client::module_proxy(&config).await?;
        proxy
            .call_confidential(names::methods::PROTECTED_FLOOR2, ())
            .await
            .map_err(|_| "TinySecurity ProtectedFloor2 failed".into())
    }

    async fn budget(
        &self,
        workspace_dir: PathBuf,
        policy: CommandPolicy,
        op: BudgetOp,
    ) -> Result<BudgetDecision2, String> {
        let config = config::host_config(&workspace_dir, crate::core::runtime::is_saas()).await?;
        require_enabled(&config)?;
        let mut state = state().lock().await;
        let result = async {
            let proxy = super::proxy(&config, &mut state)
                .await
                .map_err(CallError::faulted)?;
            let command_policy =
                super::command::register_cached_command_scope(&mut state, &proxy, policy).await?;
            let member = match op {
                BudgetOp::Spend => names::methods::SPEND_ACTION2,
                BudgetOp::Peek => names::methods::PEEK_BUDGET2,
            };
            proxy
                .call_confidential(member, (BudgetRequest2 { command_policy },))
                .await
                .map_err(|error| CallError::from_bus("TinySecurity budget query failed", &error))
        }
        .await;
        settle(&mut state, "budget", result)
    }

    async fn privacy(&self, request: PrivacyCheckRequest2) -> Result<Decision, String> {
        let config = ambient_config().await?;
        let proxy = super::scoped_client::module_proxy(&config).await?;
        let response: PrivacyCheckResponse2 = proxy
            .call_confidential(names::methods::PRIVACY_CHECK2, (request,))
            .await
            .map_err(|_| "TinySecurity PrivacyCheck2 failed")?;
        Ok(response.decision)
    }

    async fn host_action_class(
        &self,
        request: HostActionClassRequest2,
    ) -> Result<CommandClass, String> {
        let config = ambient_config().await?;
        let proxy = super::scoped_client::module_proxy(&config).await?;
        let response: HostActionClassResponse2 = proxy
            .call_confidential(names::methods::CLASSIFY_HOST_ACTION2, (request,))
            .await
            .map_err(|_| "TinySecurity ClassifyHostAction2 failed")?;
        Ok(response.class)
    }
}

/// Latch the shared client on a channel failure only, as the neighbouring clients do.
fn settle<T>(
    state: &mut super::ClientState,
    member: &str,
    result: Result<T, CallError>,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            log::warn!(
                "[security][gap] {member} failed fault={}: {}",
                error.fault,
                error.message
            );
            if error.fault {
                state.latch();
            }
            Err(error.message)
        }
    }
}

#[cfg(test)]
#[path = "security_gap_tests.rs"]
mod tests;

/// A scripted [`GapPort`] for tests that run without a module runtime.
#[cfg(test)]
#[path = "security_gap_stub.rs"]
pub(crate) mod stub;
