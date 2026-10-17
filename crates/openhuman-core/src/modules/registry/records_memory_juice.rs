//! Registry record for the `tinyjuice` module.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYJUICE: ModuleRecord = ModuleRecord {
    id: "tinyjuice",
    description: "Content-aware tool-output compression and recoverable caching",
    bus_name: tinyjuice_bus::names::BUS_NAME,
    object_path: tinyjuice_bus::names::OBJECT_PATH,
    version: "0.10.5",
    release_url: "https://github.com/tinyhumansai/tinyjuice/releases/tag/v0.10.5",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyjuice-module-0.10.5-archlinux-arm64.tar.gz",
            sha256: "e5ded2dde83b9594b95aa74532b8007818a6a3b489b8cf41498575124d5f3359",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyjuice-module-0.10.5-archlinux-x86_64.tar.gz",
            sha256: "4d910171b3ae3e4ff86fa1af21835361459c116065142c03f61cdcb685600eb9",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyjuice-module-0.10.5-fedora-43-arm64.tar.gz",
            sha256: "cab0ef1415b6d70a5c794c5d03e9e307f868af47c77f50df12366a3616388d40",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyjuice-module-0.10.5-fedora-43-x86_64.tar.gz",
            sha256: "6ddc6e95aa26f1099b54f1bb05ce2fceaa5632eeddef8cefcf62ca5b1389807d",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyjuice-module-0.10.5-fedora-44-arm64.tar.gz",
            sha256: "7e02a173120b620d95572b87e7638db17579ce877cb78ccaa66807d5323596bc",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyjuice-module-0.10.5-fedora-44-x86_64.tar.gz",
            sha256: "750380711a7bdb6c04b008c096596f305cbf3f31ba58139cf22379da2208e094",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyjuice-module-0.10.5-macos-15-arm64.tar.gz",
            sha256: "21cbdb9a558a6c6bec76aaff6356c0f9c12b833b0681c346a79c0b32fa53b923",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyjuice-module-0.10.5-macos-15-x86_64.tar.gz",
            sha256: "b318942cdf5dcca278196388e06c69b43a1453b2ebda6e355914777357c4f866",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyjuice-module-0.10.5-macos-26-arm64.tar.gz",
            sha256: "1ec6070d3bf2d3388858450e04894c76edbecae231e2a30541985c3b44366b2c",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyjuice-module-0.10.5-macos-26-x86_64.tar.gz",
            sha256: "13d7ac454c4763ab52e4421d8effc647e648e55c466a2662d1d1c1028c7f0570",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyjuice-module-0.10.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "47f2dad6ababb112076b2402e63f40b1aa73a4182a3ff14c94e30f6db1762947",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyjuice-module-0.10.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "91084bd47636ebcfe8b8bf8a9389a47312aa06d72b76101752a42f367d783707",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyjuice-module-0.10.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "03629348298bf40ffd78324b2655e7255c0e4cd3e73d108cf581bcd5345964ac",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyjuice-module-0.10.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "ba2485e9d9cf23c7b0cc9e5f0111ae5850b905627ce8cf6ef6fade37dbc293c5",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyjuice-module-0.10.5-windows-11-arm64.zip",
            sha256: "9ba4f1e51b7a14aa2935cf5def8f5aa1ed89f346a991fe41cd441a29c35a814d",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyjuice-module-0.10.5-windows-2022-x86_64.zip",
            sha256: "411adca6d55973c52f3cd49719acd74d925ffe7177e17b295ae5147f7017c20c",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyjuice-module-0.10.5-windows-2025-x86_64.zip",
            sha256: "d2e54f7474315861b48a57c3084e59b9d2ecb09dd5fd4d469520e5ce9b1e74f0",
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyjuice",
        manifest: "crates/tinyjuice-module/Cargo.toml",
        package: "tinyjuice-module",
        lib_name: "tinyjuice_module",
        features: &[],
    },
};
