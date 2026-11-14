//! Published TinySecurity admission pins copied verbatim from v0.21.0/checksum.toml.
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
            archive: "tinysecurity-module-0.21.0-windows-2025-x86_64.zip",
            sha256: "260884a7ca22f99969a162b464a47caa78380aeb6e9b903d2fa274f84fe46f7b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.21.0-windows-2022-x86_64.zip",
            sha256: "b500a2e6161b85a06ad54644dfdb685b9369aadd0d868a0cdc11ab18abf9693d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.21.0-windows-11-arm64.zip",
            sha256: "d888f4e60df0fd7ecc2e9226695ab3541d83206203e391331b8a78e81f5dee0b",
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
