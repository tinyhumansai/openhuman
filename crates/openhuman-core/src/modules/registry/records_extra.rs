//! Registry records for additional first-party TinyBus modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

/// The `tinybox` module, loaded on demand.
#[cfg(feature = "modules")]
pub(crate) const TINYBOX: ModuleRecord = ModuleRecord {
    id: "tinybox",
    description: "Sandbox execution and lifecycle operations through TinyBox",
    bus_name: tinybox_bus::INTERFACE,
    object_path: tinybox_bus::OBJECT_PATH,
    version: "0.1.42",
    release_url: "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.42",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinybox-0.1.42-archlinux-arm64.tar.gz",
            sha256: "5f276b8b3bc291710619775e5a8dffea3c65010040f8782c9e34ba0d2bc09fe6",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinybox-0.1.42-archlinux-x86_64.tar.gz",
            sha256: "570b2a0c693a3404e085c633ed9085833131d256c08ea32afddfcb0c6229ae72",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinybox-0.1.42-fedora-43-arm64.tar.gz",
            sha256: "1c19f32de81571691aca6c72cf8e23cda3f15a96849204f0b0ec2f19b5e95c52",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinybox-0.1.42-fedora-43-x86_64.tar.gz",
            sha256: "876d48362e1384b805fc80eda6bdd5840ae265f812d2a058e74bf6c6d7ce633d",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinybox-0.1.42-fedora-44-arm64.tar.gz",
            sha256: "cb765100fa700d751c51746697d499005eb1338dbe5d0aaf3a2cc70975eb35e2",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinybox-0.1.42-fedora-44-x86_64.tar.gz",
            sha256: "d3ec5b0ab0b5bafb9ecc61701997aa1bfb84d5c4afdff64aa90c4500a087d030",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinybox-0.1.42-macos-15-arm64.tar.gz",
            sha256: "d87fec4edace1f14514b46d4a442d8a3e124eeb8890cc1dcf851fcd4e663c257",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinybox-0.1.42-macos-15-x86_64.tar.gz",
            sha256: "aa5b5920e5463db8fc8b891c81bd1695de67373809b93aa331c2ba5800309b90",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinybox-0.1.42-macos-26-arm64.tar.gz",
            sha256: "1a06b12a46cbd48b8bd8ff2ef3a90b3df2421df8e13e0541ce12dea7dac21345",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinybox-0.1.42-macos-26-x86_64.tar.gz",
            sha256: "0efcbf868c59e8ba933fc745bc8bb51cb5052f6574de4f22df50195a45ba65a4",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinybox-0.1.42-ubuntu-22.04-arm64.tar.gz",
            sha256: "9876bc55ceca33cceebeff78e9e81fb6edc3a8b1a876c71c727cce942833ead9",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinybox-0.1.42-ubuntu-22.04-x86_64.tar.gz",
            sha256: "63c40e2577b97ce0ba369657a153e725a8db89b0043abfc3cb874075c266ecd5",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinybox-0.1.42-ubuntu-24.04-arm64.tar.gz",
            sha256: "58b533cd33ed5d3ef83814dbde6a9e417bb83f2087a9717f1a98a0babdac6c7b",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinybox-0.1.42-ubuntu-24.04-x86_64.tar.gz",
            sha256: "2dcc913153c5c6a03b88cc41e8aa14151159b308799b9174eb9e1a27edf1cf36",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinybox-0.1.42-windows-11-arm64.zip",
            sha256: "19e781b93bcb0d401ff6b9349b62bfd00d05f92e82875eed36d0689d3484692b",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinybox-0.1.42-windows-2022-x86_64.zip",
            sha256: "e9f8cc22dfce616bdf57af076ca33390c37f1417b56a26d58d265b00564ee52d",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinybox-0.1.42-windows-2025-x86_64.zip",
            sha256: "8ace9208d6a89a6d4d2766a91d5f9be937e1aaf076b4a7ca3af5c965997167d3",
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinybox",
        manifest: "crates/tinybox-module/Cargo.toml",
        package: "tinybox-module",
        lib_name: "tinybox",
        features: &[],
    },
};

