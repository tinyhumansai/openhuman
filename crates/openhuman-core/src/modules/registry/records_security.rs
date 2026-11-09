//! Published TinySecurity admission pins copied verbatim from v0.19.0/checksum.toml.
//!
//! `LoadPolicy::Eager` is the "load at boot" policy (the issue calls it Boot): a missing
//! security module changes behaviour, so it must be serving before the first request.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSECURITY: ModuleRecord = ModuleRecord {
    id: "tinysecurity",
    description: "Native security policy and durable approval tool grants",
    bus_name: tinysecurity_bus::names::INTERFACE,
    object_path: tinysecurity_bus::names::OBJECT_PATH,
    version: "0.19.0",
    release_url: "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.19.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.19.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "a52f41593935b71b644753776bca041fded2baf2fd9ad6aa0d8dc8fbb478acf8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.19.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "8e5a9d1e2b4ef0c985647c23f6cf0b9dc3a3258f810348e86fa8e2ee914b29b5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.19.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "4eba772220e02e4f9b3589e78a84c7a73f7000ed2d9ca20bd83e34a2f514afe6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.19.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "d0ebd99f0bd6697d4e4bd4bce205a10c84275798d53dc3074c8c8b5eabc2d70d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.19.0-macos-26-arm64.tar.gz",
            sha256: "4a45348f65346b2c02999be625d8d56a4ef91175155c30fff4ad8e33e8f78f76",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.19.0-macos-26-x86_64.tar.gz",
            sha256: "8beb108fb78bc1bbb9513e976081b2cf30418e64fe7e8646c9dd498c5796775a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.19.0-macos-15-arm64.tar.gz",
            sha256: "2e225d2e3b1b76517182c0f27f07f37f0217a3663c9ede326e1837606fa99b68",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.19.0-macos-15-x86_64.tar.gz",
            sha256: "b3eac95abc3f013eae8853fc24abe50450db143de2b6269bba208ce13ddb0be1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysecurity-module-0.19.0-windows-2025-x86_64.zip",
            sha256: "d2edf257fab30fea9e463b2a4c76dae7358714c8104bfca4658daa4c7adb5021",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.19.0-windows-2022-x86_64.zip",
            sha256: "7bbddf0b10da1abb68ce1d4f37b4ba716365a0633981c2e2a71096d10fba5a78",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.19.0-windows-11-arm64.zip",
            sha256: "d1431618ae6aecd442887cea946c21de5f6ac494eb727e5d2edfb2be8d9873bb",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Eager,
    source: SourceSpec {
        submodule: "tinysecurity",
        manifest: "crates/tinysecurity-module/Cargo.toml",
        package: "tinysecurity-module",
        lib_name: "tinysecurity_module",
        features: &[],
    },
};
