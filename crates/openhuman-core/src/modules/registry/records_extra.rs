//! Registry records for additional first-party TinyBus modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

/// The `tinybox` module, loaded on demand.
pub(crate) const TINYBOX: ModuleRecord = ModuleRecord {
    id: "tinybox",
    description: "Sandbox capability discovery through TinyBox",
    bus_name: "ai.tinyhumans.tinybox.Box",
    object_path: "/ai/tinyhumans/tinybox/Box",
    version: "0.1.19",
    release_url: "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.19",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinybox-0.1.19-ubuntu-24.04-x86_64.tar.gz",
            sha256: "a1dae0e4af4a1ba84fc37b048ad1d1ef0e226ec2c2b312d7607a8ec08d09d134",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinybox-0.1.19-ubuntu-24.04-arm64.tar.gz",
            sha256: "245eaf3ab5465e9e01da207ef63bf930c46069b3858fe9ce9da8e71559c0faf8",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinybox-0.1.19-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c23102c295b8e5ceef428d1ed178ac910031a23640eaebceb7ebfe3c19a2e05c",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinybox-0.1.19-ubuntu-22.04-arm64.tar.gz",
            sha256: "b81e147875a5f21b9b681dffa633c480cfa0bf0121e3ea309feb859d7206d866",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinybox-0.1.19-macos-26-arm64.tar.gz",
            sha256: "437ae2b09337b30ad1af85f3fe95be5cc0d8134986f401a231448a37c340f54b",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinybox-0.1.19-macos-26-x86_64.tar.gz",
            sha256: "24c3397596e20e3661e6fc6d0ba28f2f36a8c3d7f4e5106030f674d5d3703f6d",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinybox-0.1.19-macos-15-arm64.tar.gz",
            sha256: "61145d8dbb56d866d85610702938973b7c8f523093154ec3a27790dc060565bd",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinybox-0.1.19-macos-15-x86_64.tar.gz",
            sha256: "8e68322fe3ff762ad6b31a55413cfd7772174b2bdadc0b5391d1816d36a6b373",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinybox-0.1.19-windows-2025-x86_64.zip",
            sha256: "8ac5b73f23deee35d2f61209b801a5fcb6465277e35afc45dac4e91cfe528ce0",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinybox-0.1.19-windows-2022-x86_64.zip",
            sha256: "f2682b28908c9736382fab57c0f579db4a17ae3e037d9422f185a0f949e69143",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinybox-0.1.19-windows-11-arm64.zip",
            sha256: "50873a041c05d009f8f65812e6ac95fef001b1d71a897383da1f34fac1f2a7d0",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinychannels` module, loaded on demand.
pub(crate) const TINYCHANNELS: ModuleRecord = ModuleRecord {
    id: "tinychannels",
    description: "Channel provider lifecycle and message transport",
    bus_name: "ai.tinyhumans.tinychannels.Channels",
    object_path: "/ai/tinyhumans/tinychannels/Channels",
    version: "0.1.18",
    release_url: "https://github.com/tinyhumansai/tinychannels/releases/tag/v0.1.18",
    assets: &[
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinychannels-module-0.1.18-macos-15-arm64.tar.gz",
            sha256: "f152d03721ddd9ab5b0074f55b29958a141e023a0657e0f158f7f16070055607",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinychannels-module-0.1.18-macos-15-x86_64.tar.gz",
            sha256: "2492e497f45db161f588ff625ee18a59b0edaa8cc8e1e36e94a30e2604b48021",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinychannels-module-0.1.18-macos-26-arm64.tar.gz",
            sha256: "e5cdd8318e6c7d71a514e71c36ade1f0833a1ede250fb0026d3c4fce9a1c1f18",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinychannels-module-0.1.18-macos-26-x86_64.tar.gz",
            sha256: "13fec100c1bcf92cb4713fc7e7c83dc5e0a148d78ad7b9c7f71f48f94984c927",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinychannels-module-0.1.18-ubuntu-22.04-arm64.tar.gz",
            sha256: "fd0ff1d518b24e692d00cd387d2f721808f0d682fe2c98a91e98293a150337ad",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinychannels-module-0.1.18-ubuntu-22.04-x86_64.tar.gz",
            sha256: "ecc1bff5be6adf6ac6deb210523429fabe8d3ad2e877d7885f374c9cd21d0ac4",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinychannels-module-0.1.18-ubuntu-24.04-arm64.tar.gz",
            sha256: "120816212af496e9617e478dad2a1fc0f007bd01e507c1613d8f9255b6006500",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinychannels-module-0.1.18-ubuntu-24.04-x86_64.tar.gz",
            sha256: "24efc6a352339afed5a0bec99304fd9567defdd73f13cc350471ce83c7b3e8b8",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinychannels-module-0.1.18-windows-11-arm64.zip",
            sha256: "806aa6b642dbc618f010d4eef3e7a7aef37d45bbfdfec311e1c2a908f5973b60",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinychannels-module-0.1.18-windows-2022-x86_64.zip",
            sha256: "9433165bf99c27b834a5f9366d78709d3eb15c7acede4a8064e62af84d1da8fe",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinychannels-module-0.1.18-windows-2025-x86_64.zip",
            sha256: "4f6d659cca8559093aa91f4a49d0b718f4087d242473358e67ae1e35714e0682",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinyhosts` module, loaded on demand.
pub(crate) const TINYHOSTS: ModuleRecord = ModuleRecord {
    id: "tinyhosts",
    description: "Hosting provider operations",
    bus_name: "ai.tinyhumans.tinyhosts.Hosting",
    object_path: "/ai/tinyhumans/tinyhosts/Hosting",
    version: "0.3.0",
    release_url: "https://github.com/tinyhumansai/tinyhosts/releases/tag/v0.3.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyhosts-0.3.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "368f00ee397649481ff8f61d33eeb8b908cc416d6b2f9d4ef3c81263d6b0460e",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyhosts-0.3.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "e298942fddec8930fd78651632c02abe78968a17ab3589608875b8dc4f731669",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyhosts-0.3.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "f7c9484176b62357fb449eb79830e3df0eebfd4e0814c8d8e1c58669d829f482",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyhosts-0.3.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "576ff6204ccaa1e2dc2ef1b27762848efa4555d4d32ef46b4c329daf1e04eac3",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyhosts-0.3.0-macos-26-arm64.tar.gz",
            sha256: "b0083e70b567659d095a1ff0f9d2c8cb5bafe110d90fb08527c509362ce77a55",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyhosts-0.3.0-macos-26-x86_64.tar.gz",
            sha256: "44fa7c9efde0c388f154f154809ce2829460bae4931f27d20f8f0d3ee3bf2a8c",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyhosts-0.3.0-macos-15-arm64.tar.gz",
            sha256: "56ca93b2cab2010ff599c38179eb99f6ad67631e39f671be973256c3d9142381",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyhosts-0.3.0-macos-15-x86_64.tar.gz",
            sha256: "0ffb9fc8d1c7fc7d95a7808af7722d8c6537a508c0c3a294ecf8fb8a28430207",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyhosts-0.3.0-windows-2025-x86_64.zip",
            sha256: "3d6d5f7fd0806ac6fd6c2ed0c60a9155d6c09a861ebd1ac0c43864e969f88fb9",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyhosts-0.3.0-windows-2022-x86_64.zip",
            sha256: "a5003f1254052828d6fa362b993b0bed6b7256c252f969536090e148378c331f",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyhosts-0.3.0-windows-11-arm64.zip",
            sha256: "21f8909a2a81a541e3d0347510b2bf25baec59438a5536edcd57e65fae7f4a4d",
        },
    ],
    load: LoadPolicy::Lazy,
};
