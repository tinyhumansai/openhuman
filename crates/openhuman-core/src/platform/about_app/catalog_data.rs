use super::super::types::{
    Capability, CapabilityCategory, CapabilityPrivacy, CapabilityStatus, PrivacyDataKind,
};

const LOCAL_RAW: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: false,
    data_kind: PrivacyDataKind::Raw,
    destinations: &[],
});

// Memory v2 always runs on a remote engine: it receives the documents, conversations
// and learnings the assistant stores and the recall/fetch queries it runs.
const MEMORY_TO_REMOTE_ENGINE: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &[
        "TinyHumans-hosted CortexDB (when selected)",
        "User-configured CortexDB service (when selected)",
    ],
});

const DESKTOP_TO_JEV: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &[
        "Configured OpenHuman inference provider",
        "TinyHumans OpenRouter Jev proxy",
    ],
});

const DERIVED_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Derived,
    destinations: &["OpenHuman backend", "TinyHumans Neocortex"],
});

const RAW_TO_INFERENCE_PROVIDER: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &["Configured OpenHuman inference provider"],
});

// AGENTS.md instruction layers are injected verbatim into the agent's system
// prompt, which is sent to whichever inference provider is configured (the
// managed cloud default or a user-selected remote model). The raw file content
// therefore leaves the device whenever a remote provider is active —
// `LOCAL_RAW` (leaves_device: false) under-reported this. Same shape as
// `RAW_TO_INFERENCE_PROVIDER`: raw payload to the configured provider.
const AGENTS_MD_TO_INFERENCE_PROVIDER: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &["Configured OpenHuman inference provider"],
});

// Vision sub-agent ships the attached image (raw pixels) to the managed
// multimodal model for analysis.
const IMAGE_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &["OpenHuman backend", "TinyHumans Neocortex"],
});

// Media generation sends the prompt (and any reference image URL) to GMI Cloud
// via the OpenHuman backend; generated media is downloaded back to the device.
const MEDIA_GEN_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &["OpenHuman backend", "GMI Cloud"],
});

const LOCAL_CREDENTIALS: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: false,
    data_kind: PrivacyDataKind::Credentials,
    destinations: &[],
});

const DIAGNOSTICS_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Diagnostics,
    destinations: &["OpenHuman backend"],
});

const MODEL_DOWNLOAD: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["Hugging Face"],
});

// Self-update flows talk to GitHub Releases directly, not the OpenHuman
// backend. The outbound payload is metadata only (release list query for
// `update.check`, asset download URL request for `update.apply`) so
// `data_kind: Metadata` is the right label — but the destination must
// reflect that this is a third-party host, otherwise the capability
// catalog under-reports where the user's request actually goes.
const GITHUB_RELEASES_METADATA: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["GitHub Releases"],
});

// Persona Pack fetches the published mascot manifest directly from GitHub raw
// content, then downloads the selected runtime asset from the manifest's
// declared file URL. The request is metadata-class (manifest and asset URLs),
// but it does leave the device and bypasses the managed backend.
const GITHUB_MASCOT_MANIFEST: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &[
        "GitHub raw content (raw.githubusercontent.com) and manifest-declared mascot asset hosts",
    ],
});

const SEARXNG_RAW_TO_CONFIGURED_INSTANCE: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &["Configured SearXNG instance"],
});

// Direct-mode Composio: the user's API key and tool arguments leave the
// device — they are sent to backend.composio.dev, not the OpenHuman backend.
// LOCAL_CREDENTIALS was incorrect here because leaves_device must be true.
const COMPOSIO_DIRECT_CREDENTIALS: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Credentials,
    destinations: &["Composio (backend.composio.dev)"],
});

// "Test Connection" on the Embeddings settings panel routes a small probe
// payload to *whichever provider the user has selected* — not just the
// managed cloud default. `DERIVED_TO_BACKEND` only enumerates the managed
// path (OpenHuman backend / Neocortex), which under-reports the actual
// privacy surface when the user has switched to OpenAI / Cohere / a
// self-hosted endpoint. The catalog needs to list every reachable
// destination so the Privacy surface can render the full set instead of
// implying probes always stay on the managed path.
const EMBEDDING_PROBE_TO_CONFIGURED_PROVIDER: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Derived,
    destinations: &[
        "OpenHuman backend / TinyHumans Neocortex (managed cloud default)",
        "OpenAI API (api.openai.com)",
        "Cohere API (api.cohere.com)",
        "User-configured OpenAI-compatible endpoint (custom:<url>)",
    ],
});

use std::sync::LazyLock;

#[path = "catalog_auth_channels_team.rs"]
mod catalog_auth_channels_team;
#[path = "catalog_conversation_intelligence.rs"]
mod catalog_conversation_intelligence;
#[path = "catalog_localai_settings_mobile.rs"]
mod catalog_localai_settings_mobile;
#[path = "catalog_workflows_automation.rs"]
mod catalog_workflows_automation;

pub(super) static CAPABILITIES: LazyLock<Vec<Capability>> = LazyLock::new(|| {
    [
        catalog_conversation_intelligence::CAPABILITIES,
        catalog_workflows_automation::CAPABILITIES,
        catalog_auth_channels_team::CAPABILITIES,
        catalog_localai_settings_mobile::CAPABILITIES,
    ]
    .concat()
});
