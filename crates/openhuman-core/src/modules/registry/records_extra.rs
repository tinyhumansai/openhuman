//! Registry records for additional first-party TinyBus modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

/// The `tinybox` module, loaded on demand.
#[cfg(feature = "modules")]
pub(crate) const TINYBOX: ModuleRecord = ModuleRecord {
    id: "tinybox",
    description: "Sandbox execution and lifecycle operations through TinyBox",
    bus_name: tinybox_bus::INTERFACE,
    object_path: tinybox_bus::OBJECT_PATH,
    version: "0.2.6",
    release_url: "https://github.com/tinyhumansai/tinybox/releases/tag/v0.2.6",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinybox-0.2.6-archlinux-arm64.tar.gz",
            sha256: "284cda47d58de498a02f9643243746deb858d8f71c8f54ccac7275f0965e3be1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinybox-0.2.6-archlinux-x86_64.tar.gz",
            sha256: "55f4ecda62225df11f6e546c13851c9a1a78ed3037f99394b955816f79ffce6b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinybox-0.2.6-fedora-43-arm64.tar.gz",
            sha256: "2e4d134f8dbdef9b41fba43036e5803babebb3d0f24bdfff341c8f29d25ae0d8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinybox-0.2.6-fedora-43-x86_64.tar.gz",
            sha256: "57a259f7297f5a7e20e9f9b8cb47c57d70a4edbaf6f6ea0d4c33f5f6321a66dc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinybox-0.2.6-fedora-44-arm64.tar.gz",
            sha256: "3ab57d75c478091384e23019e8fe7807ffddaaeb436b52b8d9604ac0336d9ceb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinybox-0.2.6-fedora-44-x86_64.tar.gz",
            sha256: "6b9f65d8501ff0a32ee60f13c22e6501696d9722fd2951f73fbe5a796bb21eb9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinybox-0.2.6-macos-15-arm64.tar.gz",
            sha256: "8d62d2e86c41a2220ca6c4d989fb116fb5e09664b6ead2b0cdccb984db50b068",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinybox-0.2.6-macos-15-x86_64.tar.gz",
            sha256: "8e4f2ebde00a6f8a06445b310df7417de5877c8bdb7d6fbe7ae5653d003f7afa",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinybox-0.2.6-macos-26-arm64.tar.gz",
            sha256: "a9c14971de977c02202197a57329de9c73275e09b13cdaa4b2726472b711f718",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinybox-0.2.6-macos-26-x86_64.tar.gz",
            sha256: "c63f858405bdbe38f6635b77cb79cc45b11fbe186e69ca03b5d88cb2a7df7696",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinybox-0.2.6-ubuntu-22.04-arm64.tar.gz",
            sha256: "a8b79919554b603022b1acf028cc75594b999569b242f12630ec0f1c13ce7a8b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinybox-0.2.6-ubuntu-22.04-x86_64.tar.gz",
            sha256: "9df93f7112ce5f05285446c75891d886dfe9228c7f2b42620f8d9ec68ec9e0a1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinybox-0.2.6-ubuntu-24.04-arm64.tar.gz",
            sha256: "422bec85c4e45953fc830aab4d7f9774e91df46c533b53028db6eb51b7a982f8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinybox-0.2.6-ubuntu-24.04-x86_64.tar.gz",
            sha256: "576c03733c70bba869647ab1fc137b6b941aee977c322494df0e72e4194705ee",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinybox-0.2.6-windows-11-arm64.zip",
            sha256: "7400205c377b2803ba89a8be5e78cb06307da113d452582be8cfe04afd7c877c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinybox-0.2.6-windows-2022-x86_64.zip",
            sha256: "4cad9cad7f79502757583d46134ee25bf80d0d9745d889f91e3e68b98bc1cd0a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinybox-0.2.6-windows-2025-x86_64.zip",
            sha256: "a21e2d12a1ad844abad51f584c9ffb6fa5a8bb9c5900f8713a4f89e5bd70dfaf",
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
    version: "0.2.1",
    release_url: "https://github.com/tinyhumansai/tinychannels/releases/tag/v0.2.1",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinychannels-module-0.2.1-archlinux-arm64.tar.gz",
            sha256: "2680ef5f51775adcf4d92c4cb228ed7518f8d0ec47fc93830c239569926cf98b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinychannels-module-0.2.1-archlinux-x86_64.tar.gz",
            sha256: "e8c5d4b98f1177a589f52b88bb0087c77370db37eab8387a177a86a3dab7a577",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinychannels-module-0.2.1-fedora-43-arm64.tar.gz",
            sha256: "31f1e49213391a5ef3c30d195dc18230305cd7d1096afc1a26d50b1cb1094655",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinychannels-module-0.2.1-fedora-43-x86_64.tar.gz",
            sha256: "d5c8093c19347f80f371dd192b634d6d1c381098e269a8469a1fb429ef8ed112",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinychannels-module-0.2.1-fedora-44-arm64.tar.gz",
            sha256: "c09fc5658b3db0d61a41592a0af985e01256ad1e57f4698956cd1aafbc50dbb8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinychannels-module-0.2.1-fedora-44-x86_64.tar.gz",
            sha256: "224047a34161dea3918918623a907214ba37243ecd86470a9edd89974f335481",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinychannels-module-0.2.1-macos-15-arm64.tar.gz",
            sha256: "a5978525dec30a0179bb25c03dbddb7b188bc383ec609879ef4be2327ce37bfa",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinychannels-module-0.2.1-macos-15-x86_64.tar.gz",
            sha256: "76080dbcd6c0ff4a0c2eb591f23087bb238520658a19372ff1b5d328e15138e4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinychannels-module-0.2.1-macos-26-arm64.tar.gz",
            sha256: "06722cfb0686534cbdbcc7a07f5613d8e4f692b7df6a4087fffd34669f8735c8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinychannels-module-0.2.1-macos-26-x86_64.tar.gz",
            sha256: "450e65b3c2db1ad7a95bd2e9a97a90412d35808a55ca882ce2015e8683269704",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinychannels-module-0.2.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "aa6fccae1e6762363c205c66be620fee096afabef0adb8652e0efe50b07d40ce",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinychannels-module-0.2.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "30ef63ef849ee8c7f959d1711a8f875629feb1643169ba4a1c5a6e75279ceebe",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinychannels-module-0.2.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "d12dca9a9eee6bd487d45f845e9ee449f035049e8d47da1be26a2b2d5d282ad8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinychannels-module-0.2.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "c22ab9d0342a7f24e37952665b8fd9ba7e51dbb11397c75f4e242bc228135a29",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinychannels-module-0.2.1-windows-11-arm64.zip",
            sha256: "ce07269fcda05f1ea4798fbf567cd26081d229c503a940981c48fbcd1d77771f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinychannels-module-0.2.1-windows-2022-x86_64.zip",
            sha256: "03ce46d2e8e6728a531244d5e771ad26d082f490eab571079b051dc0aa323205",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinychannels-module-0.2.1-windows-2025-x86_64.zip",
            sha256: "fe8f9aaa89a3993df7c819a7fcdda4298c7f653116990a5355f0f726ba868d62",
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
