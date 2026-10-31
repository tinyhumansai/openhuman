//! TinySearch web-search module. Published digests are added only from its
//! release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description:
        "Web search, grounded answers and page contents across providers through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.5.4",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.5.4",
    // Verbatim from the published v0.5.4 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinysearch-0.5.4-archlinux-arm64.tar.gz",
            sha256: "8b21421e632d48b52628db0c964c0cdba79fd1a9d12eefe4d3a756fd38bba1a7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinysearch-0.5.4-archlinux-x86_64.tar.gz",
            sha256: "cd5a7eab472f5819ab8adcd755cb64db5826ee80d8c74119d6f3276e445919c4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinysearch-0.5.4-fedora-43-arm64.tar.gz",
            sha256: "f92740ec8ec2be508499a73d64ced29cc3512972746eff58cbbed24d95f47f4b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinysearch-0.5.4-fedora-43-x86_64.tar.gz",
            sha256: "c72c0a0005c7a8c6a8abc8beab61b93f7841998b565a1b940b1d483a7f0533e7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinysearch-0.5.4-fedora-44-arm64.tar.gz",
            sha256: "a62933df3368d1a8e9a89c66dddca5e9f327ca3378a3d30c810d1aa405674f0b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinysearch-0.5.4-fedora-44-x86_64.tar.gz",
            sha256: "5b336d726013e8b730ff9ff386edad13ef43b458a6a2b85d672ee1563a4fee8a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysearch-0.5.4-macos-15-arm64.tar.gz",
            sha256: "73fac684e332f455d00260fa0201c670117e35d72c5fa962842573bf004fc044",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysearch-0.5.4-macos-15-x86_64.tar.gz",
            sha256: "7db059ed577e03f1f46bbefdf045a29b49c6c658687aa82361990000b700d8af",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysearch-0.5.4-macos-26-arm64.tar.gz",
            sha256: "ffb42d860e2374d2eda565af262133573d95bf9b60ad21bbcefb7dea56d0bffc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysearch-0.5.4-macos-26-x86_64.tar.gz",
            sha256: "e131b55bd5952ae5e5929f2d7a4f2c05b39d71cabefd15f951473e587e9164a5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysearch-0.5.4-ubuntu-22.04-arm64.tar.gz",
            sha256: "38dba8fcc45b0d75eebc355af714bc742ec6b072416aa073a77a15ca5db60065",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysearch-0.5.4-ubuntu-22.04-x86_64.tar.gz",
            sha256: "879053a08393906de0dcfb19de65e9e5ae879dde81c11a7189ccdc899d4f38af",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysearch-0.5.4-ubuntu-24.04-arm64.tar.gz",
            sha256: "952616aeceafaccf122ed643076a97ecb6fca123a3c2498470c75bef62ad9024",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysearch-0.5.4-ubuntu-24.04-x86_64.tar.gz",
            sha256: "44fc5fe514416063542253f3f3dd3ff5c68042eaba29de5777a2bf0a4b33f100",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysearch-0.5.4-windows-11-arm64.zip",
            sha256: "c006c243c4c5fbf1b6d6d2b12ed2e9f13244e6599c6f0af4306f8cf1d2e656bb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysearch-0.5.4-windows-2022-x86_64.zip",
            sha256: "0a5bd999e7d65caee1caa106a7c68958b34a9bdf728c5c853978c79a5880abf2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysearch-0.5.4-windows-2025-x86_64.zip",
            sha256: "fa714eefe9ed83949dcb5216f9abddec7ec71e149e1c84a2c5818fbe21b071f2",
            library_sha256: None,
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
