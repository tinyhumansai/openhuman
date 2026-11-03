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
    version: "0.4.5",
    release_url: "https://github.com/tinyhumansai/tinyhosts/releases/tag/v0.4.5",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyhosts-0.4.5-archlinux-arm64.tar.gz",
            sha256: "3acc6a429997a067a39b91b2c414ccc8f0352245147be127173a2ed6965abd3d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyhosts-0.4.5-archlinux-x86_64.tar.gz",
            sha256: "1d4104f2d1da999c05c08a6226a3b324e77fc362cfe2ba7c0b5a81ac17e31c48",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyhosts-0.4.5-fedora-43-arm64.tar.gz",
            sha256: "6bed53d9a240f28940faf48e3391d9e45192cc1e9b9ba4470343b43f765f24bb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyhosts-0.4.5-fedora-43-x86_64.tar.gz",
            sha256: "d11945bb9f328c656ed0ce706b46ebeff1c7b8547f93e9232748cfeababd8f57",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyhosts-0.4.5-fedora-44-arm64.tar.gz",
            sha256: "c2ad9b3128c32da6fd3575904aef262e219f949c08745c1d5b9a1d48b0bf095d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyhosts-0.4.5-fedora-44-x86_64.tar.gz",
            sha256: "bb5704decd33d35a6054961ebb7e4ca9a4934bb642e50c1aa5a0a1d6a9b30d1d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyhosts-0.4.5-macos-15-arm64.tar.gz",
            sha256: "7314f37b28cdb4b891d0441eeefb5fdddf39bb6760ddd1b33b55304c683263f8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyhosts-0.4.5-macos-15-x86_64.tar.gz",
            sha256: "f412018f1b74e26e0ab837a59d3f3c1f5c3da316f5858d5a3621e85a9366b9d3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyhosts-0.4.5-macos-26-arm64.tar.gz",
            sha256: "1bc91723ad21eae0a0fd221af38f7b8349c33314753e842a6547586908d79669",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyhosts-0.4.5-macos-26-x86_64.tar.gz",
            sha256: "fce287ecacbeb0f383386cf9a71a8efd70f2e70849a00786a69ce5e8aef434d2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyhosts-0.4.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "a79976107cfd2d439d6f692d2479ce7a6f336e1550a67c9131d6f6f41ed07ff0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyhosts-0.4.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "b6bd602900fd9887fd91a9dec762b94add86b49d19759a1de1af271d52dc7ad7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyhosts-0.4.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "32a560d5a53723f0338020b9d889ec01354b463b21fdec21807d576ba5d99240",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyhosts-0.4.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "62e71cd7c5e0f414d7059f96d4b62bfb952e8aab6c7ae3d716f93da3b7e96791",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyhosts-0.4.5-windows-11-arm64.zip",
            sha256: "e433c2881178727693d90bba7d4ba33ec1439ee28c3ca3d9e7eb1b1c4cabb0dc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyhosts-0.4.5-windows-2022-x86_64.zip",
            sha256: "515105650694e8b290e8aa53039dee760054ce6c791a9901e57b8225139e9274",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyhosts-0.4.5-windows-2025-x86_64.zip",
            sha256: "8b9b5ba24fb645545e0e37b433deb244d4f65d26c58a8ff5acc7c05b95d9e5df",
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
