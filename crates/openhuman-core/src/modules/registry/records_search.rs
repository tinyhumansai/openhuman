//! TinySearch web-search module. Published digests are added only from its
//! release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description:
        "Web search, grounded answers and page contents across providers through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.5.1",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.5.1",
    // Verbatim from the published v0.5.0 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysearch-0.5.1-macos-26-arm64.tar.gz",
            sha256: "dc2464f8f6995425a757a0b6c8afdb58a49ef402dcf1b9c4448652ffb1fcd414",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysearch-0.5.1-macos-26-x86_64.tar.gz",
            sha256: "811cf06a8ad560231fbdb5981e357d003a35e6fbc757bf38fc2c73c6b7f11925",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysearch-0.5.1-macos-15-arm64.tar.gz",
            sha256: "09b542f2ced3cb4dff7ee9e59ec8eda53a669136269f8c43fe8ca3e6a4120997",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysearch-0.5.1-macos-15-x86_64.tar.gz",
            sha256: "e67875550b5390b10f4834bf2ed061904961221d18a803d362624fa9fa4eae7d",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysearch-0.5.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "f078d63ada0c5051a830d803079051d9dee4c7c08c9b6e61100692ed2c63c887",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysearch-0.5.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "47cb89a8d704e6c097300c97da544d28757bdf3ff3022c995e9f8f47979140a0",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysearch-0.5.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "494bfdf799abe9200e6f6da1bbd6b26496f727e0d4fa5e5b135396681540de35",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysearch-0.5.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "e7ea0d94bdb41952ff31d29fb8353930c79183c3ecd746f15e516c0902f89f49",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysearch-0.5.1-windows-2025-x86_64.zip",
            sha256: "59cda410efa9ea0c38eb7fa273485c0d66e4e59fef56f954819c913913d114b3",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysearch-0.5.1-windows-2022-x86_64.zip",
            sha256: "ddca87348d4cc3d0aa26c90c9270d45a32dc0e92b0499dac9cdf8bcc213888be",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysearch-0.5.1-windows-11-arm64.zip",
            sha256: "408b6ad81212e2bbe3873543002d6dbadf4647cc14a29f1300078fc3e6170b51",
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinysearch",
        manifest: "crates/tinysearch/Cargo.toml",
        package: "tinysearch",
        lib_name: "tinysearch",
        features: &[],
    },
};
