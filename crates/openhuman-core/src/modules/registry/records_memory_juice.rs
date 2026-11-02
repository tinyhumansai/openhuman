//! Registry record for the `tinyjuice` module.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYJUICE: ModuleRecord = ModuleRecord {
    id: "tinyjuice",
    description: "Content-aware tool-output compression and recoverable caching",
    bus_name: tinyjuice_bus::names::BUS_NAME,
    object_path: tinyjuice_bus::names::OBJECT_PATH,
    version: "0.11.5",
    release_url: "https://github.com/tinyhumansai/tinyjuice/releases/tag/v0.11.5",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyjuice-module-0.11.5-archlinux-arm64.tar.gz",
            sha256: "f865d734eac10f9d487d6ec0b4e9898a4bb8e570207e7d9272163a0c2a44a825",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyjuice-module-0.11.5-archlinux-x86_64.tar.gz",
            sha256: "0936436d1659af2a6a677d2f660eea24596066f2dd9eaafd6fea843144fdcada",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyjuice-module-0.11.5-fedora-43-arm64.tar.gz",
            sha256: "c40e47b81f8f7f2fdaf26bbd7310286c052edb80dd9504a9e4dcd3d9c99e34e6",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyjuice-module-0.11.5-fedora-43-x86_64.tar.gz",
            sha256: "d27c7efaa70ebfa7c4fbe6bdf5f4121b2afcec5702735c6b1425acf8671e05c4",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyjuice-module-0.11.5-fedora-44-arm64.tar.gz",
            sha256: "8e34fc840fd8e673cb5d1eac191f4d58b63d43394bafac8b2954f149655da716",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyjuice-module-0.11.5-fedora-44-x86_64.tar.gz",
            sha256: "dac80d49626952c4c8a45a8596cc861bf5c527a85561ce8747aba097b54ea1f9",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyjuice-module-0.11.5-macos-15-arm64.tar.gz",
            sha256: "f661177d68d29c2409ca9421e10ed82d3b78679784ac94875fe0658f38a3a47d",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyjuice-module-0.11.5-macos-15-x86_64.tar.gz",
            sha256: "c912a9dc14932a7b5ef7c6e799a0a4b03edb2df093596dcf4ac766ec18118c15",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyjuice-module-0.11.5-macos-26-arm64.tar.gz",
            sha256: "58f3317b35a71dbb811e7d5b9a6e9cbf36ed682e1f97853927eb3c3f852c16cf",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyjuice-module-0.11.5-macos-26-x86_64.tar.gz",
            sha256: "11fe4d323a0b52186f4a0204a036354d004dc2f8597d631d8f0f690e06eb2f44",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyjuice-module-0.11.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "7b8f4c8cf6e17d1557c1593ec636ff7baa48273eaa5a1d2f46f8dce83587329f",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyjuice-module-0.11.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "9796d39f8a02c846a51e893ed5bed0b82e1ebf66d23eb339b9d737fca048266d",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyjuice-module-0.11.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "d5e12d3384b6a9bab2ae934dfa1a70e2d243d11475340c8788ebaa89890763ed",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyjuice-module-0.11.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "7f04334724e1246eb22f63303d988b7d27a8bf85c2995f3b1fcb01b36f64f490",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyjuice-module-0.11.5-windows-11-arm64.zip",
            sha256: "732ac279d412067ad59ebd0710b5304ec846d25c01404bad79ff984d06a1680c",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyjuice-module-0.11.5-windows-2022-x86_64.zip",
            sha256: "aaa1cd900f6633a4bb645398643e344f7d32b790b11c52281928bba09082a1f2",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyjuice-module-0.11.5-windows-2025-x86_64.zip",
            sha256: "7c1f2d063e0115fbfd77e6d138132f45c427b684edf3665d28333e8cddaa3782",
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyjuice",
        manifest: "crates/tinyjuice-module/Cargo.toml",
        package: "tinyjuice-module",
        lib_name: "tinyjuice_module",
        features: &[],
    },
};
