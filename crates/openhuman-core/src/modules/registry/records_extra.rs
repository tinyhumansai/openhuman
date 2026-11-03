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
    version: "0.4.6",
    release_url: "https://github.com/tinyhumansai/tinyhosts/releases/tag/v0.4.6",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyhosts-0.4.6-archlinux-arm64.tar.gz",
            sha256: "86c400ebb50bfb44ba78db2f30fbab1593c2ac27d9a7f18a8fb297d9ab177573",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyhosts-0.4.6-archlinux-x86_64.tar.gz",
            sha256: "c79a0ff4c89b1a07abe962c89cc960731c8d03dcdc3c224d3144b960f746c4d6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyhosts-0.4.6-fedora-43-arm64.tar.gz",
            sha256: "34731376697380deb682a3b0e4eb112e6154c795a60374edba7602c141db257c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyhosts-0.4.6-fedora-43-x86_64.tar.gz",
            sha256: "eea28e3045f8a92ee4e91c90d60963a0517cc0cc6c428e20b06f33d77a844b84",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyhosts-0.4.6-fedora-44-arm64.tar.gz",
            sha256: "0c9a40f3ac4012a48d93a9b99afd5eda9d6810a4b9917459a4cb5b657dac9206",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyhosts-0.4.6-fedora-44-x86_64.tar.gz",
            sha256: "6ba31f17adbc07ec1fbf2bfd3cab25a6b8df9dcf844112206226328a2ea82400",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyhosts-0.4.6-macos-15-arm64.tar.gz",
            sha256: "d8879188ed15cb8ca2a94116a965c622f6e21eae092a9fa98e06e374f8188598",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyhosts-0.4.6-macos-15-x86_64.tar.gz",
            sha256: "ba134f10d3d9bc7cd60ddabea59caa1457ec91f0dfa916478f2417300cd1e94c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyhosts-0.4.6-macos-26-arm64.tar.gz",
            sha256: "554e61e29d3db27e4946118e193eb74df41194c360e7416b790cf77fa3d9d4e2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyhosts-0.4.6-macos-26-x86_64.tar.gz",
            sha256: "fe88e08e691341a180070b36e7612ca5629c479e7e8daf467d21afdb69870359",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyhosts-0.4.6-ubuntu-22.04-arm64.tar.gz",
            sha256: "c1bd520099bcf0989673e5a2bd7e1db54d67aa4c523e5ca5f365c752c728ef20",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyhosts-0.4.6-ubuntu-22.04-x86_64.tar.gz",
            sha256: "02d4b29debb5be65026ff9d57703a363d7ddb8c7bb0edfffbd9addd14b1bdaba",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyhosts-0.4.6-ubuntu-24.04-arm64.tar.gz",
            sha256: "c3efc57e25e6d1f736884b784291e87103fb32beb535dccaa42e295f7e892566",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyhosts-0.4.6-ubuntu-24.04-x86_64.tar.gz",
            sha256: "e41702d214a71ece7e39474389df50d7ca652fc0471603b78b4d4f0c210afc7b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyhosts-0.4.6-windows-11-arm64.zip",
            sha256: "cbf8439ed1c38a1eee8ac721409914e2b6623a0238ec57f6b1477980f13ae723",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyhosts-0.4.6-windows-2022-x86_64.zip",
            sha256: "3ac46c30370516b4389f1f17580e12843293f43529094d7818bd90d4f6907d2f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyhosts-0.4.6-windows-2025-x86_64.zip",
            sha256: "f606cd368aa7ae0d6744f5089e6058514f9cb19a706a516c6115b1950c9f4523",
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
