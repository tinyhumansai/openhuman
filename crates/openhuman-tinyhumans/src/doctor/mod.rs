//! Read-only doctor checks for the TinyHumans-hosted backend.

use std::sync::Arc;

use async_trait::async_trait;
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::credentials::api_key::get_api_key_read_only;
use openhuman_embed::__host::credentials::backend_credential::BackendCredential;
use openhuman_embed::__host::credentials::session_state::{
    classify_session_token, is_local_session_token, load_app_session_profile_read_only,
    session_token_for_config, SessionTokenCheck,
};
use openhuman_embed::__host::doctor::{
    register_check_source, register_check_source_in, ComponentId, DoctorCheck, DoctorCheckSource,
    DoctorCtx, DoctorTier, Finding, FindingSeverity,
};
use openhuman_embed::__host::platform::socket::{global_socket_manager, ConnectionStatus};
use tinyhumans_sdk::{Error as SdkError, TinyHumansClient};

use crate::backend::product::product_identity_headers;
use crate::backend::url::{
    effective_backend_api_url, looks_like_inference_provider_endpoint, looks_like_local_ai_endpoint,
};
use crate::client::shared_transport;
use crate::hosted::client::{CredentialKind, HostedClient};
use crate::transport::{BackendTransport, TransportProfile};

const SOURCE_ID: &str = "tinyhumans";
const HEALTH_FIX: &str = "Check the configured backend URL and network access, then rerun doctor.";
const AUTH_FIX: &str = "Sign in again through the supported auth flow, then rerun doctor.";
const CATALOG_FIX: &str = "Check managed inference access for this profile, then rerun doctor.";

type SocketStateReader = Arc<dyn Fn(&str, &str) -> Option<ConnectionStatus> + Send + Sync>;

/// The stateless group of TinyHumans checks registered by this crate.
pub(crate) struct HostedDoctorCheckSource {
    socket_state: SocketStateReader,
}

impl Default for HostedDoctorCheckSource {
    fn default() -> Self {
        Self {
            socket_state: Arc::new(|url, token| {
                global_socket_manager().and_then(|manager| manager.status_for(url, token))
            }),
        }
    }
}

impl HostedDoctorCheckSource {
    #[cfg(test)]
    fn with_socket_state_reader(
        reader: impl Fn(&str, &str) -> Option<ConnectionStatus> + Send + Sync + 'static,
    ) -> Self {
        Self {
            socket_state: Arc::new(reader),
        }
    }
}

impl DoctorCheckSource for HostedDoctorCheckSource {
    fn id(&self) -> &str {
        SOURCE_ID
    }

    fn checks(&self) -> Vec<Arc<dyn DoctorCheck>> {
        vec![
            Arc::new(BackendReachable),
            Arc::new(BackendCredentialCheck),
            Arc::new(BackendIdentity),
            Arc::new(BackendSocket(Arc::clone(&self.socket_state))),
            Arc::new(BackendApiUrl),
            Arc::new(ManagedCatalog),
        ]
    }
}

pub(crate) fn register(registration: crate::register::DoctorRegistration<'_>) {
    let source = HostedDoctorCheckSource::default();
    match registration {
        crate::register::DoctorRegistration::Process => register_check_source(source),
        crate::register::DoctorRegistration::Context(extensions) => {
            register_check_source_in(extensions, source)
        }
    }
}

struct BackendReachable;

#[async_trait]
impl DoctorCheck for BackendReachable {
    fn id(&self) -> &str {
        "backend/reachable"
    }

    fn category(&self) -> &str {
        "Backend"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Hosting)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        vec![match health_check(ctx.config()).await {
            Ok(()) => Finding::pass(self.id(), "The hosted backend is reachable."),
            Err(HealthFailure::Status(status)) => Finding::error(
                self.id(),
                format!("The hosted backend health check returned HTTP {status}."),
                HEALTH_FIX,
            ),
            Err(HealthFailure::Transport) => Finding::error(
                self.id(),
                "The hosted backend health check could not complete.",
                HEALTH_FIX,
            ),
        }]
    }
}

struct BackendCredentialCheck;

#[async_trait]
impl DoctorCheck for BackendCredentialCheck {
    fn id(&self) -> &str {
        "backend/credential"
    }

    fn category(&self) -> &str {
        "Backend"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Hosting)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        vec![match doctor_credential(ctx.config()).await {
            CredentialState::Usable(_) => {
                Finding::pass(self.id(), "A usable hosted credential is configured.")
            }
            CredentialState::Expired => Finding::error(
                self.id(),
                "SESSION_EXPIRED: the backend session has expired.",
                AUTH_FIX,
            ),
            CredentialState::Local => Finding::skipped(
                self.id(),
                "This profile uses a local-only session without hosted backend access.",
                "Sign in with a TinyHumans account to run hosted checks.",
            ),
            CredentialState::Absent => Finding::skipped(
                self.id(),
                "No usable hosted credential is configured.",
                "Sign in with a TinyHumans account to run hosted checks.",
            ),
            CredentialState::BackendMismatch => Finding::error(
                self.id(),
                "SESSION_BACKEND_MISMATCH: the stored session belongs to a different backend.",
                "Restore the backend that issued this session or sign in to the configured backend.",
            ),
            CredentialState::Unreadable => Finding::error(
                self.id(),
                "The stored hosted credential could not be read safely.",
                AUTH_FIX,
            ),
        }]
    }
}

