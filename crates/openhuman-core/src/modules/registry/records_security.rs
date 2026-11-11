//! Published TinySecurity admission pins copied verbatim from v0.20.0/checksum.toml.
//!
//! `LoadPolicy::Eager` is the "load at boot" policy (the issue calls it Boot): a missing
//! security module changes behaviour, so it must be serving before the first request.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSECURITY: ModuleRecord = ModuleRecord {
    id: "tinysecurity",
    description: "Native security policy and durable approval tool grants",
    bus_name: tinysecurity_bus::names::INTERFACE,
    object_path: tinysecurity_bus::names::OBJECT_PATH,
    version: "0.20.0",
    release_url: "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.20.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.20.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "e694059c8f2e9f4da4265ae8523fa8d6643236a4eff745e5e4290ee6ffe06fa9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.20.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "745b0583c73703644da49b9c741c8c8a487ea24116d51b797133f3cea0a5d680",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.20.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "d531a36c266cb467c1fc258543ed4713ddd2234930fc3beeba386b48e4bbe0ac",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.20.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "3b89af12be97ccac2a2593ff9886cd8f9cb7fcba5405fe04c9d0986e4d6c110c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.20.0-macos-26-arm64.tar.gz",
            sha256: "9f51e6bfda60aa7d44520130cf4a3e1e88e5fee7b423ca663a979926e79e3d30",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.20.0-macos-26-x86_64.tar.gz",
            sha256: "4668ee65802f5aa55b8c9310f8dd11a3a290ff4e121895ba73f1e7951ad9fb21",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.20.0-macos-15-arm64.tar.gz",
            sha256: "d0c80a5fc21527d812504d64dc25aeadc55327f17ff938ed18d9b5da36800239",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.20.0-macos-15-x86_64.tar.gz",
            sha256: "6d351306f7d970715199c6e9b9e4b566e4ff29dfd94701d6ce19716bf03114c1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysecurity-module-0.20.0-windows-2025-x86_64.zip",
            sha256: "77359a4c2b3c1207e98e8c36e86bd014d7933cffbd4889bc4e7fc36c53adf3e7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.20.0-windows-2022-x86_64.zip",
            sha256: "282f9e8cfff87da92b0f0c0aba2650edaf10def411a3a7423d46d890adfaae26",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.20.0-windows-11-arm64.zip",
            sha256: "5c45a750b61315609a88fb487bab0a2ee87292c5f2eff6d4a19d46c93a53ca15",
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
