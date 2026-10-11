//! TinySearch web-search module. Published digests are added only from its
//! release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description:
        "Web search, grounded answers and page contents across providers through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.4.2",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.4.2",
    // Verbatim from the published v0.4.2 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysearch-0.4.2-macos-26-arm64.tar.gz",
            sha256: "c89f54a958a95d802c9b608a5d2b9469dd63819f03274bd0ea484212f88cca2a",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysearch-0.4.2-macos-26-x86_64.tar.gz",
            sha256: "6af3f7adc28a4ec99344c57019b6243f7c597a130862956e98170eeac18304e1",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysearch-0.4.2-macos-15-arm64.tar.gz",
            sha256: "5f0f56e3d1a202ee4e81ae8d6a83479fb0635d2ed231f91dab005d00cd05c6bc",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysearch-0.4.2-macos-15-x86_64.tar.gz",
            sha256: "c303bd5832945ff456412eb66bd4159da312de982c0c4a5b2fb77b796fcc99c0",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysearch-0.4.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "a799fe7f9dabddfcf7421f462b8517588242f69f6531938cd94a7ff75076d711",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysearch-0.4.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "1f75b38b0d7811e6d3610a0ff430b55614d3d43dbdfd44127086953593e7f01d",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysearch-0.4.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "a07d6bdace72fdb57879b7383b9d1a56ba397ddfce45d0ea4152a90ed9f79c0b",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysearch-0.4.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "a5675addd35aaab67316bbd01ee056476dcb5de281ac99428eb4d500362c664a",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysearch-0.4.2-windows-2025-x86_64.zip",
            sha256: "5f10f8dd32b3bf420ab7aa627d9a4400503a3168cf84b52556e08c0a4e90b368",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysearch-0.4.2-windows-2022-x86_64.zip",
            sha256: "e997d23c1bc9152a179ba51630a04053597c4db9e0fad4f24658dde6cffab128",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysearch-0.4.2-windows-11-arm64.zip",
            sha256: "61a15ad5a952b2acb80e95fe6e0775308ecaa28148c432ed48b989212339740f",
        },
    ],
    load: LoadPolicy::Lazy,
};
