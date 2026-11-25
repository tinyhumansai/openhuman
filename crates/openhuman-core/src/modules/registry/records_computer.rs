//! Native computer-use module (desktop and browser control). Published digests are added only from its release manifest.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};
use tinycomputer_bus::names;

pub(crate) const TINYCOMPUTER: ModuleRecord = ModuleRecord {
    id: "tinycomputer",
    description: "Permission-aware desktop and browser observation and control",
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.19.0",
    release_url: "https://github.com/tinyhumansai/tinycomputer/releases/tag/v0.19.0",
    // All seventeen published archives were checked against checksum.toml
    // and GitHub asset digests before pinning. Linux permission probes are
    // part of the shipped voice surface alongside macOS and Windows.
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinycomputer-0.19.0-archlinux-arm64.tar.gz",
            sha256: "ea4a12c51ce892079d1e6b1fd9b953abb0c60e086647b30d897995fda0461d19",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinycomputer-0.19.0-archlinux-x86_64.tar.gz",
            sha256: "3a91ce5c644e24e9e83acbbc4d74d4a76a78206b2eab2ebdadf9f6995249dd77",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinycomputer-0.19.0-fedora-43-arm64.tar.gz",
            sha256: "a9257c5347c7fe4a6a0579298ab7a2a03f655faf542b7689dfbd137ab2a486a2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinycomputer-0.19.0-fedora-43-x86_64.tar.gz",
            sha256: "025e999d6cd489e0749676757d7780980ff3c5fea7fe94d85bae294ea9fc5b3b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinycomputer-0.19.0-fedora-44-arm64.tar.gz",
            sha256: "a2620a91a090b39601ce9184392d767c5ab4a6a285b315c99e88713489d52258",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinycomputer-0.19.0-fedora-44-x86_64.tar.gz",
            sha256: "be9af3bc9f8da9fa16f62a6e0116e05282071990365524b81c2fac83198bf682",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinycomputer-0.19.0-macos-15-arm64.tar.gz",
            sha256: "9730bdd2feae352d04817c4fec22ee29928a254985000c885706178c5acb94a4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinycomputer-0.19.0-macos-15-x86_64.tar.gz",
            sha256: "6645458507757c41e67269e6653da49f9908e831159447743c8f9e120ae6a550",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinycomputer-0.19.0-macos-26-arm64.tar.gz",
            sha256: "56ca22e19b2e937ca2ab9416a73c82faa5ecf57701c9d1b781d922382e85a593",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinycomputer-0.19.0-macos-26-x86_64.tar.gz",
            sha256: "89ee27db6560645f205d2d32085d739b12f58d375a9a84110e68c4b1a2ba8b53",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinycomputer-0.19.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "4153e966cf6d5a0482367eaff213106f005b31045a250437edf1c7daa8e321cf",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinycomputer-0.19.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "21a5886f79670706ff7aa31d3a8136723046283d79837ab6ba7efd0330fd6295",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinycomputer-0.19.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "4568ee593e006300320ec0e68d524325b0bd33f0f774df8da83eb0de005e2cc2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinycomputer-0.19.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "087e0f80858d7641ee2c925d86cde4d6e47f0260f7182fdddbb98a34af560f84",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinycomputer-0.19.0-windows-11-arm64.zip",
            sha256: "eda643fe7dac2452aa7a7f9827b389f0479e7867af65306ed7ebb6ec900167a6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinycomputer-0.19.0-windows-2022-x86_64.zip",
            sha256: "82f94ba13a0dfe11e6bc663d19f0791f1dcaba30952adcc3c24fb5804c11070e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinycomputer-0.19.0-windows-2025-x86_64.zip",
            sha256: "7269870c6abf98aa57f2076156ba8888cbc7be6eaa96274cd4a49f3d135e4e9f",
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
