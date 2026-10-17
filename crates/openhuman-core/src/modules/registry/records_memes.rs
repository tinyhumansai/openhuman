//! Registry record for the `tinymemes` module.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

/// The `tinymemes` module: final-reply slang and meme remixing, loaded on
/// first use. The host serves the callbacks it asks for (inference, Jev,
/// search, index storage, delivery).
pub(crate) const TINYMEMES: ModuleRecord = ModuleRecord {
    id: "tinymemes",
    description: "Final-reply slang and meme remixing through TinyMemes",
    bus_name: tinymemes_bus::STATE_INTERFACE,
    object_path: tinymemes_bus::STATE_PATH,
    version: "0.4.0",
    release_url: "https://github.com/tinyhumansai/tinymemes/releases/tag/v0.4.0",
    assets: &[
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymemes-module-0.4.0-macos-15-arm64.tar.gz",
            sha256: "2ca122a4c95d18d6d3a64f082c60109569ea0750b8592023da31822b0f38a078",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymemes-module-0.4.0-macos-15-x86_64.tar.gz",
            sha256: "d94d8c17a8ca06d6a8032c18646058fff3fc743392ad605c9a305580a6b080f7",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymemes-module-0.4.0-macos-26-arm64.tar.gz",
            sha256: "9c50a75bfc05a1d95f1823f7e8370f993d94fa35bca9cde7a8b81f8aa525e9d6",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymemes-module-0.4.0-macos-26-x86_64.tar.gz",
            sha256: "24b3fa4efb8333a9fb2580c998c02d87cbb9e68f155364c4a8ff6238c84e1a19",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymemes-module-0.4.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "a2ef81642fdf916e7a58702febea091330217e7107ab6e0c5645bc6df3597efe",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymemes-module-0.4.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "9f6daa13b3661b2e37bb5e5a4f318fa2ef3a6de92895c4844be1bdff30ea1761",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymemes-module-0.4.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "f15ccee705a0da3321bee73b4d6bb6663a55d3ff9a7ed19c97e223bdee2472aa",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymemes-module-0.4.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "0e2beffa0f8f575dec530c8fc67c27a8f8b8a8d66079f6057c530c45911c3a59",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymemes-module-0.4.0-windows-11-arm64.zip",
            sha256: "7498c8d7ade28f27da37083937676a0f44b1cc037ff389e98209a99b7232d7d4",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymemes-module-0.4.0-windows-2022-x86_64.zip",
            sha256: "0ba16d4897558bf7da6f138b08252e109f212bb6ecb5de8bf1976142d7c4baf9",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymemes-module-0.4.0-windows-2025-x86_64.zip",
            sha256: "8d9330f349451fc72035fb716fd800e27c046e7df2124dd46a59690d62655478",
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinymemes",
        manifest: "crates/tinymemes-module/Cargo.toml",
        package: "tinymemes-module",
        lib_name: "tinymemes_module",
        features: &[],
    },
};
