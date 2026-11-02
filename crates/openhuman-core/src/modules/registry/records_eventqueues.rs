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
    version: "0.2.1",
    release_url: "https://github.com/tinyhumansai/tinyeventqueues/releases/tag/v0.2.1",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinyeventqueues-0.2.1-archlinux-rolling-x86_64.tar.gz",
            sha256: "ee7fcd7316fbae4bc932ac231dfc35fc0a6c309fa12e88154fc2bb14e2475925",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyeventqueues-0.2.1-fedora-43-arm64.tar.gz",
            sha256: "d41f2aeb3767f51b31c62fa92c7dcbb45f3686cf0951cd85eee5c2b6c1d2633c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyeventqueues-0.2.1-fedora-43-x86_64.tar.gz",
            sha256: "042e505157e6b18d66307f4e00c2929bb5279875d209db6698b5f01438c5b6e1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyeventqueues-0.2.1-fedora-44-arm64.tar.gz",
            sha256: "1627eb8f7e4dd56abffa3f3c4b882edf9bd216fe474728f5788a9997703a38d9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyeventqueues-0.2.1-fedora-44-x86_64.tar.gz",
            sha256: "41a0072c767f53a39ebe6756057253991b54cd47f43ab4f9a6e87bf5b26b40d1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyeventqueues-0.2.1-macos-15-arm64.tar.gz",
            sha256: "36f52348e93ba516de0072b89858c4f960433081c20b93a4956dbd226190dd45",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyeventqueues-0.2.1-macos-15-x86_64.tar.gz",
            sha256: "b8ff10113e3874dc06d9ae06ca9bcff6b7689f49c084c780b29f93cf5a5e8e5b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyeventqueues-0.2.1-macos-26-arm64.tar.gz",
            sha256: "5e16b2121643783c00cab1c129093c5007926069d1be4abab98ed29effe9da96",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyeventqueues-0.2.1-macos-26-x86_64.tar.gz",
            sha256: "5d6449cb5027887570c9033cf38b91fdf3a2cc505c43fa863011566e1a885000",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyeventqueues-0.2.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "92631e5ff5ec5aa5f515472a9a11478a0e2aa88006fc8d82694d7e5bbaf5140c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyeventqueues-0.2.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "6c7698049080c33c00233a9fbe26ffad0d50638f91ea3a2908c11fc530994c9e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyeventqueues-0.2.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "65180e0ebb42ca48c56e90c056a76b02b7c589a52fbca798b97d2a3b56975fff",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyeventqueues-0.2.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "13ccb0fea19b30fd26981d30eee1b8f937a948c3f2f6b670a7b0ad6d06d926a2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyeventqueues-0.2.1-windows-11-arm64.zip",
            sha256: "dbdbecd9bd273c9d7e0f4bbf5ac537158df1f7b065595063b8068eb117da5ea0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyeventqueues-0.2.1-windows-2022-x86_64.zip",
            sha256: "463b302229221fb8fb1cb0b2857c34460a52296a4377ac5cc95657d40ad70de8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyeventqueues-0.2.1-windows-2025-x86_64.zip",
            sha256: "ebbe289ece1b6ebe8ec4f4f76b7e655605822d6babd89880111584780fb7f9e7",
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
