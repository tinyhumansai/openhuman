//! Registry record for the `tinyeventqueues` module.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

/// The `tinyeventqueues` module: durable inbound event queues. It persists,
/// deduplicates, filters and paces inbound events (Composio triggers, agent
/// webhooks) on the host's storage, loaded on first ingest, and calls the
/// host back (`Deliver`, `ClassifySalience`) to run triage.
pub(crate) const TINYEVENTQUEUES: ModuleRecord = ModuleRecord {
    id: "tinyeventqueues",
    description: "Durable inbound event queues: dedupe, filtering, pacing and retry before triage",
    bus_name: tinyeventqueues_bus::INTERFACE,
    object_path: tinyeventqueues_bus::OBJECT_PATH,
    version: "0.2.3",
    release_url: "https://github.com/tinyhumansai/tinyeventqueues/releases/tag/v0.2.3",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinyeventqueues-0.2.3-archlinux-rolling-x86_64.tar.gz",
            sha256: "c716df10e0db7f4561a2481c84adda66fc87dc91872d8a3719062a0f2ff3f9ab",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyeventqueues-0.2.3-fedora-43-arm64.tar.gz",
            sha256: "5cbdbca68daaa3d6202115f4c1fdd2f0e486f7bae55e953f3214c9187e535b5c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyeventqueues-0.2.3-fedora-43-x86_64.tar.gz",
            sha256: "1973ce80a2d98537c9354ded5bd45c89b462df2af4ebd952b04d80d6b280948a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyeventqueues-0.2.3-fedora-44-arm64.tar.gz",
            sha256: "20ae7c7e0beea8383199348a456dfb7c5e6b2fb6be8a2f7b6168cb78bfb81cbb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyeventqueues-0.2.3-fedora-44-x86_64.tar.gz",
            sha256: "24ef4bdd752cf904951ccf1a403644b9c327b98307ff3c4b176c4054cf62cb76",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyeventqueues-0.2.3-macos-15-arm64.tar.gz",
            sha256: "d3e40cad7123858202df194016770d0fd33e456ca54d66b4b834f3bde94c36cd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyeventqueues-0.2.3-macos-15-x86_64.tar.gz",
            sha256: "bfb06657af89b4e390ac472d313515f575f316af9e37b6ec82355eb4c591d387",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyeventqueues-0.2.3-macos-26-arm64.tar.gz",
            sha256: "3193d5c030e94da14b21d22ab898a138f8c7c123ad126494e9a900f03dc86b9d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyeventqueues-0.2.3-macos-26-x86_64.tar.gz",
            sha256: "904bf1831f2ccd3ee39dfbd76b217ff12960b07ee34e505345e487815e7a5cbb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyeventqueues-0.2.3-ubuntu-22.04-arm64.tar.gz",
            sha256: "aea910c100358633b9c15b19ec822f49de33d686f3c762d35ab6514274ec3e32",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyeventqueues-0.2.3-ubuntu-22.04-x86_64.tar.gz",
            sha256: "326ab638b2a58615dde08b1caf25548768227e2a13a93f49d84e10c21369ff52",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyeventqueues-0.2.3-ubuntu-24.04-arm64.tar.gz",
            sha256: "aace0e94d82aca79a473346364ed6c54282356a0a1698cff00016983dc778a53",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyeventqueues-0.2.3-ubuntu-24.04-x86_64.tar.gz",
            sha256: "b86d1fdbd56c569ab184b7a10fab9e07cf3c895b60b43585492ce89a603fd6e9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyeventqueues-0.2.3-windows-11-arm64.zip",
            sha256: "7d0b10558977d44edb54cd0526cf7fe38cee5c48c6697fbb1770680e813e80e5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyeventqueues-0.2.3-windows-2022-x86_64.zip",
            sha256: "79237c2ecada6ccf4adeadbe32a003369f041f5d4028ca1b433fbd6193e4dae9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyeventqueues-0.2.3-windows-2025-x86_64.zip",
            sha256: "93ff2b16fbfc545b986d1e2b92f4448d525bd8e90a44a8de80c2a8a4826007e0",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyeventqueues",
        manifest: "crates/tinyeventqueues/Cargo.toml",
        package: "tinyeventqueues",
        lib_name: "tinyeventqueues",
        features: &[],
    },
};
