//! Published TinySecurity admission pins copied verbatim from v0.7.0/checksum.toml.
//!
//! `LoadPolicy::Eager` is the "load at boot" policy (the issue calls it Boot): a missing
//! security module changes behaviour, so it must be serving before the first request.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSECURITY: ModuleRecord = ModuleRecord {
    id: "tinysecurity",
    description: "Native security policy and immutable filesystem authorization scopes",
    bus_name: tinysecurity_bus::names::INTERFACE,
    object_path: tinysecurity_bus::names::OBJECT_PATH,
    version: "0.7.0",
    release_url: "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.7.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.7.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "2e4bb01e3d8ee5a848c989218270199b8318eb8200a6a16a7a6092522d5b4fce",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.7.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "9a77bc13d6a818c0ce54c1ebca1db0b22340be8be19dd8b123a156d9362395c6",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.7.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "ca34b2fa2edeac74e543288f0615ffe32d309b35414ef12e27a69ce69feeb267",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.7.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "cbf79b38aae8bd58794beb9649d8f679b2df347d89684364596bf2696a342329",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.7.0-macos-26-arm64.tar.gz",
            sha256: "c91db3423d8fbc42b2e7e22b362c5f081ea659faf0d17319aa90dd21233cad6e",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.7.0-macos-26-x86_64.tar.gz",
            sha256: "1eee71432be1fef3b6941b5b19daff4a9f4882d61c3cb01856ed5cd9ea65838e",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.7.0-macos-15-arm64.tar.gz",
            sha256: "4d83b29fb73fca32118df3a669a9f60c40e903b508345fac1bb5accc2a2aa0ec",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.7.0-macos-15-x86_64.tar.gz",
            sha256: "53d44b1c1d6cbd33c0d458dfc57417e1929ebb5700e19bee285de5115dc5e0da",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysecurity-module-0.7.0-windows-2025-x86_64.zip",
            sha256: "c99c54d35ec69de72edbf45a3fab13043ae552e8dc793823b470a01685a72850",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.7.0-windows-2022-x86_64.zip",
            sha256: "9dc08702e7a701ab409b62ba79351078a181198481b33ef7f434cb6a5547b707",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.7.0-windows-11-arm64.zip",
            sha256: "bdef2fe6e520ab2b55ea1cd892fbd9b1c5ddca2190327cc2cce7d0c9a31b3f37",
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
