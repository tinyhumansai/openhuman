//! Published TinySecurity admission pins copied verbatim from v0.21.1/checksum.toml.
//!
//! `LoadPolicy::Eager` is the "load at boot" policy (the issue calls it Boot): a missing
//! security module changes behaviour, so it must be serving before the first request.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSECURITY: ModuleRecord = ModuleRecord {
    id: "tinysecurity",
    description: "Native security policy and durable approval tool grants",
    bus_name: tinysecurity_bus::names::INTERFACE,
    object_path: tinysecurity_bus::names::OBJECT_PATH,
    version: "0.21.1",
    release_url: "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.21.1",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.21.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "9f94fa86fe883e6aa7d96b0835ec8feef0c589c9f029a22b1c1c859103a088f2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.21.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "caad79e9772384a0879c85700566c1efd58075bf7f8ac57f9067ca510459d440",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.21.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "bcd8512c79e4b85d84b03fb791ac273ade830775ac86ff7ed16452fbf829d8e0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.21.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "c1c2b68d8e23bc80056dacb9bb343f705d7ead11b9aa139d87a82069318f743a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.21.1-macos-26-arm64.tar.gz",
            sha256: "d66db53772d3843c9d1790b1b218bc5388c2b9b5f01a7ff6f4d5391498010213",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.21.1-macos-26-x86_64.tar.gz",
            sha256: "a7e95e03289917a69253f6d28c8fdfcf076ffa48d748e522c4c82d75f81e607b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.21.1-macos-15-arm64.tar.gz",
            sha256: "1de5e5e83a30a400fcbeff3cc5cd8090e9bc243e4bc35a2f61b0a6e3193728d8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.21.1-macos-15-x86_64.tar.gz",
            sha256: "6f4329bff1efcdcf82ed3d590e4536c89e814b43f3eccea4d4a5c902157babb1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysecurity-module-0.21.1-windows-2025-x86_64.zip",
            sha256: "612d04ef0616589f61c852789768f06e0c4aa5fe02449e266a2bbc3df242d590",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.21.1-windows-2022-x86_64.zip",
            sha256: "504c9937191a1c84f8e03964c3536e0e83417072769b1dba2ee31c94a2870e91",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.21.1-windows-11-arm64.zip",
            sha256: "cd77275f4849678c71e8dcaa6bad8b2ca4c473b093a641660a7b8195265e00f5",
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
