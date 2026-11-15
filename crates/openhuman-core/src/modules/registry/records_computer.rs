//! Native computer-use module (desktop and browser control). Published digests are added only from its release manifest.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};
use tinycomputer_bus::names;

pub(crate) const TINYCOMPUTER: ModuleRecord = ModuleRecord {
    id: "tinycomputer",
    description: "Permission-aware desktop and browser observation and control",
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.17.0",
    release_url: "https://github.com/tinyhumansai/tinycomputer/releases/tag/v0.17.0",
    // All seventeen published archives were checked against checksum.toml
    // and GitHub asset digests before pinning. Linux permission probes are
    // part of the shipped voice surface alongside macOS and Windows.
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinycomputer-0.17.0-archlinux-arm64.tar.gz",
            sha256: "8c6faf95e445844f0623170bd91e6694432ad156ad0d4f227298f54fb3cda953",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinycomputer-0.17.0-archlinux-x86_64.tar.gz",
            sha256: "d651abff22ef46a6dc1599ed44dd3f1dc7e64b6a3071a0cd121fd07c3dbaba49",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinycomputer-0.17.0-fedora-43-arm64.tar.gz",
            sha256: "4ee6c92a197fe6e040e580be4ffda170f04db0af4ba90a42fc3699130ddd7fd3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinycomputer-0.17.0-fedora-43-x86_64.tar.gz",
            sha256: "d2206a7e9420758703b1356463192e3725404886b9e31ffed9580068464ce2cd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinycomputer-0.17.0-fedora-44-arm64.tar.gz",
            sha256: "8f979c0cd4bf36e2f8a18f5b69b719ebc525fe2ce555fe3c62d83839092f6817",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinycomputer-0.17.0-fedora-44-x86_64.tar.gz",
            sha256: "566c6c86005d12bcd0614332fb4305a5ffe5e474d0717bfb33e49addb2114843",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinycomputer-0.17.0-macos-15-arm64.tar.gz",
            sha256: "843cd1ba0ca027ba2d7f24a88e1d292d859036b86b6ff2b8d825d70a8d4a60bf",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinycomputer-0.17.0-macos-15-x86_64.tar.gz",
            sha256: "7a1ef72f514a26b14e66f6659cd137e49dd7e0e0cf08ae95b5b34a33c01b97cc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinycomputer-0.17.0-macos-26-arm64.tar.gz",
            sha256: "fd06962d4966081da42a1393d6166c2ac1823ad76b55b251d5a5670b40b9477f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinycomputer-0.17.0-macos-26-x86_64.tar.gz",
            sha256: "873d1bc009b1b585b9f2a07bd07cc4d8e75c9b9c7104e328f09ab2547035be37",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinycomputer-0.17.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "9d4ed58f67a962473faf20698dd8a83a67bebf2bdbf935d5c7bf2c29b577f8db",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinycomputer-0.17.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "d2fb8b6f8846104c8d6d56caf65a6cf9e3a4837f677b2be5f7b1f4eaba1c792a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinycomputer-0.17.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "89cb31bbd797f7373504b91708f13d79dc594c3ddf4061967915a9471a5f7287",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinycomputer-0.17.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "ae63415bedbc1321d99a4f9938b0c338bfa753b0800d1b0aa495d2025e3fe1cd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinycomputer-0.17.0-windows-11-arm64.zip",
            sha256: "e2f1a0d8e504ca1eb3499e8c6469e5a3a7ebaabaf6d9f3b4f693acd59d6dc0b3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinycomputer-0.17.0-windows-2022-x86_64.zip",
            sha256: "cd1363608db6085a317a70ff3c41e9c62ebebf3cf319e6be26a4c54708e54c74",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinycomputer-0.17.0-windows-2025-x86_64.zip",
            sha256: "1a73781bf83c78cb5dc6a785d971508a8441a0adeabf5b403581d7d6e7adbf5b",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinycomputer",
        manifest: "crates/tinycomputer/Cargo.toml",
        package: "tinycomputer",
        lib_name: "tinycomputer",
        features: &[],
    },
};
