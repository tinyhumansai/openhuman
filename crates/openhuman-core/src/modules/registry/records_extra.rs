//! Registry records for additional first-party TinyBus modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

/// The `tinybox` module, loaded on demand.
#[cfg(feature = "modules")]
pub(crate) const TINYBOX: ModuleRecord = ModuleRecord {
    id: "tinybox",
    description: "Sandbox execution and lifecycle operations through TinyBox",
    bus_name: tinybox_bus::INTERFACE,
    object_path: tinybox_bus::OBJECT_PATH,
    version: "0.1.35",
    release_url: "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.35",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinybox-0.1.35-archlinux-arm64.tar.gz",
            sha256: "63f6c029317ab539a2c8f922d5cbb9bf9ee69fa185ff3ea94b1050c98f4537aa",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinybox-0.1.35-archlinux-x86_64.tar.gz",
            sha256: "3f2b1fafd32819fecfe22c86423566ea47e416a5531d82e9c76ef2f3cb5cd0e1",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinybox-0.1.35-fedora-43-arm64.tar.gz",
            sha256: "4064fff0f9d67acb8658f7d7669c9beb26f56c63f572fd406aac720b5817e5cf",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinybox-0.1.35-fedora-43-x86_64.tar.gz",
            sha256: "a62c55240465bef70d653fbc875b959dca49ef6cc37c472c1b798db047c7bf08",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinybox-0.1.35-fedora-44-arm64.tar.gz",
            sha256: "c618d63c544890f370e21fb9d4278ab3ba4d65436c4665a0f96e5dfe709815d4",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinybox-0.1.35-fedora-44-x86_64.tar.gz",
            sha256: "e96d326d9c3b1572c1dabca60e5859d3ace02c0bfcc9b695937f2a1f083f5ecf",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinybox-0.1.35-macos-15-arm64.tar.gz",
            sha256: "93295b38243fb192191739dcca5ccec457561d27d9a9ead0ffc7633fe1f65771",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinybox-0.1.35-macos-15-x86_64.tar.gz",
            sha256: "a4a63bbdc630dea58c03ec009dc9969ae9a07196e49a91cc5657345b9df89f54",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinybox-0.1.35-macos-26-arm64.tar.gz",
            sha256: "e416e25042a442ce01fceb18a5993eac7b702457a162d333981bb3eced16c50c",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinybox-0.1.35-macos-26-x86_64.tar.gz",
            sha256: "7228eda7813faac69c25d52aaefd08ad96e7513288ac498dc5dcb8fc8831aa58",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinybox-0.1.35-ubuntu-22.04-arm64.tar.gz",
            sha256: "acf4b24d64ee7457311a651aa6649045a523322bb4d71fb614f0978777001deb",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinybox-0.1.35-ubuntu-22.04-x86_64.tar.gz",
            sha256: "d0d383df305f3d633063f01cf81c9ffcb3469f09532043b8be8a90f66f4b32d5",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinybox-0.1.35-ubuntu-24.04-arm64.tar.gz",
            sha256: "1a627a284532ba3cbd86689bc6961bbb38f8e5fa710eaade10d7f0d1d66fcd66",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinybox-0.1.35-ubuntu-24.04-x86_64.tar.gz",
            sha256: "6bcfb35c9c14777859f385575920e9d4a8a6af1a46a42370db464ab6de3c0067",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinybox-0.1.35-windows-11-arm64.zip",
            sha256: "def7bc85fcdf431d3b3a298f1f498b9674528be257725329df49c868b2893b9a",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinybox-0.1.35-windows-2022-x86_64.zip",
            sha256: "e2d2776969108a5f273c4f8ab309161ae495ff7cb7791dff869cc1c03ca40d5b",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinybox-0.1.35-windows-2025-x86_64.zip",
            sha256: "95801c1ee5a536e8dc6416d18426b82fc7c2be7d812b2acb1215f0b49c9dc14a",
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
