//! Authenticated TinyComputer callbacks for TinySecurity browser egress grants.

use async_trait::async_trait;
use serde_json::Value;
use tinybus::{BusName, CallContext, Connection, Interface, InterfaceName, MemberName, ObjectPath};
use tinycomputer_bus::browser_egress::{
    self as contract, BrowserEgressConsumeRequest, BrowserEgressDecision, BrowserEgressGrant,
    BrowserEgressTarget, BrowserEgressValidateRequest,
};
use tinycomputer_bus::consequence_host::{
    self, ConsequenceKind, ReviewActionRequest, ReviewActionResponse,
};

type BrowserScope = (
    String,
    u64,
    String,
    String,
    String,
    String,
    std::time::Instant,
);

const BROWSER_POLICY_REFRESH_AFTER: std::time::Duration = std::time::Duration::from_secs(25);

fn registered_scopes() -> &'static parking_lot::Mutex<Vec<BrowserScope>> {
    static SCOPES: std::sync::OnceLock<parking_lot::Mutex<Vec<BrowserScope>>> =
        std::sync::OnceLock::new();
    SCOPES.get_or_init(|| parking_lot::Mutex::new(Vec::new()))
}

/// Register the browser's destination-only policy from the authenticated host
/// context and return the immutable id and generation for TinyComputer config.
pub(crate) async fn browser_dial_config(config: &crate::config::Config) -> Result<Value, String> {
    let client = super::security::client(config)
        .await
        .map_err(|error| error.to_string())?;
    let tenant =
        crate::core::runtime::tenant::current_tenant().map_err(|error| error.to_string())?;
    let owner = super::security::policy_translation::subject_scope(
        &tenant,
        &config.workspace_dir,
        crate::core::runtime::is_saas(),
    )?;
    let privacy = match crate::security::live_policy::current_privacy_mode() {
        crate::config::PrivacyMode::Standard => tinysecurity_bus::PrivacyMode::Standard,
        crate::config::PrivacyMode::Sensitive => tinysecurity_bus::PrivacyMode::Sensitive,
        crate::config::PrivacyMode::LocalOnly => tinysecurity_bus::PrivacyMode::LocalOnly,
    };
    let info = client
        .policy_info()
        .await
        .map_err(|error| error.to_string())?;
    let privacy_key = format!("{privacy:?}");
    {
        let scopes = registered_scopes().lock();
        if let Some((id, generation)) = find_reusable_policy(
            &scopes,
            info.generation,
            &owner.user,
            &owner.workspace,
            &owner.agent,
            &privacy_key,
            std::time::Instant::now(),
        ) {
            return Ok(serde_json::json!({
                "policy_id": id,
                "generation": generation,
            }));
        }
    }
    let registered_at = std::time::Instant::now();
    let policy_id = client
        .register_egress_policy(tinysecurity_bus::EgressPolicy {
            owner: owner.clone(),
            ceiling: tinysecurity_bus::AccessTier::Full,
            privacy,
            destinations: Vec::new(),
            data_categories: vec![tinysecurity_bus::DataCategory::OpaqueBrowserTraffic],
            local_capabilities: Vec::new(),
            validity_ms: 30_000,
            open_public: true,
            preflight_transports: vec![tinysecurity_bus::EgressTransport::Browser],
        })
        .await
        .map_err(|error| error.to_string())?;
    let info = client
        .policy_info()
        .await
        .map_err(|error| error.to_string())?;
    if info.generation == 0 || policy_id.0.is_empty() {
        return Err("TinySecurity browser policy scope is unavailable".into());
    }
    let mut scopes = registered_scopes().lock();
    scopes.retain(|(id, generation, ..)| id != &policy_id.0 || *generation != info.generation);
    scopes.push((
        policy_id.0.clone(),
        info.generation,
        owner.user,
        owner.workspace,
        owner.agent,
        privacy_key,
        registered_at,
    ));
    if scopes.len() > 64 {
        scopes.remove(0);
    }
    Ok(serde_json::json!({
        "policy_id": policy_id.0,
        "generation": info.generation,
    }))
}

