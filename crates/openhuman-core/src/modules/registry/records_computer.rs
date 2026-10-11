//! Native computer-use module (desktop and browser control). Published digests are added only from its release manifest.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};
use tinycomputer_bus::names;

pub(crate) const TINYCOMPUTER: ModuleRecord = ModuleRecord {
    id: "tinycomputer",
    description: "Permission-aware desktop and browser observation and control",
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.11.0",
    release_url: "https://github.com/tinyhumansai/tinycomputer/releases/tag/v0.11.0",
    // All sixteen published archives were checked against checksum.toml
    // and GitHub asset digests before pinning. Linux permission probes are
    // part of the shipped voice surface alongside macOS and Windows.
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinycomputer-0.11.0-archlinux-rolling-x86_64.tar.gz",
            sha256: "467c64d90791957a83d2ae5e58a3bffaf02e7bd6c1b01f73a540b1f2eca2fdf4",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinycomputer-0.11.0-fedora-43-arm64.tar.gz",
            sha256: "145b21e785a43b466e0fcc5a3675c635757b5c6383cb7f082fc9ecfac418b855",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinycomputer-0.11.0-fedora-43-x86_64.tar.gz",
            sha256: "69f855af1e6e23cfe97c22aa23fc3354cddd548b2e7f885b6cb620bb33decc78",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinycomputer-0.11.0-fedora-44-arm64.tar.gz",
            sha256: "f3f89b1b2579ae383c1b13b7ad0c7543e9db51986ed77e598640a10340600d2a",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinycomputer-0.11.0-fedora-44-x86_64.tar.gz",
            sha256: "22f464deb0a04c05babc49a23583c30040b065e5620247917a9f5d447b84e89c",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinycomputer-0.11.0-macos-15-arm64.tar.gz",
            sha256: "4654e5143f81ac0865df37cc25ab3a003c7fe91edb0942c0d44d6382519d2b05",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinycomputer-0.11.0-macos-15-x86_64.tar.gz",
            sha256: "38eddc6ed86dbed7c91a36bda3ab8fe7cd46db1dd467a84003bfadf5a54ec2b7",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinycomputer-0.11.0-macos-26-arm64.tar.gz",
            sha256: "3a8e20a809262be2154c1b7339fed2a627c13c32b2a7b9de52e4d54ced94a994",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinycomputer-0.11.0-macos-26-x86_64.tar.gz",
            sha256: "a61a79cb63798b844009cc8c69575690d6b2d5a25b637e0b05d559b14ad66c10",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinycomputer-0.11.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "be9b21e5161314e3ab9fe3841ad4552a47b018b1cd999d0fcdeb008fcb3ba239",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinycomputer-0.11.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "fb7654ac75546c32b981be272c4353e2632061b2e05de2a1a535f50e88727188",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinycomputer-0.11.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "4401c22eb3cf20ce216d7627a3bacd668e27deb21019de0f47955812629014cf",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinycomputer-0.11.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "430426bdbc6f8ad8cc5ed17d6f3bd05f73b4d4fe747a8fc9b77f0b3f4fd3506b",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinycomputer-0.11.0-windows-11-arm64.zip",
            sha256: "df97f97f18ed670e57b8b79775fc9bc4cbe44b59f9320c957f93f89f7eba41c0",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinycomputer-0.11.0-windows-2022-x86_64.zip",
            sha256: "494efca1d2aee2b8371bebc70cb6b791426ada4bc35da0524c7317fd084e9157",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinycomputer-0.11.0-windows-2025-x86_64.zip",
            sha256: "b65b7ea2f8f678e5a55920b8d72ae62642e3e03ba44e0db78bc8807e8c62520d",
        },
    ],
    load: LoadPolicy::Lazy,
};
