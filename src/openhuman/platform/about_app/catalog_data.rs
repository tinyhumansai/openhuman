use super::super::types::{
    Capability, CapabilityCategory, CapabilityPrivacy, CapabilityStatus, PrivacyDataKind,
};

const LOCAL_RAW: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: false,
    data_kind: PrivacyDataKind::Raw,
    destinations: &[],
});

const DERIVED_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Derived,
    destinations: &["OpenHuman backend", "TinyHumans Neocortex"],
});

const CODING_SESSION_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &["Configured OpenHuman inference provider"],
});

// AGENTS.md instruction layers are injected verbatim into the agent's system
// prompt, which is sent to whichever inference provider is configured (the
// managed cloud default or a user-selected remote model). The raw file content
// therefore leaves the device whenever a remote provider is active —
// `LOCAL_RAW` (leaves_device: false) under-reported this. Same shape as
// `CODING_SESSION_TO_BACKEND`: raw payload to the configured provider.
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

// GitHub repo memory source: the reader queries a repository's activity
// (commits / issues / PRs) directly against the GitHub API — via the `gh`
// CLI when available, otherwise the public REST API — not through the
// OpenHuman backend. The *outbound* payload is metadata (which repo, which
// activity, pagination) plus whatever auth `gh` carries; the fetched content
// is archived locally under the vault and only its embeddings travel onward
// (covered by the embedding-provider capability). Mirrors the
// `GITHUB_RELEASES_METADATA` shape — third-party GitHub host, metadata-class
// outbound — so the Privacy surface reflects that the request leaves the
// device to a destination distinct from the managed backend.
const GITHUB_REPO_SOURCE: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["GitHub API (api.github.com)"],
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

#[path = "catalog_part_01.rs"]
mod catalog_part_01;
#[path = "catalog_part_02.rs"]
mod catalog_part_02;
#[path = "catalog_part_03.rs"]
mod catalog_part_03;

pub(super) static CAPABILITIES: LazyLock<Vec<Capability>> = LazyLock::new(|| {
    [
        catalog_part_01::CAPABILITIES,
        catalog_part_02::CAPABILITIES,
        catalog_part_03::CAPABILITIES,
    ]
    .concat()
});
