//! Native computer-use module (desktop and browser control). Published digests are added only from its release manifest.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};
use tinycomputer_bus::names;

pub(crate) const TINYCOMPUTER: ModuleRecord = ModuleRecord {
    id: "tinycomputer",
    description: "Permission-aware desktop and browser observation and control",
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.18.0",
    release_url: "https://github.com/tinyhumansai/tinycomputer/releases/tag/v0.18.0",
    // All seventeen published archives were checked against checksum.toml
    // and GitHub asset digests before pinning. Linux permission probes are
    // part of the shipped voice surface alongside macOS and Windows.
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinycomputer-0.18.0-archlinux-arm64.tar.gz",
            sha256: "32ed1e7d7eb3ed93496752d3342bb28b02d302e2425a3bfa3d3c65b8b306060f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinycomputer-0.18.0-archlinux-x86_64.tar.gz",
            sha256: "6ba9df62f52ab4f23552ed2e12ff0bbabc16d7d076a20589c6be646f175bf735",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinycomputer-0.18.0-fedora-43-arm64.tar.gz",
            sha256: "9252d5f7dc27297f71a0187b57fe82fc17595b38937bda0a242700e54db52757",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinycomputer-0.18.0-fedora-43-x86_64.tar.gz",
            sha256: "c634e7380ae1dddd04cdd45b0ef8e84dea7e15c0be4dcddf5ad2157fcfb86252",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinycomputer-0.18.0-fedora-44-arm64.tar.gz",
            sha256: "acf92a651670614ccefaadd104b7d0e1701eb82fb838abd4ab5a05fbc6684a51",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinycomputer-0.18.0-fedora-44-x86_64.tar.gz",
            sha256: "a1ce13fd3c6b9e99730c92e3bf2c4edf41bcc4dd7f4259ced9417f0f1107fc39",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinycomputer-0.18.0-macos-15-arm64.tar.gz",
            sha256: "99a078dd03d3d444e51fcb7c12d3554929b0f6a8c9acb6b3cfb9d7f49040cf35",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinycomputer-0.18.0-macos-15-x86_64.tar.gz",
            sha256: "eda9edca053a42fb82e06ba5a2a8b3dd873bc3e8d3d1be86a032787350b8a6dd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinycomputer-0.18.0-macos-26-arm64.tar.gz",
            sha256: "06b3fbdc70520ecf8bc131f67bc5c7104af06a54bcb989b379ce3cd39491073f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinycomputer-0.18.0-macos-26-x86_64.tar.gz",
            sha256: "69d20c98997cbed3a0198076b1a3ccea127959585b2245abcd130c9de19567b1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinycomputer-0.18.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "0e752ce55fe488e5372242a064857ed9e2733f6be11b223a179e40aa8003bc1e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinycomputer-0.18.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "976fdcb8e3406886bad35a6743ab56783989db67d9ab22669b04033397cadeae",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinycomputer-0.18.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "ec20f09440218efe9f29080f59fcf9dba94a25fe3532d53580204b57059937be",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinycomputer-0.18.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "6cb3141cb850f308fc63ecb441e18cd727b26591f59c379fe317c5a2d13e95ec",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinycomputer-0.18.0-windows-11-arm64.zip",
            sha256: "f3046dfea2ed3e048ec46acf2511c40262c210c564335e496aa7d418d27deb9b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinycomputer-0.18.0-windows-2022-x86_64.zip",
            sha256: "5915fd327a657dfd47f1402bcb21006e3e3a72d41cab176aac7614d9cfb20d12",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinycomputer-0.18.0-windows-2025-x86_64.zip",
            sha256: "a170cd5f27eb0db06a0852b045a29f9e205c87e6c69df929174972f1f2e04d9c",
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
