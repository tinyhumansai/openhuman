//! Native computer-use module (desktop and browser control). Published digests are added only from its release manifest.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};
use tinycomputer_bus::names;

pub(crate) const TINYCOMPUTER: ModuleRecord = ModuleRecord {
    id: "tinycomputer",
    description: "Permission-aware desktop and browser observation and control",
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.16.0",
    release_url: "https://github.com/tinyhumansai/tinycomputer/releases/tag/v0.16.0",
    // All seventeen published archives were checked against checksum.toml
    // and GitHub asset digests before pinning. Linux permission probes are
    // part of the shipped voice surface alongside macOS and Windows.
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinycomputer-0.16.0-archlinux-arm64.tar.gz",
            sha256: "6dc3229b014f15560fd7dd793fc877b653b88571db9963dc919be8866e58a4f1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinycomputer-0.16.0-archlinux-x86_64.tar.gz",
            sha256: "3f19184a00aefeb218aa1dac6ecb534be0ef9973becceb10506ac075e9911b9d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinycomputer-0.16.0-fedora-43-arm64.tar.gz",
            sha256: "7c0cc88be5169950dbf5f5677df8431c8c76c747cdc4a78d64c7b666e6ad121c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinycomputer-0.16.0-fedora-43-x86_64.tar.gz",
            sha256: "a203aef368219cf4483cd023d92b712b7ad38e1517074639c7ea38d9ccccbac3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinycomputer-0.16.0-fedora-44-arm64.tar.gz",
            sha256: "05c4dc081306af96846f9196741cfadceaee51a9c221c210fc7486649f690bc3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinycomputer-0.16.0-fedora-44-x86_64.tar.gz",
            sha256: "dd249cf9cfd0854e0493b4100a40b767e22e20b0cc316e4529625c5b3d500279",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinycomputer-0.16.0-macos-15-arm64.tar.gz",
            sha256: "97ef1ef6134d36bc25c770efcbedd239374d43e615ec57cf91f5eb44ac314265",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinycomputer-0.16.0-macos-15-x86_64.tar.gz",
            sha256: "5bafc90beff673f66b1442cb0ffb41acb33512431ed167833d0f0de64f8b6f94",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinycomputer-0.16.0-macos-26-arm64.tar.gz",
            sha256: "a028876b857e62a8a55b850811a0d20f85dd0a2bb8a6533c2c4008a7062b4a35",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinycomputer-0.16.0-macos-26-x86_64.tar.gz",
            sha256: "1604838f9793dd1e467ff299b2beda47aedeb94810fb8ac857629fdd061c0812",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinycomputer-0.16.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "5cce23246a32c23cc6a6d0adf530126e65eaaeaaa864e5cc7b20b63c92ac29e0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinycomputer-0.16.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "dbe66666a9ace5e4c1751ba191a12aa2f53e608afbd5f7383e6277d82238407b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinycomputer-0.16.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "4a5039effb89b49cc6650c66c5f32efa898c8cd0d18e2d50b3275d4c1cec372d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinycomputer-0.16.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "3144d4dd90f7c4886e4eff899210f2fc8b6f8235e4d14c8c03454f356d00d062",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinycomputer-0.16.0-windows-11-arm64.zip",
            sha256: "857f8723b4624f5f29707797310de7e95439ac09ef0c7e46b7a3aab019805121",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinycomputer-0.16.0-windows-2022-x86_64.zip",
            sha256: "ec8ce6ef45c8b399b5b3ff6eb93ff5d74f714ad93e7cfc228e52e59fcbf00f1b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinycomputer-0.16.0-windows-2025-x86_64.zip",
            sha256: "68133c2d864e8f8cbfb5773bd7afd7663e3d2ea0fd731f82570bbb961b37e694",
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
