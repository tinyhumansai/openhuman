//! Registry record for the `tinyvoice` module.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

/// The `tinyvoice` module: the host-agnostic half of the voice pipeline.
///
/// Wake-word gating, fast-path command routing, STT hallucination detection,
/// and the capture-side audio work (downmix, resample, silence gate, WAV
/// framing).
///
/// Lazy, and more clearly so than the others: voice is opt-in twice over — a
/// user has to enable dictation or always-on listening before any of this runs
/// — so a session that never speaks should not pay a download or a `dlopen`.
///
/// Devices and audio processing live in the compiled module. Hosts poll bounded
/// capture batches and apply product permission, privacy and transcription policy.
pub(crate) const TINYVOICE: ModuleRecord = ModuleRecord {
    id: "tinyvoice",
    description: "Wake-word gating, command routing, hallucination detection, capture audio",
    bus_name: "ai.tinyhumans.tinyvoice.Voice",
    object_path: "/ai/tinyhumans/tinyvoice/Voice",
    version: "0.1.16",
    release_url: "https://github.com/tinyhumansai/tinyvoice/releases/tag/v0.1.16",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyvoice-module-0.1.16-ubuntu-24.04-x86_64.tar.gz",
            sha256: "90de2c9dce82e9d8e3864e9e0f827f1110599a3b5148d905a2841492145a1015",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyvoice-module-0.1.16-ubuntu-24.04-arm64.tar.gz",
            sha256: "620167d61a331ad1260f3f432552cca57af4a0696b51b3e7d8d62ad1f0337a06",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyvoice-module-0.1.16-ubuntu-22.04-x86_64.tar.gz",
            sha256: "9a43aedb59617d6322537c411f91710ffe31c9595066592e0abc6086aca13a1a",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyvoice-module-0.1.16-ubuntu-22.04-arm64.tar.gz",
            sha256: "be42ffdf401d10bdaac083b05c42ebbeeca4f25649286cd0ada608fa02045c23",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyvoice-module-0.1.16-macos-26-arm64.tar.gz",
            sha256: "8003a988fdc2eb4a6be724caf6d1ba16a97d3bbf7372921630bb180f02dfeb28",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyvoice-module-0.1.16-macos-26-x86_64.tar.gz",
            sha256: "20142bb6bb285b863b626d2d49c4879331de1f34cfc2847755d038adf5664abe",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyvoice-module-0.1.16-macos-15-arm64.tar.gz",
            sha256: "72c75b371e72c22da9a149b0962395d74131b41910e2ed735d127667f9f38c1d",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyvoice-module-0.1.16-macos-15-x86_64.tar.gz",
            sha256: "a293f24b14570ac551be300d1aac28eddb0339066ab4ec0a231b6831fade4734",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyvoice-module-0.1.16-windows-2025-x86_64.zip",
            sha256: "271ba8c8476c22535f9fba0086b0aa0a253f9d228cd6f6a73e91cbe24dfa8a2a",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyvoice-module-0.1.16-windows-2022-x86_64.zip",
            sha256: "89fc25bdddbcb77393f8618aa4f7f00f9cd9abd27dd1e35ad184aafb4fe4fb9d",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyvoice-module-0.1.16-windows-11-arm64.zip",
            sha256: "8272daea789109e6034ee3af5ad9a46e5559b511f9cf6c965c14225e1c59c69e",
        },
    ],
    load: LoadPolicy::Lazy,
};
