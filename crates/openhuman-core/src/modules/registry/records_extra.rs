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
    release_url: "https://github.com/tinyhumansai/tinybox/releases/tag/v0.2.4",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinybox-0.2.4-archlinux-arm64.tar.gz",
            sha256: "aaf2c37f5ebe92c8897fe8b6b0300798e7afecbb00f3f438ccd19aa566f3a6ec",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinybox-0.2.4-archlinux-x86_64.tar.gz",
            sha256: "07937b998542884ed38502c534c4d857d7e9fb55da0b5ae0a5ec6d9d7a369c29",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinybox-0.2.4-fedora-43-arm64.tar.gz",
            sha256: "feec38ccea69e54962387321ac6377d207ca99c0de51781e1646b88ae6b3f899",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinybox-0.2.4-fedora-43-x86_64.tar.gz",
            sha256: "ff9aabc80750b956fb1d7824f650b51bf54b2027838641432027c03b14bef1e9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinybox-0.2.4-fedora-44-arm64.tar.gz",
            sha256: "ea56348b0b33d71e6fde07bba1b32be2c659a8be94c45feacb92b6d0e9e87d5e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinybox-0.2.4-fedora-44-x86_64.tar.gz",
            sha256: "f10fc89683e7cf0c9bc5b9baaf5d3dfce9ec75f4b7c7aed928e4668392c50d5f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinybox-0.2.4-macos-15-arm64.tar.gz",
            sha256: "0983c74b6ac9a5458518ebcb557bc2d5d3d91c2ab3ecee01f4a6de0a60dc7271",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinybox-0.2.4-macos-15-x86_64.tar.gz",
            sha256: "c068a76205f1bf435a490b5248a8408ac8aa2537b1e9d446583b8dfe194be34e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinybox-0.2.4-macos-26-arm64.tar.gz",
            sha256: "13aa4c590177eea88f0da67000494eb72fe40c238c477a2755eb6a439e8b66e3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinybox-0.2.4-macos-26-x86_64.tar.gz",
            sha256: "99e5159922eb99eaec894e49a7dcd947c0a369e5c51192052febf38c1a15a72a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinybox-0.2.4-ubuntu-22.04-arm64.tar.gz",
            sha256: "abcb145cc78d48a1cea4a7e3a94414d7d11e1ff70adef660cab94c11832fbaf3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinybox-0.2.4-ubuntu-22.04-x86_64.tar.gz",
            sha256: "39ea65a08bc1aa2be74121ebca9e815be379577dc461380c4c3eb926b1b7d735",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinybox-0.2.4-ubuntu-24.04-arm64.tar.gz",
            sha256: "0d9fa4e342afd59c2c3c1b6edf24a1d0383430c9ef238cb214b1c849ecc06f0f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinybox-0.2.4-ubuntu-24.04-x86_64.tar.gz",
            sha256: "8424a0a42894f610eb81fcfdbc6eb87f642df953a5d886127d03cdf3e51a52d1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinybox-0.2.4-windows-11-arm64.zip",
            sha256: "75163660dd3eec73b1bfc0256d5453bb81e10484a9584295c0021c1ffb3c750e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinybox-0.2.4-windows-2022-x86_64.zip",
            sha256: "8be3c207bacd75a47706e61af5367e7046b6f9d1d1bc486438f7245915d6a970",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinybox-0.2.4-windows-2025-x86_64.zip",
            sha256: "2dee76ec3fdefe95c59492f8bc95e324aae46c53147461ad1339beb1e9084186",
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
    version: "0.2.0",
    release_url: "https://github.com/tinyhumansai/tinychannels/releases/tag/v0.2.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinychannels-module-0.2.0-archlinux-arm64.tar.gz",
            sha256: "07d2285c42dbeca2d70e0eb26890df533b7daab2139a8e71c71d9ec88ce819cf",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinychannels-module-0.2.0-archlinux-x86_64.tar.gz",
            sha256: "8d67e15a9c11f75cc53fab27c838308f5d622895df403f4d8ff5a37b4000411d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinychannels-module-0.2.0-fedora-43-arm64.tar.gz",
            sha256: "82ea9f45978fa82a1ab10846b922f7a31dfd1e0d6d7e13436aec3624dce707d6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinychannels-module-0.2.0-fedora-43-x86_64.tar.gz",
            sha256: "a0ab5045055328e55e39568b77c1fd5dc63ea63a015fb86d0e0fbe905375b855",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinychannels-module-0.2.0-fedora-44-arm64.tar.gz",
            sha256: "0eb80bc0ba0c36694e92a2ea628f73c010b9409952d232eca4261fc9a20cd837",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinychannels-module-0.2.0-fedora-44-x86_64.tar.gz",
            sha256: "a0ee9aa9d8f462ccbe60e6122fb32beb519f6e080d7ca7c19c150bff1a0e83a6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinychannels-module-0.2.0-macos-15-arm64.tar.gz",
            sha256: "846b9391cdb7e5488c6b62c5c25fbd495e1899e3db92517cb0b0be0e32de8567",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinychannels-module-0.2.0-macos-15-x86_64.tar.gz",
            sha256: "88a249b5ea3d5e80eb5bc753ee8d503bf8a2b2293d2c1127ca6a5fd3015dd421",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinychannels-module-0.2.0-macos-26-arm64.tar.gz",
            sha256: "3a666957b9d4cce0ffc23e1ea2a21b6fce90d823276cca018cba04ce957bfd54",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinychannels-module-0.2.0-macos-26-x86_64.tar.gz",
            sha256: "ef0edd5d7990398a4b97ef114e8858d53f7418dbc818c1fad949472a712a2b61",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinychannels-module-0.2.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "8d947a1b283bcb538ac59f68e1fdcce07fe47d9bb59d8ea0406316c213c04b13",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinychannels-module-0.2.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "92c90089eef81d7e90aeb8f2992dea706bcd91fd2cd9485004edea24eac18c0c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinychannels-module-0.2.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "cd7c1522e32d0cb125fe643feea5f3602663c99a8ae96f201fc5077a2496d588",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinychannels-module-0.2.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "b4556e866c309b1792a73af48bf7297c4a2714bae25c9ef1adc22c403f25dc04",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinychannels-module-0.2.0-windows-11-arm64.zip",
            sha256: "3e29456cd6b161a605c9224ebaa51dbdaa3f0a4b944529cfc3a8e471bc0d5725",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinychannels-module-0.2.0-windows-2022-x86_64.zip",
            sha256: "456d7c6cd16494aff56adfa2b4b96db47abd284203e11de10fc0e9efb6f76eda",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinychannels-module-0.2.0-windows-2025-x86_64.zip",
            sha256: "427bf8719b9cd2ac874f6cc9f74a5dc49fb08c8758b62cf8a09616448cb25134",
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
