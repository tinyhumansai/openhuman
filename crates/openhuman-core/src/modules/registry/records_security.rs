//! Published TinySecurity admission pins copied verbatim from v0.18.4/checksum.toml.
//!
//! `LoadPolicy::Eager` is the "load at boot" policy (the issue calls it Boot): a missing
//! security module changes behaviour, so it must be serving before the first request.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSECURITY: ModuleRecord = ModuleRecord {
    id: "tinysecurity",
    description: "Native security policy and durable approval tool grants",
    bus_name: tinysecurity_bus::names::INTERFACE,
    object_path: tinysecurity_bus::names::OBJECT_PATH,
    version: "0.18.4",
    release_url: "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.18.4",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.18.4-ubuntu-24.04-x86_64.tar.gz",
            sha256: "2e51299127281ae7c524c606590bb41ae2f635536c63791abe327d7f6abed0ba",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.18.4-ubuntu-24.04-arm64.tar.gz",
            sha256: "61cb28f3ebc5bfb77503b20abf128a4dbf55f2c5ba9f826ba240edf8bd51ff2c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.18.4-ubuntu-22.04-x86_64.tar.gz",
            sha256: "d56513cfc8d3f0ad2552abf31918365e3bbbdb700010a03e3273c79d6ac53b06",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.18.4-ubuntu-22.04-arm64.tar.gz",
            sha256: "f406c66e69817160527b55cd037e5421ddaadbb586702014e9c78d04622de991",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.18.4-macos-26-arm64.tar.gz",
            sha256: "107f42a9cedaae3a95edf09c82818ad85195e7f0668cf1255b51ee4fbc8ac26e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.18.4-macos-26-x86_64.tar.gz",
            sha256: "b408ab2832246c1c66630c21ad2a274d6d6592a0f3eaf59039aa517e9955d8a4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.18.4-macos-15-arm64.tar.gz",
            sha256: "17d143697610cb7ab391d50d7d08d55165c1074ea1b454de08c9486cda19cc28",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.18.4-macos-15-x86_64.tar.gz",
            sha256: "b11d3ffe0a7a99e0a0fd71fd6fdb87d209e9b8e09c1cb4d3af9656376f368e8b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysecurity-module-0.18.4-windows-2025-x86_64.zip",
            sha256: "fe1f64a199a820d5b58347eea41e29109096b62d81d00e0ed5df083a6fd5005a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.18.4-windows-2022-x86_64.zip",
            sha256: "8aa5719147a32209f14f9ad9fd4300e9c114a56dfe31d8f59d019b7540b0f111",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.18.4-windows-11-arm64.zip",
            sha256: "825d2fde75cea1ffb11ff88cc037d1ec9428d784b2d5041f5eec828179f92020",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Eager,
    source: SourceSpec {
        submodule: "tinysecurity",
        manifest: "crates/tinysecurity-module/Cargo.toml",
        package: "tinysecurity-module",
        lib_name: "tinysecurity_module",
        features: &[],
    },
};