struct BackendIdentity;

#[async_trait]
impl DoctorCheck for BackendIdentity {
    fn id(&self) -> &str {
        "backend/identity"
    }

    fn category(&self) -> &str {
        "Backend"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Hosting)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let credential = match doctor_credential(ctx.config()).await {
            CredentialState::Usable(credential) => credential,
            _ => return vec![credential_skipped(self.id())],
        };
        if health_check(ctx.config()).await.is_err() {
            return vec![backend_dependent_skipped(self.id())];
        }
        let client = match hosted_client(ctx.config(), credential) {
            Ok(client) => client,
            Err(reason) => return vec![endpoint_refused(self.id(), &reason)],
        };
        vec![match client.sdk().auth().me().await {
            Ok(_) => Finding::pass(self.id(), "The backend accepted the configured identity."),
            Err(error) => auth_finding(self.id(), client.kind(), error, AUTH_FIX),
        }]
    }
}

struct BackendSocket(SocketStateReader);

#[async_trait]
impl DoctorCheck for BackendSocket {
    fn id(&self) -> &str {
        "backend/socket"
    }

    fn category(&self) -> &str {
        "Backend"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Hosting)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let Some(url) = ctx.config().resolved_endpoints.control_plane_url.as_deref() else {
            return vec![Finding::skipped(
                self.id(),
                "The selected profile has no resolved backend socket endpoint.",
                "Configure the hosted backend before checking its socket connection.",
            )];
        };
        let CredentialState::Usable(BackendCredential::Session(token)) =
            doctor_credential(ctx.config()).await
        else {
            return vec![Finding::skipped(
                self.id(),
                "The selected profile has no usable session for a backend socket check.",
                "Sign in with a TinyHumans account to run this check.",
            )];
        };
        let Some(status) = (self.0)(url, &token) else {
            return vec![Finding::skipped(
                self.id(),
                "No socket connection is active for the selected profile.",
                "Enable the socket service for this profile, then rerun doctor.",
            )];
        };

        let finding = match status {
            ConnectionStatus::Connected => {
                Finding::pass(self.id(), "The backend socket is connected.")
            }
            ConnectionStatus::Connecting => Finding::new(
                self.id(),
                FindingSeverity::Info,
                "The backend socket is connecting.",
                "No action required while the connection starts.",
            ),
            ConnectionStatus::Reconnecting => Finding::new(
                self.id(),
                FindingSeverity::Warn,
                "The backend socket is reconnecting.",
                HEALTH_FIX,
            ),
            ConnectionStatus::Disconnected => Finding::new(
                self.id(),
                FindingSeverity::Warn,
                "The backend socket is disconnected.",
                HEALTH_FIX,
            ),
            ConnectionStatus::Error => Finding::error(
                self.id(),
                "The backend socket reported a connection error.",
                HEALTH_FIX,
            ),
        };
        vec![finding]
    }
}

struct BackendApiUrl;

#[async_trait]
impl DoctorCheck for BackendApiUrl {
    fn id(&self) -> &str {
        "backend/api-url"
    }

    fn category(&self) -> &str {
        "Backend"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Hosting)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let configured = ctx.config().api_url.as_deref().unwrap_or_default().trim();
        if !configured.is_empty() {
            let Ok(parsed) = url::Url::parse(configured) else {
                return vec![Finding::error(
                    self.id(),
                    "The configured backend API URL is invalid.",
                    "Set api_url to an absolute HTTP or HTTPS backend URL.",
                )];
            };
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                return vec![Finding::error(
                    self.id(),
                    "The configured backend API URL must use HTTP or HTTPS and include a host.",
                    "Set api_url to an absolute HTTP or HTTPS backend URL.",
                )];
            }
            if looks_like_local_ai_endpoint(configured)
                || looks_like_inference_provider_endpoint(configured)
            {
                return vec![Finding::new(
                    self.id(),
                    FindingSeverity::Warn,
                    "The configured api_url looks like an inference endpoint; hosted requests use the backend default.",
                    "Set api_url to the hosted backend URL and configure model inference with inference_url.",
                )];
            }
        }

        let resolved = effective_backend_api_url(&ctx.config().api_url);
        match url::Url::parse(&resolved) {
            Ok(parsed)
                if matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some() =>
            {
                vec![Finding::pass(
                    self.id(),
                    "The hosted backend API URL is valid.",
                )]
            }
            _ => vec![Finding::error(
                self.id(),
                "The hosted backend API URL could not be resolved.",
                "Set api_url to an absolute HTTP or HTTPS backend URL.",
            )],
        }
    }
}

