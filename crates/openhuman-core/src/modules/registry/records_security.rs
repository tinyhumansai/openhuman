//! Published TinySecurity admission pins copied verbatim from v0.18.2/checksum.toml.
//!
//! `LoadPolicy::Eager` is the "load at boot" policy (the issue calls it Boot): a missing
//! security module changes behaviour, so it must be serving before the first request.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSECURITY: ModuleRecord = ModuleRecord {
    id: "tinysecurity",
    description: "Native security policy and durable approval tool grants",
    bus_name: tinysecurity_bus::names::INTERFACE,
    object_path: tinysecurity_bus::names::OBJECT_PATH,
    version: "0.18.2",
    release_url: "https://github.com/senamakel/tinysecurity/releases/tag/v0.18.2",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.18.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "4c8d90347c37293d5ea773245e0ce9dda4a18234117736757564beeffea5f5fa",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.18.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "a91db19a360840fd5f8a7c341ef4856fcdc25db98cc597b7acdd80ac075ab046",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.18.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "000e9a3a07a8388fee3a6b823615998959d5255e584526b978b653959fad8a30",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.18.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "34c3ef7b6fde18d525ae27fe09716ad2775a91b3152663a1f72b88a0da15e325",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.18.2-macos-26-arm64.tar.gz",
            sha256: "bc765db3337e98c4f0c96ba6fd47576181c9783e833f9bdeccc745dcab276dc5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.18.2-macos-26-x86_64.tar.gz",
            sha256: "10575824c14ea3503170420523ecdb6b3fa33cecb85f9ff3d36817666bb7833e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.18.2-macos-15-arm64.tar.gz",
            sha256: "7cd14bf7bbb11bceb4e09560a8f66c941e28541e94ac42320156ba9a7473952b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.18.2-macos-15-x86_64.tar.gz",
            sha256: "7829b6e6b41a34d789973e8f94f828235c26ae201050de3645dfd6b53568941c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysecurity-module-0.18.2-windows-2025-x86_64.zip",
            sha256: "2b41ef005c4d0756233451ca8667fdce492b73a85234bd7f4e2af06949130b70",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.18.2-windows-2022-x86_64.zip",
            sha256: "105ccd4d6fc2787cefaedba2dcc2f5e40129c0b4759e41095099322da5d6058b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.18.2-windows-11-arm64.zip",
            sha256: "dc6e7b53e7bf65c9adea251d0f1f5852a89f0588c782dbdb6ac0bab08a33c8ef",
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