fn find_reusable_policy<'a>(
    scopes: &'a [BrowserScope],
    generation: u64,
    user: &str,
    workspace: &str,
    agent: &str,
    privacy: &str,
    now: std::time::Instant,
) -> Option<(&'a str, u64)> {
    scopes
        .iter()
        .find(
            |(
                _,
                cached_generation,
                cached_user,
                cached_workspace,
                cached_agent,
                cached_mode,
                created_at,
            )| {
                *cached_generation == generation
                    && cached_user == user
                    && cached_workspace == workspace
                    && cached_agent == agent
                    && cached_mode == privacy
                    && now.saturating_duration_since(*created_at) < BROWSER_POLICY_REFRESH_AFTER
            },
        )
        .map(|(id, generation, ..)| (id.as_str(), *generation))
}

fn is_registered(policy_id: &str, generation: u64) -> bool {
    registered_scopes()
        .lock()
        .iter()
        .any(|(id, current, ..)| id == policy_id && *current == generation)
}

#[derive(Clone)]
struct BrowserEgressHost;

#[derive(Clone)]
struct ConsequenceReviewHost;

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.ConsequenceHost")]
impl ConsequenceReviewHost {
    #[tinybus(confidential)]
    async fn review(&self, request: ReviewActionRequest) -> tinybus::Result<ReviewActionResponse> {
        let Some(gate) = crate::security::approval::ApprovalGate::try_global() else {
            return Ok(ReviewActionResponse {
                allow: false,
                reason: Some("Approval gate is unavailable".into()),
            });
        };
        let kind = match request.kind {
            ConsequenceKind::Irreversible => "irreversible",
            ConsequenceKind::Payment => "payment",
        };
        let summary = format!("{} ({kind})", request.action);
        let args = serde_json::json!({
            "task": request.task,
            "action": request.action,
            "target": request.target,
            "kind": kind,
        });
        match gate.intercept_forced("browser", &summary, args).await {
            crate::security::approval::GateOutcome::Allow => Ok(ReviewActionResponse {
                allow: true,
                reason: None,
            }),
            crate::security::approval::GateOutcome::Deny { reason } => Ok(ReviewActionResponse {
                allow: false,
                reason: Some(reason),
            }),
        }
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinycomputer.BrowserEgressHost")]
impl BrowserEgressHost {
    async fn authorize(&self, target: BrowserEgressTarget) -> tinybus::Result<BrowserEgressGrant> {
        if !is_registered(&target.policy_id, target.expected_generation) {
            return Err(refused());
        }
        let config = crate::core::runtime::DispatchCtx::ambient()
            .load_config()
            .await
            .map_err(|_| refused())?;
        let client = super::security::client(&config)
            .await
            .map_err(|_| refused())?;
        let response = client
            .authorize_browser_dial(tinysecurity_bus::BrowserDialRequest {
                policy_id: tinysecurity_bus::EgressPolicyId(target.policy_id),
                expected_generation: target.expected_generation,
                session_id: target.session_id,
                scheme: target.scheme,
                hostname: target.hostname,
                port: target.port,
            })
            .await
            .map_err(|_| refused())?;
        if response.generation != target.expected_generation {
            return Err(refused());
        }
        let authorization = response.authorization.ok_or_else(refused)?;
        if response.generation != authorization.generation {
            return Err(refused());
        }
        let addresses = authorization
            .addresses
            .iter()
            .map(ToString::to_string)
            .collect();
        let authorization = serde_json::to_string(&authorization).map_err(|_| refused())?;
        Ok(BrowserEgressGrant {
            generation: response.generation,
            authorization,
            addresses,
        })
    }

    async fn consume(
        &self,
        request: BrowserEgressConsumeRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        let client = security_client().await?;
        let authorization = decode_browser_authorization(&request.authorization)?;
        let address = request.address.parse().map_err(|_| refused())?;
        let allowed = client
            .consume_browser_dial(tinysecurity_bus::ConsumeBrowserDialRequest {
                request: browser_request(request.target),
                address,
                authorization,
            })
            .await
            .unwrap_or(false);
        Ok(BrowserEgressDecision { allowed })
    }

    async fn validate(
        &self,
        request: BrowserEgressValidateRequest,
    ) -> tinybus::Result<BrowserEgressDecision> {
        let client = security_client().await?;
        let authorization = decode_browser_authorization(&request.authorization)?;
        let allowed = client
            .validate_browser_dial(tinysecurity_bus::ValidateBrowserDialRequest { authorization })
            .await
            .unwrap_or(false);
        Ok(BrowserEgressDecision { allowed })
    }
}

async fn security_client() -> tinybus::Result<super::security::Client> {
    let config = crate::core::runtime::DispatchCtx::ambient()
        .load_config()
        .await
        .map_err(|_| refused())?;
    super::security::client(&config)
        .await
        .map_err(|_| refused())
}

fn browser_request(target: BrowserEgressTarget) -> tinysecurity_bus::BrowserDialRequest {
    tinysecurity_bus::BrowserDialRequest {
        policy_id: tinysecurity_bus::EgressPolicyId(target.policy_id),
        expected_generation: target.expected_generation,
        session_id: target.session_id,
        scheme: target.scheme,
        hostname: target.hostname,
        port: target.port,
    }
}

fn decode_browser_authorization(
    value: &str,
) -> tinybus::Result<tinysecurity_bus::BrowserDialAuthorization> {
    serde_json::from_str(value).map_err(|_| refused())
}

fn refused() -> tinybus::Error {
    tinybus::Error::failed("TinySecurity browser egress authorization unavailable")
}

struct ComputerOnly<T> {
    inner: T,
    connection: Connection,
}

#[async_trait]
impl<T: Interface> Interface for ComputerOnly<T> {
    fn name(&self) -> InterfaceName {
        self.inner.name()
    }
    fn members(&self) -> Vec<MemberName> {
        self.inner.members()
    }
    fn requires_confidential(&self, member: &MemberName) -> bool {
        self.inner.requires_confidential(member)
    }
    async fn call(&self, _member: &MemberName, _args: Value) -> tinybus::Result<Value> {
        Err(refused())
    }
    async fn call_with_context(
        &self,
        member: &MemberName,
        args: Value,
        context: &CallContext,
    ) -> tinybus::Result<Value> {
        let owner: Option<BusName> = self
            .connection
            .name_owner(tinycomputer_bus::names::INTERFACE)
            .await?;
        if owner.is_none() || context.authenticated_sender() != owner.as_ref() {
            return Err(refused());
        }
        self.inner.call(member, args).await
    }
}

pub(super) async fn install(connection: &Connection) -> tinybus::Result<()> {
    connection
        .serve_at(
            ObjectPath::new(contract::OBJECT_PATH)?,
            ComputerOnly {
                inner: BrowserEgressHost,
                connection: connection.clone(),
            },
        )
        .await?;
    connection.request_name(contract::INTERFACE).await
}

/// Prepare TinyComputer's authenticated, confidential consequence callback.
/// TinyBus permits confidential delivery only to this exact callback recipient
/// bound to the verified module instance; the ordinary host connection is not
/// an eligible recipient.
pub(crate) async fn prepare_consequence_callback(
    runtime: &tinybus::module::ModuleRuntime,
) -> tinybus::Result<std::sync::Arc<tinybus::module::HostCallbackBinding>> {
    let binding = std::sync::Arc::new(
        runtime
            .host()
            .prepare_host_callbacks(vec![tinybus::module::HostCallbackEndpoint {
                path: consequence_host::OBJECT_PATH.parse()?,
                interface: consequence_host::INTERFACE.parse()?,
                members: vec![consequence_host::methods::REVIEW.parse()?],
            }])
            .await?,
    );
    binding
        .serve_at(
            consequence_host::OBJECT_PATH.parse()?,
            ConsequenceReviewHost,
        )
        .await?;
    Ok(binding)
}

#[cfg(test)]
#[path = "browser_egress_host_tests.rs"]
mod tests;
