//! OpenRouter media generators reached through the TinyHumans backend's
//! `/agent-integrations/openrouter` proxy.
//!
//! TinyInference owns the wire contract (`POST /images`, `POST /videos`,
//! `GET /videos/{id}`, `GET /videos/{id}/content`); this module supplies only
//! what the host owns:
//!
//! - **Endpoint and headers** — the hosted client's `raw_client()` (which
//!   already carries `x-sdk-name` and the version headers) and the
//!   `/agent-integrations/openrouter` base URL.
//! - **Credential** — a per-request resolver over
//!   [`resolve_backend_credential`], so a desktop session JWT and a library
//!   API key both work and a refreshed session is picked up.
//! - **Policy** — local-only enforcement and the egress disclosure before any
//!   request leaves the device, and the managed-credit budget gate before a
//!   billed submit. These are the same gates `IntegrationClient` applies to
//!   every `/agent-integrations/*` call.

use std::sync::Arc;

use async_trait::async_trait;
use tinyinference_image::{
    self as ti_image, BearerResolver, GeneratedMedia, ImageGenerator, ImageRequest, ImageResponse,
    MediaAuth, MediaModel, MediaTransport, OpenRouterImageGenerator,
};
use tinyinference_video::{
    self as ti_video, OpenRouterVideoGenerator, VideoGenerator, VideoJob, VideoJobStatus,
    VideoRequest,
};

use crate::client::BackendClient;
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::media::generation::MediaGenerators;
use openhuman_embed::__host::security::approval::ApprovalChatContext;
use openhuman_embed::__host::security::credentials::session_support::{
    resolve_backend_credential, BackendCredential,
};

/// Backend route prefix that proxies OpenRouter's media API.
pub const OPENROUTER_PROXY_PATH: &str = "/agent-integrations/openrouter";

/// Builds generators against the managed backend, or `None` when no backend
/// transport is installed (a core with no TinyHumans connection).
pub fn managed_generators(config: &Config) -> Option<MediaGenerators> {
    let client = match BackendClient::from_config(config) {
        Ok(client) => client,
        Err(error) => {
            tracing::debug!(%error, "[media_generation] no backend client; media tools skipped");
            return None;
        }
    };
    let http = match client.raw_client() {
        Ok(http) => http,
        Err(error) => {
            tracing::debug!(%error, "[media_generation] no backend transport; media tools skipped");
            return None;
        }
    };
    let base = match client.url_for(OPENROUTER_PROXY_PATH) {
        Ok(url) => url,
        Err(error) => {
            tracing::debug!(%error, "[media_generation] cannot build proxy URL; media tools skipped");
            return None;
        }
    };
    let config = Arc::new(config.clone());
    let transport = MediaTransport::new(MediaAuth::Bearer(bearer_resolver(Arc::clone(&config))))
        .with_client(http)
        .with_base_url(base.as_str());
    tracing::debug!(base = %base, "[media_generation] managed OpenRouter media generators ready");
    // The generators are built once per chat binding: `routed` rebuilds them
    // around a guard that discloses to that chat.
    let build = Arc::new(move |chat: Option<ApprovalChatContext>| {
        let image: Arc<dyn ImageGenerator> = Arc::new(GuardedImage {
            inner: OpenRouterImageGenerator::with_transport(transport.clone()),
            guard: Guard {
                config: Arc::clone(&config),
                chat: chat.clone(),
            },
        });
        let video: Arc<dyn VideoGenerator> = Arc::new(GuardedVideo {
            inner: OpenRouterVideoGenerator::with_transport(transport.clone()),
            guard: Guard {
                config: Arc::clone(&config),
                chat,
            },
        });
        (image, video)
    });
    let (image, video) = build(None);
    Some(MediaGenerators::with_router(image, video, build))
}

/// Resolves the backend credential on every request, so a session refreshed
/// mid-run is used and an API-key host needs no session at all.
pub(crate) fn bearer_resolver(config: Arc<Config>) -> BearerResolver {
    Arc::new(move || match resolve_backend_credential(&config) {
        Ok(BackendCredential::Session(token) | BackendCredential::ApiKey(token)) => Ok(token),
        Err(error) => Err(ti_image::Error::Auth(error)),
    })
}

/// Host policy applied before a request leaves the device.
struct Guard {
    config: Arc<Config>,
    /// The chat the call's disclosures are surfaced to; `None` outside a chat
    /// turn.
    chat: Option<ApprovalChatContext>,
}

impl Guard {
    /// Local-only enforcement, then the egress disclosure. A blocked call is
    /// neither disclosed nor sent.
    fn admit(&self, route: &str) -> ti_image::Result<()> {
        let descriptor = crate::integrations::client::requests::backend_egress_descriptor(
            &format!("{OPENROUTER_PROXY_PATH}/{route}"),
        );
        openhuman_embed::__host::egress::enforce_egress(&descriptor).map_err(|error| {
            tracing::info!(route, %error, "[media_generation] blocked by privacy policy");
            ti_image::Error::Validation(format!("blocked by the privacy policy: {error}"))
        })?;
        openhuman_embed::__host::egress::emit_external_transfer(self.chat.as_ref(), descriptor);
        Ok(())
    }

    /// Refuses a billed submit when managed credits are exhausted.
    async fn budget(&self) -> ti_image::Result<()> {
        if crate::integrations::client::budget_gate::managed_tool_budget_exhausted(&self.config)
            .await
        {
            tracing::info!("[media_generation] managed credits exhausted; submit refused");
            return Err(ti_image::Error::Validation(
                "Managed cloud tools are disabled because your OpenHuman AI credits are exhausted. \
                 Add credits or route the task to user-supplied providers."
                    .into(),
            ));
        }
        Ok(())
    }
}

struct GuardedImage {
    inner: OpenRouterImageGenerator,
    guard: Guard,
}

#[async_trait]
impl ImageGenerator for GuardedImage {
    fn name(&self) -> &str {
        "openhuman-openrouter"
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    async fn generate(&self, request: ImageRequest) -> ti_image::Result<ImageResponse> {
        self.guard.admit("images")?;
        self.guard.budget().await?;
        self.inner.generate(request).await
    }

    async fn list_models(&self) -> ti_image::Result<Vec<MediaModel>> {
        self.guard.admit("images/models")?;
        self.inner.list_models().await
    }
}

struct GuardedVideo {
    inner: OpenRouterVideoGenerator,
    guard: Guard,
}

#[async_trait]
impl VideoGenerator for GuardedVideo {
    fn name(&self) -> &str {
        "openhuman-openrouter"
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    async fn submit(&self, request: VideoRequest) -> ti_video::Result<VideoJob> {
        self.guard.admit("videos")?;
        self.guard.budget().await?;
        self.inner.submit(request).await
    }

    async fn poll(&self, job_id: &str) -> ti_video::Result<VideoJobStatus> {
        self.guard.admit("videos/{jobId}")?;
        self.inner.poll(job_id).await
    }

    async fn content(&self, job_id: &str, index: usize) -> ti_video::Result<GeneratedMedia> {
        self.guard.admit("videos/{jobId}/content")?;
        self.inner.content(job_id, index).await
    }

    async fn list_models(&self) -> ti_video::Result<Vec<MediaModel>> {
        self.guard.admit("videos/models")?;
        self.inner.list_models().await
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