struct ManagedCatalog;

#[async_trait]
impl DoctorCheck for ManagedCatalog {
    fn id(&self) -> &str {
        "inference/managed-catalog"
    }

    fn category(&self) -> &str {
        "Inference"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Connect
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Hosting)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let credential = match doctor_credential(ctx.config()).await {
            CredentialState::Usable(credential) => credential,
            _ => return vec![credential_skipped(self.id())],
        };
        if health_check(ctx.config()).await.is_err() {
            return vec![backend_dependent_skipped(self.id())];
        }
        let client = match hosted_client(ctx.config(), credential) {
            Ok(client) => client,
            Err(reason) => return vec![endpoint_refused(self.id(), &reason)],
        };
        let query = [("catalog", Some("openrouter".to_owned()))];
        vec![match client.sdk().inference().list_models(&query).await {
            Ok(_) => Finding::pass(self.id(), "The managed inference catalog is reachable."),
            Err(error) => auth_finding(self.id(), client.kind(), error, CATALOG_FIX),
        }]
    }
}

enum CredentialState {
    Usable(BackendCredential),
    Expired,
    Local,
    Absent,
    BackendMismatch,
    Unreadable,
}

async fn doctor_credential(config: &Config) -> CredentialState {
    match get_api_key_read_only(config).await {
        Ok(Some(key)) => return CredentialState::Usable(BackendCredential::ApiKey(key)),
        Ok(None) => {}
        Err(_) => return CredentialState::Unreadable,
    }

    let profile = match load_app_session_profile_read_only(config).await {
        Ok(profile) => profile,
        Err(_) => return CredentialState::Unreadable,
    };
    let token = match session_token_for_config(config, profile.as_ref()) {
        Ok(Some(token)) => token,
        Ok(None) => return CredentialState::Absent,
        Err(error) if error.starts_with("SESSION_BACKEND_MISMATCH:") => {
            return CredentialState::BackendMismatch;
        }
        Err(_) => return CredentialState::Unreadable,
    };
    match classify_session_token(profile.as_ref(), chrono::Utc::now()) {
        SessionTokenCheck::Absent => CredentialState::Absent,
        SessionTokenCheck::Expired => CredentialState::Expired,
        SessionTokenCheck::Live(_) if is_local_session_token(&token) => CredentialState::Local,
        SessionTokenCheck::Live(_) => CredentialState::Usable(BackendCredential::Session(token)),
    }
}

fn hosted_client(config: &Config, credential: BackendCredential) -> Result<HostedClient, String> {
    HostedClient::with_credential(&effective_backend_api_url(&config.api_url), credential)
}

fn endpoint_refused(id: &str, reason: &str) -> Finding {
    Finding::error(
        id,
        format!("The hosted credential was not sent: {reason}."),
        "Point the API URL at the managed backend over HTTPS, or at a loopback endpoint for local testing.",
    )
}

fn credential_skipped(id: &str) -> Finding {
    Finding::skipped(
        id,
        "A usable hosted credential is required for this check.",
        "Sign in with a TinyHumans account to run hosted checks.",
    )
}

fn backend_dependent_skipped(id: &str) -> Finding {
    Finding::skipped(
        id,
        "This hosted check was skipped because the backend is unreachable.",
        HEALTH_FIX,
    )
}

fn auth_finding(id: &str, kind: CredentialKind, error: SdkError, fix_hint: &str) -> Finding {
    match error {
        SdkError::Status { status: 401, .. } if kind == CredentialKind::Session => Finding::error(
            id,
            "SESSION_EXPIRED: the backend rejected the session.",
            AUTH_FIX,
        ),
        SdkError::Status { status: 401, .. } if kind == CredentialKind::ApiKey => Finding::error(
            id,
            "API_KEY_REJECTED: the backend rejected the configured API key.",
            AUTH_FIX,
        ),
        SdkError::Status { status, .. } => Finding::error(
            id,
            format!("The hosted request returned HTTP {status}."),
            fix_hint,
        ),
        _ => Finding::error(id, "The hosted request could not complete.", fix_hint),
    }
}

enum HealthFailure {
    Status(u16),
    Transport,
}

async fn health_check(config: &Config) -> Result<(), HealthFailure> {
    let url = effective_backend_api_url(&config.api_url);
    let transport = shared_transport().map_err(|_| HealthFailure::Transport)?;
    let client = TinyHumansClient::new(url)
        .with_http_client(transport.http_client(TransportProfile::Api))
        .with_default_headers(product_identity_headers());
    match client.health().check().await {
        Ok(_) => Ok(()),
        Err(SdkError::Status { status, .. }) => Err(HealthFailure::Status(status)),
        Err(_) => Err(HealthFailure::Transport),
    }
}

#[cfg(test)]
#[path = "hosted_checks_tests.rs"]
mod tests;
