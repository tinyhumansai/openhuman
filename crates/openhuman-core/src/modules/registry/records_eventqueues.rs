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
    version: "0.2.4",
    release_url: "https://github.com/tinyhumansai/tinyeventqueues/releases/tag/v0.2.4",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinyeventqueues-0.2.4-archlinux-rolling-x86_64.tar.gz",
            sha256: "69a11fdf607e8d5529eed23f5d782bda75aaa6f9992db473b086d9082f591964",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyeventqueues-0.2.4-fedora-43-arm64.tar.gz",
            sha256: "636f87a164c4dfc28bc3d06968812b6d7f3c2c76b17f0ac8119d65ddd6b12ab8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyeventqueues-0.2.4-fedora-43-x86_64.tar.gz",
            sha256: "f156684dc51868b78920f582c8f4245d2b997e8a2ffe405cdba484339c5e5e94",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyeventqueues-0.2.4-fedora-44-arm64.tar.gz",
            sha256: "509e8f590e769f61f43545c3555815284ac0e5b571e48fefe13bc55bdc9daad1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyeventqueues-0.2.4-fedora-44-x86_64.tar.gz",
            sha256: "455983dbd11165b119cfcf0999bd1e0411448b0228ab08dd7d1af2c9bc76024c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyeventqueues-0.2.4-macos-15-arm64.tar.gz",
            sha256: "ade233944a750d0c74bd1f05938d35d600d902a2c5b0757629962aa75588115b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyeventqueues-0.2.4-macos-15-x86_64.tar.gz",
            sha256: "7bb4fb66dd1f458bc369920adf88475915d1117ca5257d5dccf31a5acd41d28f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyeventqueues-0.2.4-macos-26-arm64.tar.gz",
            sha256: "a8d6fc02f5449bb213d121881a5070efd365fdf8b6d579702d1fc33ee69dda4c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyeventqueues-0.2.4-macos-26-x86_64.tar.gz",
            sha256: "65b32c021c8cb42c6bedd16c3df580ae5a2b6ea6e9a2392fd0dba48395396729",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyeventqueues-0.2.4-ubuntu-22.04-arm64.tar.gz",
            sha256: "d6159231731150c5e284f5f9fe6905aeb88d6881be26e1f8a64e1517f577fa00",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyeventqueues-0.2.4-ubuntu-22.04-x86_64.tar.gz",
            sha256: "df4d30b8833cdf50db54608ceee5062033121ad5edb649da1218488dc6ec5cb6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyeventqueues-0.2.4-ubuntu-24.04-arm64.tar.gz",
            sha256: "e3b967bf4cc1d73b3e909535f5be077a9e862579d25e05a412ad67fdea8fa82b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyeventqueues-0.2.4-ubuntu-24.04-x86_64.tar.gz",
            sha256: "8a02f90dae9295a372042b6e4f15023948d8aedec99391e11e165fc643bdd0b0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyeventqueues-0.2.4-windows-11-arm64.zip",
            sha256: "e0db90d65ce1f463d5dd6fab298344807a0129a58471c7e035eb8b61070e8f5f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyeventqueues-0.2.4-windows-2022-x86_64.zip",
            sha256: "f70d618f903cdc98f5f8853cc5135583732a67539a77b373eb90aa14b0e9b12f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyeventqueues-0.2.4-windows-2025-x86_64.zip",
            sha256: "591b2d559e6807b4c295dc4275827bdede573fb9ce74bb7c0ab65921a192c398",
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
