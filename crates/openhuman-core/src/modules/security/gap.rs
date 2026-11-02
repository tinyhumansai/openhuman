//! Contract 1.22 policy-gap calls used by host security policy.
//!
//! TinySecurity owns the answer and the action ledger. The host supplies only
//! immutable configuration derived from its policy object and fails closed when
//! the module cannot answer.

#[cfg(test)]
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
#[cfg(test)]
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinysecurity_bus::{
    BudgetDecision2, CommandPolicy, CommandPolicyId, Decision, PathCheckKind, PathPolicy,
    PathPolicyId, PrivacyCheckRequest2,
};

/// Test seam for descriptor-level egress decisions.
#[cfg(test)]
#[async_trait]
pub(crate) trait PrivacyPort: std::fmt::Debug + Send + Sync {
    async fn privacy(&self, request: PrivacyCheckRequest2) -> Result<Decision, String>;
}

#[cfg(test)]
thread_local! {
    static TEST_PRIVACY_PORT: std::cell::RefCell<Option<Arc<dyn PrivacyPort>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct PrivacyPortGuard(Option<Arc<dyn PrivacyPort>>);

#[cfg(test)]
impl Drop for PrivacyPortGuard {
    fn drop(&mut self) {
        TEST_PRIVACY_PORT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

#[cfg(test)]
pub(crate) fn install_privacy_port(port: Arc<dyn PrivacyPort>) -> PrivacyPortGuard {
    PrivacyPortGuard(TEST_PRIVACY_PORT.with(|slot| slot.borrow_mut().replace(port)))
}

/// `PrivacyCheck2` for a single host-derived egress descriptor. Module errors
/// remain errors so callers can refuse the transfer.
pub(crate) async fn privacy_check(request: PrivacyCheckRequest2) -> Result<Decision, String> {
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    privacy_check_with_config(&config, request).await
}

/// `PrivacyCheck2` for a host-owned configuration, used by bounded sync seams.
pub(crate) async fn privacy_check_with_config(
    config: &crate::config::Config,
    request: PrivacyCheckRequest2,
) -> Result<Decision, String> {
    #[cfg(test)]
    if let Some(port) = TEST_PRIVACY_PORT.with(|slot| slot.borrow().clone()) {
        return port.privacy(request).await;
    }
    super::client(config)
        .await
        .map_err(|error| error.to_string())?
        .privacy_check2(request)
        .await
        .map_err(|error| error.to_string())
}

/// TinySecurity's U5 provider-host locality classification. Failure is returned
/// as an error so callers can treat an unavailable authority as external.
pub(crate) async fn classify_provider_host(host: String) -> Result<bool, String> {
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    classify_provider_host_with_config(&config, host).await
}

pub(crate) async fn classify_provider_host_with_config(
    config: &crate::config::Config,
    host: String,
) -> Result<bool, String> {
    let response = super::client(config)
        .await
        .map_err(|error| error.to_string())?
        .classify_provider_endpoint2(host)
        .await
        .map_err(|error| error.to_string())?;
    Ok(response.locality == tinysecurity_bus::ProviderEndpointLocality2::OnDevice)
}

/// Whether to reserve an action or inspect the budget without reserving it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BudgetOp {
    /// Reserve one allowed action.
    Spend,
    /// Read the budget without reserving.
    Peek,
}

/// Whether a command answer is informational or reserves an allowed action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandOp {
    /// Classify without applying admission or spending budget.
    Classify,
    /// Apply the configured command gate and spend on allow.
    Check,
}

/// Port for the action budget, so host policy tests exercise the same boundary
/// as production without booting a module runtime.
#[async_trait]
pub(crate) trait BudgetPort: std::fmt::Debug + Send + Sync {
    /// Ask TinySecurity to spend or inspect the policy's hourly budget.
    async fn budget(
        &self,
        workspace_dir: PathBuf,
        policy: CommandPolicy,
        op: BudgetOp,
    ) -> Result<BudgetDecision2, String>;
}

/// Scriptable TinySecurity path-floor operations for jail and path adapters.
#[cfg(test)]
#[async_trait]
pub(crate) trait PathFloorPort: std::fmt::Debug + Send + Sync {
    async fn protected_paths(&self, paths: &[PathBuf]) -> Result<Vec<bool>, String>;
    async fn check_paths(
        &self,
        _kind: tinysecurity_bus::PathCheckKind,
        paths: &[PathBuf],
    ) -> Result<Vec<bool>, String> {
        Ok(vec![false; paths.len()])
    }
    async fn protected_floor(&self) -> Result<tinysecurity_bus::ProtectedFloor2, String>;
    async fn credential_roots(&self, home: Option<PathBuf>) -> Result<Vec<PathBuf>, String>;
}

/// Scripted path authorization result for host request/outcome tests.
#[cfg(test)]
#[async_trait]
pub(crate) trait PathValidationPort: std::fmt::Debug + Send + Sync {
    async fn validate_host_path(
        &self,
        policy: PathPolicy,
        path: String,
        parent: bool,
    ) -> Result<tinysecurity_bus::PathValidationResult, String>;
}

#[cfg(test)]
thread_local! {
    static TEST_PATH_FLOOR_PORT: std::cell::RefCell<Option<Arc<dyn PathFloorPort>>> = const { std::cell::RefCell::new(None) };
    static TEST_PATH_VALIDATION_PORT: std::cell::RefCell<Option<Arc<dyn PathValidationPort>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct PathFloorPortGuard(Option<Arc<dyn PathFloorPort>>);

#[cfg(test)]
impl Drop for PathFloorPortGuard {
    fn drop(&mut self) {
        TEST_PATH_FLOOR_PORT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

#[cfg(test)]
pub(crate) fn install_path_floor_port(port: Arc<dyn PathFloorPort>) -> PathFloorPortGuard {
    PathFloorPortGuard(TEST_PATH_FLOOR_PORT.with(|slot| slot.borrow_mut().replace(port)))
}

#[cfg(test)]
pub(crate) struct PathValidationPortGuard(Option<Arc<dyn PathValidationPort>>);

#[cfg(test)]
impl Drop for PathValidationPortGuard {
    fn drop(&mut self) {
        TEST_PATH_VALIDATION_PORT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

#[cfg(test)]
pub(crate) fn install_path_validation_port(
    port: Arc<dyn PathValidationPort>,
) -> PathValidationPortGuard {
    PathValidationPortGuard(TEST_PATH_VALIDATION_PORT.with(|slot| slot.borrow_mut().replace(port)))
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct ScriptedPathFloorPort {
    protected: HashSet<PathBuf>,
    floor: tinysecurity_bus::ProtectedFloor2,
    credentials: Vec<PathBuf>,
}

#[cfg(test)]
impl ScriptedPathFloorPort {
    pub(crate) fn new(
        protected: impl IntoIterator<Item = PathBuf>,
        credentials: Vec<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            protected: protected.into_iter().collect(),
            floor: tinysecurity_bus::ProtectedFloor2 {
                components: vec![".ssh".into(), ".gnupg".into(), ".aws".into()],
                files: vec![".netrc".into()],
                pairs: vec![[".cargo".into(), "credentials.toml".into()]],
                prefixes: vec!["/etc".into(), "/root".into(), "/proc".into()],
            },
            credentials,
        })
    }
}

#[cfg(test)]
#[async_trait]
impl PathFloorPort for ScriptedPathFloorPort {
    async fn protected_paths(&self, paths: &[PathBuf]) -> Result<Vec<bool>, String> {
        Ok(paths
            .iter()
            .map(|path| self.protected.contains(path))
            .collect())
    }

    async fn protected_floor(&self) -> Result<tinysecurity_bus::ProtectedFloor2, String> {
        Ok(self.floor.clone())
    }

    async fn credential_roots(&self, _home: Option<PathBuf>) -> Result<Vec<PathBuf>, String> {
        Ok(self.credentials.clone())
    }
}

#[derive(Debug)]
struct ModuleBudgetPort;

thread_local! {
    #[cfg(test)]
    static TEST_BUDGET_PORT: std::cell::RefCell<Option<Arc<dyn BudgetPort>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) struct BudgetPortGuard(Option<Arc<dyn BudgetPort>>);

#[cfg(test)]
impl Drop for BudgetPortGuard {
    fn drop(&mut self) {
        TEST_BUDGET_PORT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

#[cfg(test)]
pub(crate) fn install_budget_port(port: Arc<dyn BudgetPort>) -> BudgetPortGuard {
    BudgetPortGuard(TEST_BUDGET_PORT.with(|slot| slot.borrow_mut().replace(port)))
}

/// Test implementation that returns a configured result and records each ask.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct ScriptedBudgetPort {
    result: BudgetDecision2,
    asked: Mutex<Vec<(BudgetOp, u32)>>,
}

#[cfg(test)]
impl ScriptedBudgetPort {
    pub(crate) fn replying(result: BudgetDecision2) -> Arc<Self> {
        Arc::new(Self {
            result,
            asked: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn asked(&self) -> Vec<(BudgetOp, u32)> {
        self.asked.lock().expect("budget ask lock").clone()
    }
}

#[cfg(test)]
#[async_trait]
impl BudgetPort for ScriptedBudgetPort {
    async fn budget(
        &self,
        _workspace_dir: PathBuf,
        policy: CommandPolicy,
        op: BudgetOp,
    ) -> Result<BudgetDecision2, String> {
        self.asked
            .lock()
            .map_err(|_| "scripted budget port poisoned".to_owned())?
            .push((op, policy.max_actions_per_hour));
        Ok(self.result.clone())
    }
}

/// Ask the module to spend or peek. Test callers may install a scripted port;
/// production always uses the attested module client.
pub(crate) async fn budget(
    workspace_dir: PathBuf,
    policy: CommandPolicy,
    op: BudgetOp,
) -> Result<BudgetDecision2, String> {
    #[cfg(test)]
    if let Some(port) = TEST_BUDGET_PORT.with(|slot| slot.borrow().clone()) {
        return port.budget(workspace_dir, policy, op).await;
    }
    ModuleBudgetPort.budget(workspace_dir, policy, op).await
}

/// `SpendAction2` for a registered command scope.
pub(crate) async fn spend_action(
    workspace_dir: &Path,
    policy: CommandPolicy,
) -> Result<BudgetDecision2, String> {
    budget(workspace_dir.to_path_buf(), policy, BudgetOp::Spend).await
}

/// `PeekBudget2` for a registered command scope.
pub(crate) async fn peek_budget(
    workspace_dir: &Path,
    policy: CommandPolicy,
) -> Result<BudgetDecision2, String> {
    budget(workspace_dir.to_path_buf(), policy, BudgetOp::Peek).await
}

/// Bounded sync boundary: synchronous adapters refuse unless an async admission
/// has already populated their policy snapshot. It never re-enters the caller's
/// Tokio runtime.
pub(crate) fn budget_blocking(
    _workspace_dir: &Path,
    _policy: CommandPolicy,
    _op: BudgetOp,
) -> Result<BudgetDecision2, String> {
    Err("TinySecurity budget query requires async admission".into())
}

/// Batch path checks through the registered TinySecurity scope.
pub(crate) async fn check_paths(
    policy: PathPolicy,
    kind: PathCheckKind,
    paths: &[PathBuf],
) -> Result<Vec<bool>, String> {
    #[cfg(test)]
    if let Some(port) = TEST_PATH_FLOOR_PORT.with(|slot| slot.borrow().clone()) {
        let answers = port.check_paths(kind, paths).await?;
        if answers.len() != paths.len() {
            return Err("scripted TinySecurity CheckPaths2 answer length mismatch".into());
        }
        return Ok(answers);
    }
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    let client = super::client(&config)
        .await
        .map_err(|error| error.to_string())?;
    static PATH_SCOPES: OnceLock<tokio::sync::Mutex<Vec<(PathPolicy, PathPolicyId)>>> =
        OnceLock::new();
    let scopes = PATH_SCOPES.get_or_init(|| tokio::sync::Mutex::new(Vec::new()));
    let mut scopes = scopes.lock().await;
    let policy_id = match scopes
        .iter()
        .find(|(registered, _)| registered == &policy)
        .map(|(_, id)| id.clone())
    {
        Some(id) => id,
        None => {
            let id = client
                .register_path_policy(policy.clone())
                .await
                .map_err(|error| error.to_string())?;
            scopes.push((policy, id.clone()));
            id
        }
    };
    let response = client
        .check_paths2(tinysecurity_bus::PathBatchRequest2 {
            policy_id,
            kind,
            paths: paths.to_vec(),
        })
        .await
        .map_err(|error| error.to_string())?;
    if response.allowed.len() != paths.len() {
        return Err("TinySecurity CheckPaths2 answer length mismatch".into());
    }
    Ok(response.allowed)
}

/// Resolve and authorize an existing path or its destination parent.
pub(crate) async fn validate_host_path(
    policy: PathPolicy,
    path: &str,
    parent: bool,
) -> Result<tinysecurity_bus::PathValidationResult, String> {
    #[cfg(test)]
    if let Some(port) = TEST_PATH_VALIDATION_PORT.with(|slot| slot.borrow().clone()) {
        return port
            .validate_host_path(policy, path.to_owned(), parent)
            .await;
    }
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    let client = super::client(&config)
        .await
        .map_err(|error| error.to_string())?;
    let scopes = PATH_SCOPES.get_or_init(|| tokio::sync::Mutex::new(Vec::new()));
    let mut scopes = scopes.lock().await;
    let policy_id = match scopes
        .iter()
        .find(|(registered, _)| registered == &policy)
        .map(|(_, id)| id.clone())
    {
        Some(id) => id,
        None => {
            let id = client
                .register_path_policy(policy.clone())
                .await
                .map_err(|error| error.to_string())?;
            scopes.push((policy, id.clone()));
            id
        }
    };
    let request = tinysecurity_bus::PathValidationRequest {
        policy_id,
        path: path.to_owned(),
    };
    if parent {
        client.validate_parent(request).await
    } else {
        client.validate_path(request).await
    }
    .map_err(|error| error.to_string())
}

static PATH_SCOPES: OnceLock<tokio::sync::Mutex<Vec<(PathPolicy, PathPolicyId)>>> = OnceLock::new();

/// `ProtectedPath2`: one answer per path, in request order.
pub(crate) async fn protected_paths(paths: &[PathBuf]) -> Result<Vec<bool>, String> {
    #[cfg(test)]
    if let Some(port) = TEST_PATH_FLOOR_PORT.with(|slot| slot.borrow().clone()) {
        let answers = port.protected_paths(paths).await?;
        if answers.len() != paths.len() {
            return Err("scripted TinySecurity ProtectedPath2 answer length mismatch".into());
        }
        return Ok(answers);
    }
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    let response = super::client(&config)
        .await
        .map_err(|error| error.to_string())?
        .protected_path2(tinysecurity_bus::ProtectedPathRequest2 {
            paths: paths.to_vec(),
        })
        .await
        .map_err(|error| error.to_string())?;
    if response.protected.len() != paths.len() {
        return Err("TinySecurity ProtectedPath2 answer length mismatch".into());
    }
    Ok(response.protected)
}

/// `ProtectedFloor2` vocabulary for consumers that hand the floor to a jail.
pub(crate) async fn protected_floor() -> Result<tinysecurity_bus::ProtectedFloor2, String> {
    #[cfg(test)]
    if let Some(port) = TEST_PATH_FLOOR_PORT.with(|slot| slot.borrow().clone()) {
        return port.protected_floor().await;
    }
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    super::client(&config)
        .await
        .map_err(|error| error.to_string())?
        .protected_floor2()
        .await
        .map_err(|error| error.to_string())
}

/// Credential roots for handing path denials to a host jail.
pub(crate) async fn credential_roots(home: Option<PathBuf>) -> Result<Vec<PathBuf>, String> {
    #[cfg(test)]
    if let Some(port) = TEST_PATH_FLOOR_PORT.with(|slot| slot.borrow().clone()) {
        let roots = port.credential_roots(home).await?;
        let response = tinysecurity_bus::CredentialRootsResponse2 { roots };
        response.validate().map_err(|error| error.to_string())?;
        return Ok(response.roots);
    }
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    let response = super::client(&config)
        .await
        .map_err(|error| error.to_string())?
        .credential_roots2(tinysecurity_bus::CredentialRootsRequest2 { home })
        .await
        .map_err(|error| error.to_string())?;
    response.validate().map_err(|error| error.to_string())?;
    Ok(response.roots)
}

/// `ClassifyHostAction2` uses only trusted host declarations.
pub(crate) async fn host_action_class(
    request: tinysecurity_bus::HostActionClassRequest2,
) -> Result<tinysecurity_bus::CommandClass, String> {
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    Ok(super::client(&config)
        .await
        .map_err(|error| error.to_string())?
        .classify_host_action2(request)
        .await
        .map_err(|error| error.to_string())?
        .class)
}

/// Path operations from sync-only host traits fail closed instead of nesting
/// an async runtime on the caller's thread.
pub(crate) fn blocking<T, F>(_future: F) -> Result<T, String>
where
    T: Send + 'static,
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
{
    Err("TinySecurity path query requires async admission".into())
}

/// Classify or check a command under an immutable registered command scope.
pub(crate) async fn command_decision(
    workspace_dir: PathBuf,
    policy: CommandPolicy,
    command: String,
    declared_class: Option<tinysecurity_bus::CommandClass>,
    op: CommandOp,
) -> Result<tinysecurity_bus::CommandDecision, String> {
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await?;
    let client = super::client(&config)
        .await
        .map_err(|error| error.to_string())?;
    let scopes = COMMAND_SCOPES.get_or_init(|| tokio::sync::Mutex::new(Vec::new()));
    let mut scopes = scopes.lock().await;
    let policy_id = match scopes
        .iter()
        .find(|(workspace, registered, _)| workspace == &workspace_dir && registered == &policy)
        .map(|(_, _, id)| id.clone())
    {
        Some(id) => id,
        None => {
            let id = client
                .register_command_policy(policy.clone())
                .await
                .map_err(|error| error.to_string())?;
            scopes.push((workspace_dir, policy, id.clone()));
            id
        }
    };
    let request = tinysecurity_bus::CommandRequest {
        policy_id,
        command,
        declared_class,
    };
    match op {
        CommandOp::Classify => client.classify_command(request).await,
        CommandOp::Check => client.check_command(request).await,
    }
    .map_err(|error| error.to_string())
}

static COMMAND_SCOPES: OnceLock<
    tokio::sync::Mutex<Vec<(PathBuf, CommandPolicy, CommandPolicyId)>>,
> = OnceLock::new();

#[async_trait]
impl BudgetPort for ModuleBudgetPort {
    async fn budget(
        &self,
        workspace_dir: PathBuf,
        policy: CommandPolicy,
        op: BudgetOp,
    ) -> Result<BudgetDecision2, String> {
        let config = crate::core::runtime::DispatchCtx::ambient()
            .load_config()
            .await?;
        let client = super::client(&config)
            .await
            .map_err(|error| error.to_string())?;
        let scopes = COMMAND_SCOPES.get_or_init(|| tokio::sync::Mutex::new(Vec::new()));
        let mut scopes = scopes.lock().await;
        let policy_id = match scopes
            .iter()
            .find(|(workspace, registered, _)| workspace == &workspace_dir && registered == &policy)
            .map(|(_, _, id)| id.clone())
        {
            Some(id) => id,
            None => {
                let id = client
                    .register_command_policy(policy.clone())
                    .await
                    .map_err(|error| error.to_string())?;
                scopes.push((workspace_dir, policy, id.clone()));
                id
            }
        };
        let request = tinysecurity_bus::BudgetRequest2 {
            command_policy: policy_id,
        };
        match op {
            BudgetOp::Spend => client.spend_action2(request).await,
            BudgetOp::Peek => client.peek_budget2(request).await,
        }
        .map_err(|error| error.to_string())
    }
}