/// The `tinychannels` module, loaded on demand.
pub(crate) const TINYCHANNELS: ModuleRecord = ModuleRecord {
    id: "tinychannels",
    description: "Channel provider lifecycle and message transport",
    bus_name: tinychannels_bus::names::BUS_NAME,
    object_path: tinychannels_bus::names::OBJECT_PATH,
    version: "0.1.39",
    release_url: "https://github.com/tinyhumansai/tinychannels/releases/tag/v0.1.39",
    assets: &[
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinychannels-module-0.1.39-macos-15-arm64.tar.gz",
            sha256: "2a360880d72239bb370fd25c50f384f7c07a6a349d2da7854645b88fe2bc567f",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinychannels-module-0.1.39-macos-15-x86_64.tar.gz",
            sha256: "672c0aa2a34954c0445fdcf532f4507468288f045925950a4c5cd726a34a16b8",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinychannels-module-0.1.39-macos-26-arm64.tar.gz",
            sha256: "2c89dee2d3b99f6879ede752b29ffce2532ac7b4c37f0d1cd9f51693d97ca807",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinychannels-module-0.1.39-macos-26-x86_64.tar.gz",
            sha256: "725b3b02bf312523940551b8456a043a6b0fec56c4738ee751776206438d2b76",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinychannels-module-0.1.39-ubuntu-22.04-arm64.tar.gz",
            sha256: "a0356ad855de475cdb56bb60a501a75adceb07a860ac06c15ce4aa57dbc35ab2",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinychannels-module-0.1.39-ubuntu-22.04-x86_64.tar.gz",
            sha256: "d14e379f625ed1dad0f2f0d5ca6e76d57caa2c678a929f11de096a55df8ebbbc",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinychannels-module-0.1.39-ubuntu-24.04-arm64.tar.gz",
            sha256: "89dac4db8429b0eb3a99a5edc8fd56e1edeef76ffcfc996aa073810a05c6b5a4",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinychannels-module-0.1.39-ubuntu-24.04-x86_64.tar.gz",
            sha256: "04c53b7649b1f0323fc4af0bed6f7518c1feadbf570262c5a96a0c2879f51287",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinychannels-module-0.1.39-windows-11-arm64.zip",
            sha256: "e9da5f08300cf40e1f23f7bea1abb5322b69425178e0b59df6eaa3e75e16aed5",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinychannels-module-0.1.39-windows-2022-x86_64.zip",
            sha256: "b4fb4dfb2019593ddd1dc336d660bd3926ba39dc2e0e8849dc8c06053acea53b",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinychannels-module-0.1.39-windows-2025-x86_64.zip",
            sha256: "4adefa0e8589a52f2cd92d5839c78b52476f03734a6f7f534283f2b6b2ad436c",
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinychannels",
        manifest: "crates/tinychannels-module/Cargo.toml",
        package: "tinychannels-module",
        lib_name: "tinychannels_module",
        features: &[],
    },
};

#[cfg(feature = "hosting")]
/// The `tinyhosts` module, loaded on demand.
pub(crate) const TINYHOSTS: ModuleRecord = ModuleRecord {
    id: "tinyhosts",
    description: "Hosting provider operations",
    bus_name: tinyhosts_bus::BUS_NAME,
    object_path: tinyhosts_bus::OBJECT_PATH,
    version: "0.4.1",
    release_url: "https://github.com/tinyhumansai/tinyhosts/releases/tag/v0.4.1",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyhosts-0.4.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "7e8a72b7951db71cc98c94fed0908660a58bd3f2cbf23103d953e28e1315d4d5",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyhosts-0.4.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "27cbc5a233292850797adc7b51d9d654a10d4aa77ec285a7e564e378889e0a03",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyhosts-0.4.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "a03588ef8b012aea29f2b2b7105fde6c3c7b4bb0dc6f93d99868e91468c2b6d6",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyhosts-0.4.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "80b26591eb7beff4e27de9e6239b919ff6c17b00ead11d22bae8e7fcb0d33c15",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyhosts-0.4.1-macos-26-arm64.tar.gz",
            sha256: "ba40e889ebd450edd791865945b4146b2f243bc885f4228463e65b0a4ea8074e",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyhosts-0.4.1-macos-26-x86_64.tar.gz",
            sha256: "eeb2a66603374934724036e9fb86fcba0c22a2fd3aa168127cca5960eb5a19aa",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyhosts-0.4.1-macos-15-arm64.tar.gz",
            sha256: "b67a0aecb0527af68d5b5428d55d897ece11500e2ee9798b3d133ef0df7344b7",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyhosts-0.4.1-macos-15-x86_64.tar.gz",
            sha256: "7582dc327e247b28e657a01c8f8a4cb0924c8211144dfb24e4917868cd237354",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyhosts-0.4.1-windows-2025-x86_64.zip",
            sha256: "42c2a5b98eb5c51a8a05731e762af801c585383024d27cf4cd8bb01f9014308f",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyhosts-0.4.1-windows-2022-x86_64.zip",
            sha256: "0d579e0d08856307881b8965bf78762f0064b129861e13e796f91fcb9895ccaf",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyhosts-0.4.1-windows-11-arm64.zip",
            sha256: "67b64f0917fa0cf561aa08dfe0c74818e9ac9066a9f03c03e6b216f7af87aea3",
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyhosts",
        manifest: "Cargo.toml",
        package: "tinyhosts",
        lib_name: "tinyhosts",
        features: &[],
    },
};
