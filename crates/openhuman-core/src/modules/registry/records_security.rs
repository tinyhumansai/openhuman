//! Published TinySecurity admission pins copied verbatim from v0.17.1/checksum.toml.
//!
//! `LoadPolicy::Eager` is the "load at boot" policy (the issue calls it Boot): a missing
//! security module changes behaviour, so it must be serving before the first request.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSECURITY: ModuleRecord = ModuleRecord {
    id: "tinysecurity",
    description: "Native security policy and durable approval tool grants",
    bus_name: tinysecurity_bus::names::INTERFACE,
    object_path: tinysecurity_bus::names::OBJECT_PATH,
    version: "0.17.1",
    release_url: "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.17.1",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.17.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "88468122ad2bac2cf6e1e10b2986d862ca37b939d6a79d093812c95487882cd7",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.17.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "6d5c0cea0bdcc30e1f4e1067ac4235f58a9036e1030a4994c0f3f80622825d0d",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.17.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c85e6eba3578d9374961730452d297be7ce31472bf751109ef446989055d9659",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.17.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "34940805fd3e4aedd061913734f32b08dda5d5cefd208da3e2af223416559f09",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.17.1-macos-26-arm64.tar.gz",
            sha256: "51930008089866f640e9696c1d72b9cfedb827882cd479e99bc89e8458750617",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.17.1-macos-26-x86_64.tar.gz",
            sha256: "ecfa7df94429119505d46cf6002bdd906b9310339e7783173d4c4737d87e510a",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.17.1-macos-15-arm64.tar.gz",
            sha256: "3ba81ee8a01c5bb573385b6f25b2ee87cc59512186d9aed65262751245f2d453",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.17.1-macos-15-x86_64.tar.gz",
            sha256: "e78a4478da417c26960729ce88819c56e1e45f0978374f5f7c6676ac9fea9605",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysecurity-module-0.17.1-windows-2025-x86_64.zip",
            sha256: "b9dead99d92b6746ab0feb1076670ff48e06b81f690801bd0706f26204d4570e",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.17.1-windows-2022-x86_64.zip",
            sha256: "d359f67dc49477866211775f13e695785123acdc34753f43a72001c7ec489c36",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.17.1-windows-11-arm64.zip",
            sha256: "c53acf93a5a15f875e62e0ebe9043da6efeaba6d7378426142619239b3040c9e",
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
