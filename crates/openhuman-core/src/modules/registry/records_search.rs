//! TinySearch web-search module. Published digests are added only from its
//! release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description:
        "Web search, grounded answers and page contents across providers through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.4.1",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.4.1",
    // Verbatim from the published v0.4.1 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysearch-0.4.1-macos-26-arm64.tar.gz",
            sha256: "596e8ccd6ea57026bf2f3d0abcb5f377c3b877fe1a926f3335f58ce917f15289",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysearch-0.4.1-macos-26-x86_64.tar.gz",
            sha256: "cd74ca7b47b05fe05d4dec04828386af8798508bafe62053392a7c9cfac2ebf3",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysearch-0.4.1-macos-15-arm64.tar.gz",
            sha256: "9793e84de25b6369548d223b374589aea3b0da32dddba88ff192ae1b85d1d185",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysearch-0.4.1-macos-15-x86_64.tar.gz",
            sha256: "a2372f25f782ef65dad2dd59b10d64d333e82fb63d38c3e3abdad9004601c74a",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysearch-0.4.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "ffac511cb21f497d6db50aa2145f9c71c97390e93e50f5e81d0a8298c179cb34",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysearch-0.4.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "f89253d43dc970102f432f86d6bc65e81f477175a3f199e89abdee9538b310b4",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysearch-0.4.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "33244ed65d97c4cb13afcf5db149d73fd900ae5e953a8f150237e6891fe80179",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysearch-0.4.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "bf2f6afc2c32c3a67f559a3e46ea2b7848388f6557ab9119dafc33e9c5509dae",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysearch-0.4.1-windows-2025-x86_64.zip",
            sha256: "1733bb9e3772f543dcefbace5b23212fa5f6f4de62eb06cbe0489009031c9953",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysearch-0.4.1-windows-2022-x86_64.zip",
            sha256: "635c6ef997dd30ed614e4f6eca32d2ae5f665be0fd450e303e9ca43d4bd10f84",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysearch-0.4.1-windows-11-arm64.zip",
            sha256: "f6bdbfb8a67dd411d1eeb071f04a9f9f7466fb742103c528a4fe69063c37553d",
        },
    ],
    load: LoadPolicy::Lazy,
};
