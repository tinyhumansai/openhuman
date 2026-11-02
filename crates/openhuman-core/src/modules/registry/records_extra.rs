//! Registry records for additional first-party TinyBus modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

/// The `tinybox` module, loaded on demand.
#[cfg(feature = "modules")]
pub(crate) const TINYBOX: ModuleRecord = ModuleRecord {
    id: "tinybox",
    description: "Sandbox execution and lifecycle operations through TinyBox",
    bus_name: tinybox_bus::INTERFACE,
    object_path: tinybox_bus::OBJECT_PATH,
    version: "0.2.4",
    release_url: "https://github.com/senamakel/tinybox/releases/tag/v0.2.4",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinybox-0.2.4-archlinux-arm64.tar.gz",
            sha256: "526301c2cf392d059fba0faaecb6f78fcc65610f68afada4025e72248934ad05",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinybox-0.2.4-archlinux-x86_64.tar.gz",
            sha256: "4457db70cf5c073d37bb624f30ff372ed7c430cad4194fb055f568f3af5bd294",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinybox-0.2.4-fedora-43-arm64.tar.gz",
            sha256: "ac7ec226b90e55a33ee8c1561ca0ce49bc1506ca45add618d498002c83e47a8d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinybox-0.2.4-fedora-43-x86_64.tar.gz",
            sha256: "506b813f2044eb7c6a52b34df0766103e1f7e5de02db27a86bc4003e7d75ba3e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinybox-0.2.4-fedora-44-arm64.tar.gz",
            sha256: "eb28e5e1c313d4b7a8450f50461897c57c100e2598d53577314372b547367cf0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinybox-0.2.4-fedora-44-x86_64.tar.gz",
            sha256: "78d181a10a23f48e27bb16478b25834b36b66b22cee857f3eef17d67f823113e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinybox-0.2.4-macos-15-arm64.tar.gz",
            sha256: "086f4eb50ac189049e04997e115923a492cf5a17016ed3d6096bae99e5378920",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinybox-0.2.4-macos-15-x86_64.tar.gz",
            sha256: "f3804d496d77c9717a3d72fb50e0426afbca7dcb44e4efdd859f389853dad60a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinybox-0.2.4-macos-26-arm64.tar.gz",
            sha256: "20196099555eb05c10dd9565147f32567237f187248e2c76fb97ec073df366ea",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinybox-0.2.4-macos-26-x86_64.tar.gz",
            sha256: "5117ac41154c447e8da5bfdf8fd4119c58ea0509233c4e21e41d8fda3b329440",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinybox-0.2.4-ubuntu-22.04-arm64.tar.gz",
            sha256: "40b0aeb0bcc09d278d09d368e513aeb11285dbd8d3ba5c7050eed5cfc849858e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinybox-0.2.4-ubuntu-22.04-x86_64.tar.gz",
            sha256: "48b67a466e9bfe8cfffddb2f90d70bd15161416f6c0592bb8c3fb66a57843da0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinybox-0.2.4-ubuntu-24.04-arm64.tar.gz",
            sha256: "365b1b40822dd007de4d4e1d8daf0db9442bb9a7f034a70de7e3b45c58b8d45c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinybox-0.2.4-ubuntu-24.04-x86_64.tar.gz",
            sha256: "9f58a872d904d9d5d23d92bb431cb39894626adf955ceed41ab8bfd24dc74f26",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinybox-0.2.4-windows-11-arm64.zip",
            sha256: "033be4c72baf00e77e70741df2fbcc6deb6872a7d5e36b49b880f9c01dbf3d3a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinybox-0.2.4-windows-2022-x86_64.zip",
            sha256: "ede48ab03fe43717fc05e7f67a355da51eff876d5bdb233acbba780aca1d6d81",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinybox-0.2.4-windows-2025-x86_64.zip",
            sha256: "235cbf0fd280758ebb6ba49bd57c440b0393439588b73474c7315564407d411a",
            library_sha256: None,
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
    version: "0.1.40",
    release_url: "https://github.com/tinyhumansai/tinychannels/releases/tag/v0.1.40",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinychannels-module-0.1.40-archlinux-arm64.tar.gz",
            sha256: "2643b5aefb51a882739640666e456022921304b1ff3cf935005803e1baeae2db",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinychannels-module-0.1.40-archlinux-x86_64.tar.gz",
            sha256: "b65e7e2e99af174166ce18acce7b3e9948b03f93644ac34958a2dd1d45b9783f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinychannels-module-0.1.40-fedora-43-arm64.tar.gz",
            sha256: "06470acac32353419b9e2c8eed0b13244de9957bc6d7575932b8bf5989579780",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinychannels-module-0.1.40-fedora-43-x86_64.tar.gz",
            sha256: "5956996b8fdc829e530a5cf808a8f9561dba7f91aead25df4f091a6c5ead605f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinychannels-module-0.1.40-fedora-44-arm64.tar.gz",
            sha256: "e449125869811077cb020b6dbd9f5dffc25fbf1526b8ee2402f0fac0b2f53bbc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinychannels-module-0.1.40-fedora-44-x86_64.tar.gz",
            sha256: "f5ab13f94fe850ec90a95f1d527ee9c55ab3fdeb427f3c1c2e390c35b300d76e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinychannels-module-0.1.40-macos-15-arm64.tar.gz",
            sha256: "98d31e633601c365200c679f288b13f9d7aef2023ebb9853bcbad690ddbbe1a0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinychannels-module-0.1.40-macos-15-x86_64.tar.gz",
            sha256: "4ac740b410102b29ba469f37f450b3e03fac3ce31ccb9d3538c1633025a7bd4d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinychannels-module-0.1.40-macos-26-arm64.tar.gz",
            sha256: "ed2aab10a198193e5bebb5957095acad664a5a66a02d6142563815df6c353b53",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinychannels-module-0.1.40-macos-26-x86_64.tar.gz",
            sha256: "210a7904dd942629af46d7ad36d0b0312f3a1cf0aa238b667ca6309ebf86c8e2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinychannels-module-0.1.40-ubuntu-22.04-arm64.tar.gz",
            sha256: "48d360aea7849bb93050dbb56f9c28de6848572632908b635fb77df2a65c3bb1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinychannels-module-0.1.40-ubuntu-22.04-x86_64.tar.gz",
            sha256: "6026081ac8ceb7b91daf0101364c92f99c10c5072a270b91eaeed6acad5d1f90",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinychannels-module-0.1.40-ubuntu-24.04-arm64.tar.gz",
            sha256: "0b4ef6b3c9c2c81f1efdbc5cbe8713478ff4e2f76682625465ee8c7120dfb798",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinychannels-module-0.1.40-ubuntu-24.04-x86_64.tar.gz",
            sha256: "bec22856f0b5f36f8c48f893c53d83015961a596d4ad56a33714ca378c2b4c98",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinychannels-module-0.1.40-windows-11-arm64.zip",
            sha256: "5f463711be6f8c856e13f77ba5d376b1b4e5b9519551411ae7bfd94a196bd633",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinychannels-module-0.1.40-windows-2022-x86_64.zip",
            sha256: "5df4ce49573c7d1e03a93b50cf092bf62dc6b92f50bcfe1471aff50a844268ce",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinychannels-module-0.1.40-windows-2025-x86_64.zip",
            sha256: "e8dd10cc102a7889d7d9ba8f50416b2ee3cec810f203555bf40cd0f157765552",
            library_sha256: None,
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
    version: "0.4.3",
    release_url: "https://github.com/tinyhumansai/tinyhosts/releases/tag/v0.4.3",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyhosts-0.4.3-archlinux-arm64.tar.gz",
            sha256: "97388fd17b72b92f0384397378fece2e7d0806004676edcdfc2b1798669b67a8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyhosts-0.4.3-archlinux-x86_64.tar.gz",
            sha256: "93e0500e30b77aca6d05de69636c4afc014a3856809e3f781868635d626ef9c0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyhosts-0.4.3-fedora-43-arm64.tar.gz",
            sha256: "fef318d960bbaea005a2f1f8bd1c64ff0bce09a3e4cef4922e7a75019dadd29f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyhosts-0.4.3-fedora-43-x86_64.tar.gz",
            sha256: "cef9120fc178bf5b40f06e23727d47901746952aab4129eecbc3babd2f516421",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyhosts-0.4.3-fedora-44-arm64.tar.gz",
            sha256: "be1a4c1c2da9e8745fd18a3d5c050cc99fc6655f6eba2e914e191086e7b0ec6d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyhosts-0.4.3-fedora-44-x86_64.tar.gz",
            sha256: "ed5a96104a1db21f3852e0d3824cb493c5091318d36f44de97122abc8f7969ef",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyhosts-0.4.3-macos-15-arm64.tar.gz",
            sha256: "79a3fa3c643f50d7e46fec58bfbd3117590853ab76792175b62b4325846377cf",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyhosts-0.4.3-macos-15-x86_64.tar.gz",
            sha256: "4ac3e43e3d48210f40361b199760df738ed3ab01c248f2d45495a8e4c1d50509",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyhosts-0.4.3-macos-26-arm64.tar.gz",
            sha256: "efff57af745179807ef8eb41a1c83df6cce1cde23ae664ac8e2e67065dae06b7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyhosts-0.4.3-macos-26-x86_64.tar.gz",
            sha256: "6c7f70714c7d2246436487ce986c58e1962a393d1d9f3db6d1634c7ce3f75dad",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyhosts-0.4.3-ubuntu-22.04-arm64.tar.gz",
            sha256: "d554e1a524040b0ae023ff6aec0c0129d9e01526077ca5d6ba1249ce5cd440f0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyhosts-0.4.3-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c6d7cf38c8b3e062e1a8af7785cf8c902c6e7d2d1be51a502407537369210b50",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyhosts-0.4.3-ubuntu-24.04-arm64.tar.gz",
            sha256: "9eec6afb328b22dad77e96ca802c39a83656f0d57faea9c2d2d2017547e03c0b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyhosts-0.4.3-ubuntu-24.04-x86_64.tar.gz",
            sha256: "70da542ec03e90485c84b8ef1f75867b15f09292ac941d3a457be8bec410f6bc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyhosts-0.4.3-windows-11-arm64.zip",
            sha256: "28e10831884064238ebf897d49eb7050f98e5550f7552061015d018fe4c778d3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyhosts-0.4.3-windows-2022-x86_64.zip",
            sha256: "19e525b143662e3580d90dcba003b661a97b5bcd8ed4e00872081766cdd2b59d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyhosts-0.4.3-windows-2025-x86_64.zip",
            sha256: "83483a4e5eaf8fd0f02c7bc6575d506f443bf0938408288e4cc4930f092fac89",
            library_sha256: None,
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
