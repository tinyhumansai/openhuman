//! Registry record for the `tinyjuice` module.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

pub(crate) const TINYJUICE: ModuleRecord = ModuleRecord {
    id: "tinyjuice",
    description: "Content-aware tool-output compression and recoverable caching",
    bus_name: "ai.tinyhumans.tinyjuice.Compression",
    object_path: "/ai/tinyhumans/tinyjuice/Compression",
    version: "0.7.1",
    release_url: "https://github.com/tinyhumansai/tinyjuice/releases/tag/v0.7.1",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyjuice-module-0.7.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "bd9ee5a1b894201f17bc19c06afb4e91c30ac3fefc2162171e78793c84a45ee7",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyjuice-module-0.7.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "b726cda4f69095bf538710f22f6978eb8ce0492e722bd1c6f90bf4fd13bc6b6c",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyjuice-module-0.7.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "00ab2e7bb215d5bd3b23c0284470001e7f1e6a82c32e1358ad3a97c48418482e",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyjuice-module-0.7.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "a9a0a60d31948f0156e05662ff9c97dd516a4202731da11b49e2357a6266ccc0",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyjuice-module-0.7.1-macos-26-arm64.tar.gz",
            sha256: "b108e4a28e1894dbe53bc24b0ab76ffc0ac48f87fbd7c74e2bba522c40bd30eb",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyjuice-module-0.7.1-macos-26-x86_64.tar.gz",
            sha256: "d2d023ab091bba47da1ef8d312397dc22cb412b085de1ef78e5b4856bcfe3829",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyjuice-module-0.7.1-macos-15-arm64.tar.gz",
            sha256: "cc05ab99239a9bb73ac7a166cd30d699b366c7e2d4b932f6163caabb60855c65",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyjuice-module-0.7.1-macos-15-x86_64.tar.gz",
            sha256: "d0eaaead342d35ed27ebeaefadcea0dc19d73e9004c80165d6c629543bcc450f",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyjuice-module-0.7.1-windows-2025-x86_64.zip",
            sha256: "e97a0d5c1fa5fa055cfa1f85a4a824f7313c08b9f5a7677853494439728be2da",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyjuice-module-0.7.1-windows-2022-x86_64.zip",
            sha256: "62e2e134fe07502703fd5cd836b176aeac157dd9a233758b99225304e5be7dbd",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyjuice-module-0.7.1-windows-11-arm64.zip",
            sha256: "c59b6e68b85eca26d83c1d1fa2491f32e1d3afb871326faacccb5e0b6344402d",
        },
    ],
    load: LoadPolicy::Lazy,
};
