//! Typed, attested calls to the TinySecurity policy interface.

use serde::{de::DeserializeOwned, Serialize};
use tinysecurity_bus::{
    BudgetDecision2, BudgetRequest2, CheckRequest, CheckResponse, CommandDecision, CommandPolicy,
    CommandPolicyId, CommandRequest, CredentialRootsRequest2, CredentialRootsResponse2, Decision,
    HostActionClassRequest2, HostActionClassResponse2, KeyringRequest2, KeyringResponse2,
    PathBatchRequest2, PathBatchResponse2, PathPolicy, PathPolicyId, PathValidationRequest,
    PathValidationResult, PolicyInfo, PrivacyCheckRequest2, PrivacyCheckResponse2, ProtectedFloor2,
    ProtectedPathRequest2, ProtectedPathResponse2, RedactMode, RedactRequest, RedactResponse,
    RedactWithRequest, ScanPromptScoredRequest2, ScanPromptScoredResponse2, ScanResponse,
    ScanToolDefinitionRequest, Secret,
};

use super::MODULE_ID;
use crate::config::Config;

/// Sanitized failure from the security module boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecurityError {
    /// The module is absent, disabled, faulted, or failed to answer.
    Unavailable(String),
}

impl std::fmt::Display for SecurityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "security module unavailable: {reason}"),
        }
    }
}

impl std::error::Error for SecurityError {}

/// A client whose object path comes from the compiled registry record.
#[derive(Clone)]
pub struct Client {
    object: SecurityProxy,
}

/// One admitted security API connection. Production calls travel through the
/// callback binding so TinySecurity sees the same authenticated host peer for
/// its API calls and confidential host callbacks; test fixtures retain the
/// regular module client.
#[derive(Clone)]
pub(crate) enum SecurityProxy {
    Callback(tinybus::module::HostCallbackProxy),
    Module(tinybus::module::ModuleObjectClient),
}

impl SecurityProxy {
    pub(crate) fn callback(proxy: tinybus::module::HostCallbackProxy) -> Self {
        Self::Callback(proxy)
    }

    #[cfg(test)]
    pub(crate) fn module(proxy: tinybus::module::ModuleObjectClient) -> Self {
        Self::Module(proxy)
    }

    pub(crate) async fn call_confidential<R: DeserializeOwned>(
        &self,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, String> {
        match self {
            Self::Callback(proxy) => proxy
                .call_confidential(member, args)
                .await
                .map_err(|_| "security module call failed".to_owned()),
            Self::Module(proxy) => proxy
                .call_confidential(member, args)
                .await
                .map_err(|_| "security module call failed".to_owned()),
        }
    }

    pub(crate) async fn call<R: DeserializeOwned>(
        &self,
        member: &str,
        args: impl Serialize,
    ) -> Result<R, String> {
        match self {
            Self::Callback(proxy) => proxy
                .call(member, args)
                .await
                .map_err(|_| "security module call failed".to_owned()),
            Self::Module(proxy) => proxy
                .call(member, args)
                .await
                .map_err(|_| "security module call failed".to_owned()),
        }
    }
}

impl Client {
    pub(crate) async fn connect(config: &Config) -> Result<Self, String> {
        super::super::ops::ensure_loaded(config, MODULE_ID).await?;
        let binding = super::super::ops::security_callback_binding()
            .await
            .ok_or_else(|| "security callback binding unavailable".to_owned())?;
        let record = super::super::registry::find(MODULE_ID)
            .ok_or_else(|| "security module registry record missing".to_owned())?;
        let proxy = binding
            .proxy(
                record.bus_name,
                record.object_path,
                tinysecurity_bus::names::INTERFACE,
            )
            .map_err(|_| "pinned security module proxy unavailable".to_owned())?;
        let object = SecurityProxy::callback(proxy);
        Ok(Self { object })
    }

    #[cfg(test)]
    fn from_object(object: tinybus::module::ModuleObjectClient) -> Self {
        Self {
            object: SecurityProxy::Module(object),
        }
    }

