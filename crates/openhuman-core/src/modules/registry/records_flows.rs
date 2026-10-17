//! TinyFlows module: the scheduler runtime, the cron job and run store, and the
//! schedule maths behind `tinyflows_bus::schedule` (contract v3), plus workflow
//! authoring. Published digests are added only from its release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYFLOWS: ModuleRecord = ModuleRecord {
    id: "tinyflows",
    description:
        "Scheduled jobs (runtime, job store, schedule maths) and workflow authoring through TinyFlows",
    bus_name: tinyflows_bus::BUS_NAME,
    object_path: tinyflows_bus::OBJECT_PATH,
    version: "0.8.23",
    release_url: "https://github.com/tinyhumansai/tinyflows/releases/tag/v0.8.23",
    // Verbatim from the published v0.8.23 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyflows-module-0.8.23-macos-26-arm64.tar.gz",
            sha256: "1ea581047502dca7e99cc701342b118f5684db44083a5dcbaeb3a924c3fe9f8f",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyflows-module-0.8.23-macos-26-x86_64.tar.gz",
            sha256: "d7a9ca4fee0b331652dd0ec5e7dc51bde59674fcf225d5102691e8d5b1b03927",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyflows-module-0.8.23-macos-15-arm64.tar.gz",
            sha256: "cd1811acc26c928a96e962b9b042dc4c2d60a318b7cb08837773812d04ce7973",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyflows-module-0.8.23-macos-15-x86_64.tar.gz",
            sha256: "084c9255dddaef3be06e0c6e733ed7cc408e5338df624ea79fbdbb304f04571c",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyflows-module-0.8.23-ubuntu-24.04-x86_64.tar.gz",
            sha256: "990870ba9e942eee481f3b4252ec840cd3ed304ad981d9d6fb9032aa4bbfdab1",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyflows-module-0.8.23-ubuntu-24.04-arm64.tar.gz",
            sha256: "867b2176f0b7e0f40b18a1f95dbfb81a876b831ccd918da2db77a893d00556d2",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyflows-module-0.8.23-ubuntu-22.04-x86_64.tar.gz",
            sha256: "d53a8f278e738e3beba2282462c55af4640ea9e2d101e5bb3143e654d6915189",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyflows-module-0.8.23-ubuntu-22.04-arm64.tar.gz",
            sha256: "b9dfbb484a7a9beab55227e82d12855842cb65d82f5c0372bc7dd0ac971aab22",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyflows-module-0.8.23-windows-2025-x86_64.zip",
            sha256: "114f5da7b480afba89bf21478e5e196a6490b3869dc253b51e99fcf76d0b1978",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyflows-module-0.8.23-windows-2022-x86_64.zip",
            sha256: "21d90646ce5b7abaa3c1cba1c19d0cc8784753a7d07c302a377d07fcd8c31a97",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyflows-module-0.8.23-windows-11-arm64.zip",
            sha256: "73f0aa5f15a8ee876e744027ac9f0c68d41e243872d15c4735ab6bbf54e7d51d",
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyflows",
        manifest: "crates/tinyflows-module/Cargo.toml",
        package: "tinyflows-module",
        lib_name: "tinyflows_module",
        features: &[],
    },
};
