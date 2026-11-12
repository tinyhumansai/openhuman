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
    version: "0.21.0",
    release_url: "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.21.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.21.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "793637899fd71bc232b215a486c51d9e888ce0aee12bb924a362b9d0729d907e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.21.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "5d05e1678aec9c27705090902c6a8027b55492dad6cbf036f67643c18368afc4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.21.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "cad96e016358b44b0179c8e23f7492c98c0521b148a123cbcb84158e36cf2b32",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.21.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "23ffa7b0cf576f1488a3b7419c0ee873425f4141e9f7899e08925909e4fcf068",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.21.0-macos-26-arm64.tar.gz",
            sha256: "d02e23e87b227e7e9f8933c0525dc99d7b81e896cd34877b145bce10ce5708cd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.21.0-macos-26-x86_64.tar.gz",
            sha256: "c12af96a7c0348c3c3ca942abb317072cffe6c982d174e25c5b155c6b9895861",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.21.0-macos-15-arm64.tar.gz",
            sha256: "0f8e2e15f6a13951ecf8396bd472a83f65701d5e139aef9be89a74dd4daad4f9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.21.0-macos-15-x86_64.tar.gz",
            sha256: "478fab9d675009570de65d8be2863f89bb6b6f3a9a552af8ab229ae5638ef455",
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