    /// Run a paired-device lifecycle operation inside TinySecurity.
    ///
    /// The operation is confidential because it may carry a handshake frame,
    /// an opaque pairing capability, or a host event identifier.
    pub async fn device_pairing2(
        &self,
        request: tinysecurity_bus::DevicePairingRequest2,
    ) -> Result<tinysecurity_bus::DevicePairingResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::DEVICE_PAIRING2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Admit the exact native approval scope into TinySecurity's paired-device
    /// owner. The module derives its secret and persistence bindings from the
    /// scope already registered at initialization.
    pub async fn register_device_pairing_scope2(
        &self,
        request: tinysecurity_bus::RegisterDevicePairingScopeRequest2,
    ) -> Result<tinysecurity_bus::DevicePairingScopeReceipt2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::REGISTER_DEVICE_PAIRING_SCOPE2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Start an audited HumanOnce review for one native device operation.
    pub async fn evaluate_device2(
        &self,
        request: tinysecurity_bus::EvaluateDeviceRequest2,
    ) -> Result<tinysecurity_bus::ApprovalRow2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::EVALUATE_DEVICE2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Execute only the exact native device operation reviewed by TinySecurity.
    pub async fn execute_device2(
        &self,
        request: tinysecurity_bus::ExecuteDeviceRequest2,
    ) -> Result<tinysecurity_bus::DeviceResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::EXECUTE_DEVICE2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Seal or open one live tunnel frame using TinySecurity's opaque session.
    pub async fn device_frame2(
        &self,
        request: tinysecurity_bus::DeviceFrameRequest2,
    ) -> Result<tinysecurity_bus::DeviceFrameResponse2, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::DEVICE_FRAME2, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Read effective settings and advertised capabilities from the module.
    pub async fn policy_info(&self) -> Result<PolicyInfo, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::POLICY_INFO, ())
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Redact text through TinySecurity's shared policy engine.
    pub async fn redact(
        &self,
        mode: RedactMode,
        text: impl Into<String>,
    ) -> Result<RedactResponse, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::REDACT,
                (RedactRequest {
                    text: text.into(),
                    mode,
                },),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Redact text using exact credentials that apply to this call only.
    pub async fn redact_with(
        &self,
        mode: RedactMode,
        text: impl Into<String>,
        secrets: impl IntoIterator<Item = String>,
    ) -> Result<RedactResponse, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::REDACT_WITH,
                (RedactWithRequest {
                    text: text.into(),
                    mode,
                    secrets: secrets.into_iter().map(Secret).collect(),
                },),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Score prompt text through TinySecurity. The result is context, never authorization.
    pub async fn scan_prompt2(
        &self,
        text: impl Into<String>,
    ) -> Result<ScanPromptScoredResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::SCAN_PROMPT2,
                (ScanPromptScoredRequest2 { text: text.into() },),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Scan a complete tool definition encoded as a JSON object.
    pub async fn scan_tool_definition(
        &self,
        name: impl Into<String>,
        definition: &serde_json::Value,
    ) -> Result<ScanResponse, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::SCAN_TOOL_DEFINITION,
                (ScanToolDefinitionRequest {
                    name: name.into(),
                    definition_json: definition.to_string(),
                },),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Ask the module for a policy decision. A missing or malformed response
    /// remains an error for the caller to deny.
    pub async fn check(&self, request: &CheckRequest) -> Result<CheckResponse, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::CHECK, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Register an immutable host-derived path scope.
    pub async fn register_path_policy(
        &self,
        policy: PathPolicy,
    ) -> Result<PathPolicyId, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::REGISTER_PATH_POLICY,
                (policy,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Check a batch of paths against a registered path scope.
    pub async fn check_paths2(
        &self,
        request: PathBatchRequest2,
    ) -> Result<PathBatchResponse2, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::CHECK_PATHS2, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Resolve and authorize an existing path under a registered path scope.
    pub async fn validate_path(
        &self,
        request: PathValidationRequest,
    ) -> Result<PathValidationResult, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::VALIDATE_PATH, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Resolve and authorize the existing parent of a destination path.
    pub async fn validate_parent(
        &self,
        request: PathValidationRequest,
    ) -> Result<PathValidationResult, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::VALIDATE_PARENT,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Query the unconditional path floor without a registered scope.
    pub async fn protected_path2(
        &self,
        request: ProtectedPathRequest2,
    ) -> Result<ProtectedPathResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::PROTECTED_PATH2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Read TinySecurity's protected-path vocabulary.
    pub async fn protected_floor2(&self) -> Result<ProtectedFloor2, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::PROTECTED_FLOOR2, ())
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Resolve TinySecurity's credential-store roots for a trusted home path.
    pub async fn credential_roots2(
        &self,
        request: CredentialRootsRequest2,
    ) -> Result<CredentialRootsResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::CREDENTIAL_ROOTS2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Classify a host tool from its trusted permission and effect declarations.
    pub async fn classify_host_action2(
        &self,
        request: HostActionClassRequest2,
    ) -> Result<HostActionClassResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::CLASSIFY_HOST_ACTION2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Ask TinySecurity whether one descriptor-level egress transfer is allowed.
    pub async fn privacy_check2(
        &self,
        request: PrivacyCheckRequest2,
    ) -> Result<Decision, SecurityError> {
        let response: PrivacyCheckResponse2 = self
            .object
            .call_confidential(tinysecurity_bus::names::methods::PRIVACY_CHECK2, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))?;
        Ok(response.decision)
    }

    pub(crate) async fn classify_provider_endpoint2(
        &self,
        host: String,
    ) -> Result<tinysecurity_bus::ProviderEndpointResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::CLASSIFY_PROVIDER_ENDPOINT2,
                (tinysecurity_bus::ProviderEndpointRequest2 { host },),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Authorize one browser socket dial through TinySecurity.
    pub(crate) async fn authorize_browser_dial(
        &self,
        request: tinysecurity_bus::BrowserDialRequest,
    ) -> Result<tinysecurity_bus::BrowserDialResponse, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::AUTHORIZE_BROWSER_DIAL,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    pub(crate) async fn register_egress_policy(
        &self,
        policy: tinysecurity_bus::EgressPolicy,
    ) -> Result<tinysecurity_bus::EgressPolicyId, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::REGISTER_EGRESS_POLICY,
                (policy,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Consume the exact one-use browser grant before the selected socket opens.
    pub(crate) async fn consume_browser_dial(
        &self,
        request: tinysecurity_bus::ConsumeBrowserDialRequest,
    ) -> Result<bool, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::CONSUME_BROWSER_DIAL,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Perform one OS keyring operation through TinySecurity. The module
    /// obtains fresh, operation-scoped consent before touching the backend.
    pub async fn keyring_access2(
        &self,
        request: KeyringRequest2,
    ) -> Result<KeyringResponse2, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::KEYRING_ACCESS2,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Persist intent for one routine config cipher operation.
    pub(crate) async fn evaluate_secret_cipher2(
        &self,
        request: &mut tinysecurity_bus::EvaluateSecretCipherRequest2,
    ) -> Result<tinysecurity_bus::SecretCipherPending2, SecurityError> {
        let result = self
            .object
            .call_confidential(
                tinysecurity_bus::names::methods::EVALUATE_SECRET_CIPHER2,
                (&*request,),
            )
            .await
            .map_err(|_| SecurityError::Unavailable("secret cipher evaluation failed".into()));
        use zeroize::Zeroize;
        request.request.input.zeroize();
        result
    }

    /// Execute a previously evaluated routine config cipher operation.
    pub(crate) async fn execute_secret_cipher2(
        &self,
        request: &mut tinysecurity_bus::ExecuteSecretCipherRequest2,
    ) -> Result<tinysecurity_bus::SecretCipherResponse2, SecurityError> {
        let result = self
            .object
            .call_confidential(
                tinysecurity_bus::names::methods::EXECUTE_SECRET_CIPHER2,
                (&*request,),
            )
            .await
            .map_err(|_| SecurityError::Unavailable("secret cipher execution failed".into()));
        use zeroize::Zeroize;
        request.request.input.zeroize();
        result
    }

    /// Request the existing native HumanOnce approval flow for a registered
    /// master slot. TinySecurity owns the pending row and verifies the decision.
    pub(crate) async fn evaluate_secret2(
        &self,
        request: &mut tinysecurity_bus::EvaluateSecretRequest2,
    ) -> Result<tinysecurity_bus::ApprovalRow2, SecurityError> {
        let result = self
            .object
            .call_confidential(
                tinysecurity_bus::names::methods::EVALUATE_SECRET2,
                (&*request,),
            )
            .await
            .map_err(|_| SecurityError::Unavailable("secret approval unavailable".into()));
        result
    }

    /// Complete the native master grant after the host's existing approval
    /// engine reports HumanOnce. The returned handle is process-private.
    pub(crate) async fn execute_secret2(
        &self,
        request: &mut tinysecurity_bus::ExecuteSecretRequest2,
    ) -> Result<tinysecurity_bus::SecretResponse2, SecurityError> {
        let result = self
            .object
            .call_confidential(
                tinysecurity_bus::names::methods::EXECUTE_SECRET2,
                (&*request,),
            )
            .await
            .map_err(|_| SecurityError::Unavailable("secret master unavailable".into()));
        result
    }

    /// Validate a live browser tunnel lease.
    pub(crate) async fn validate_browser_dial(
        &self,
        request: tinysecurity_bus::ValidateBrowserDialRequest,
    ) -> Result<bool, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::VALIDATE_BROWSER_DIAL,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Register an immutable host-derived command scope.
    pub async fn register_command_policy(
        &self,
        policy: CommandPolicy,
    ) -> Result<CommandPolicyId, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::REGISTER_COMMAND_POLICY,
                (policy,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Classify an actual command without spending its action budget.
    pub async fn classify_command(
        &self,
        request: CommandRequest,
    ) -> Result<CommandDecision, SecurityError> {
        self.object
            .call_confidential(
                tinysecurity_bus::names::methods::CLASSIFY_COMMAND,
                (request,),
            )
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Check an actual command and reserve budget when TinySecurity allows it.
    pub async fn check_command(
        &self,
        request: CommandRequest,
    ) -> Result<CommandDecision, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::CHECK_COMMAND, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Spend one action from a registered command scope.
    pub async fn spend_action2(
        &self,
        request: BudgetRequest2,
    ) -> Result<BudgetDecision2, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::SPEND_ACTION2, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }

    /// Read a command scope's budget without spending an action.
    pub async fn peek_budget2(
        &self,
        request: BudgetRequest2,
    ) -> Result<BudgetDecision2, SecurityError> {
        self.object
            .call_confidential(tinysecurity_bus::names::methods::PEEK_BUDGET2, (request,))
            .await
            .map_err(|error| SecurityError::Unavailable(error.to_string()))
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
