//! Registry records for additional first-party TinyBus modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

/// The `tinybox` module, loaded on demand.
#[cfg(feature = "modules")]
pub(crate) const TINYBOX: ModuleRecord = ModuleRecord {
    id: "tinybox",
    description: "Sandbox execution and lifecycle operations through TinyBox",
    bus_name: tinybox_bus::INTERFACE,
    object_path: tinybox_bus::OBJECT_PATH,
    version: "0.1.34",
    release_url: "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.34",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinybox-0.1.34-archlinux-arm64.tar.gz",
            sha256: "d10557f46d0be57de5f337977189f5b9e793da404ab9c0ebdf0691a695f68c73",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinybox-0.1.34-archlinux-x86_64.tar.gz",
            sha256: "96dda3c3b6b22bdb36f2bc0a6d01dda8798517d66b11225eb51af7c3598846d2",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinybox-0.1.34-fedora-43-arm64.tar.gz",
            sha256: "38296b71d7c578b2a8e3f3555b2be46f69bea3659bdefbc83d9624f9433e8dc6",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinybox-0.1.34-fedora-43-x86_64.tar.gz",
            sha256: "c59a74303a973c046ceb205093fa02332d70f8c3c6c8442a1f73bd19655bf77c",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinybox-0.1.34-fedora-44-arm64.tar.gz",
            sha256: "b6f50465f1fc28f4edc1d4d7f182c20cbf943da270debaf2ac22661d19969bb9",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinybox-0.1.34-fedora-44-x86_64.tar.gz",
            sha256: "a4ca0e455387927223a673b51e6c7203387e31d96a9f69fdbf35a9441bd81a24",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinybox-0.1.34-macos-15-arm64.tar.gz",
            sha256: "01d5685b15548732699db975f18143f050bbdc551d3b8d1580e457d8bf9b4b94",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinybox-0.1.34-macos-15-x86_64.tar.gz",
            sha256: "dd57ecb79e342394d9847767e89143d59720f9a934803805e2968363d526bcb7",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinybox-0.1.34-macos-26-arm64.tar.gz",
            sha256: "d3a92cd86bd31d38a5f47bdefde9a643de33324d2d336949d10ab61b9349fd94",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinybox-0.1.34-macos-26-x86_64.tar.gz",
            sha256: "9bc4875edc26b824014579b4723dacd6f29730bc7be961a25e8ae9de48c54970",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinybox-0.1.34-ubuntu-22.04-arm64.tar.gz",
            sha256: "99b925dcfd8cfffbd3d0239a3d0ef63bab6e4d88f28e29d3a7666031eb459ad9",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinybox-0.1.34-ubuntu-22.04-x86_64.tar.gz",
            sha256: "7db09dbbd8dd1b3a11dc40f1ea9d0c24e7c6e2c0feb14c4e859fa508a3fbab4f",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinybox-0.1.34-ubuntu-24.04-arm64.tar.gz",
            sha256: "8066e12a0ad477c66b28e2218a8118039bb1b2976184409d9e7b5f52e620edfc",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinybox-0.1.34-ubuntu-24.04-x86_64.tar.gz",
            sha256: "9f8e5765bd6e18450c9b47c0c92e8320d152e5170814b037f82b8afc30be6fa4",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinybox-0.1.34-windows-11-arm64.zip",
            sha256: "e53c68f35ebf63c6afebeabde7dbdad4a9b7b912b8e40600183f7b01695f05e6",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinybox-0.1.34-windows-2022-x86_64.zip",
            sha256: "820d751b12e9fe659319665da0b1c9ee5c07a207544761a20e0b0afdb7b065a4",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinybox-0.1.34-windows-2025-x86_64.zip",
            sha256: "84cec6888f732d6a202ea5a9a33f945323f55cf064557c5e1e4c49f2f5a936ff",
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
    version: "0.1.34",
    release_url: "https://github.com/tinyhumansai/tinychannels/releases/tag/v0.1.34",
    assets: &[
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinychannels-module-0.1.34-macos-15-arm64.tar.gz",
            sha256: "b58f68ac45af467fa9eea0443afa59ed17a2f4e7ab9c3f4223ded6d83af88b22",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinychannels-module-0.1.34-macos-15-x86_64.tar.gz",
            sha256: "ec04b49b184388b514ffc4aad9e130141987bd28d9cc5f4a76daf87c25e28a98",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinychannels-module-0.1.34-macos-26-arm64.tar.gz",
            sha256: "7b4f3be24e83a056e68d072cc1a5e00420d18f30eef0cd767cb6be40d0b2ad95",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinychannels-module-0.1.34-macos-26-x86_64.tar.gz",
            sha256: "08e3d4ad69b30c46b06f4846133bb8066396e95f8f161f98c7d20d375b50453c",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinychannels-module-0.1.34-ubuntu-22.04-arm64.tar.gz",
            sha256: "035d782536952a7e60d3ba9011ee96cca71e63d515ef78e7397d4800ecf3aece",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinychannels-module-0.1.34-ubuntu-22.04-x86_64.tar.gz",
            sha256: "98292be5f921c027a79e41b463ffc9981fca139e9c269bb8daf882392c97817c",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinychannels-module-0.1.34-ubuntu-24.04-arm64.tar.gz",
            sha256: "57af460080ecdd14970e47dc02d8f27334cf165ab70ba3fb82e34dc88e445329",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinychannels-module-0.1.34-ubuntu-24.04-x86_64.tar.gz",
            sha256: "e153a65569f3e7aba90f78c3fe6ada4684280ea02776d5263a9df19071335fc7",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinychannels-module-0.1.34-windows-11-arm64.zip",
            sha256: "ef163bcd82fcdeafbb364f1f287eda0e9778255e04fb4062534ccf365f87bdeb",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinychannels-module-0.1.34-windows-2022-x86_64.zip",
            sha256: "a570821ef804e71ad98c747e51a22b8631a7d3db760e99b240cf666fafe67ced",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinychannels-module-0.1.34-windows-2025-x86_64.zip",
            sha256: "95bbf5aca663af06a6e6b912fa519e249fd1f454c65c0bb2993d1e1a79973316",
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
