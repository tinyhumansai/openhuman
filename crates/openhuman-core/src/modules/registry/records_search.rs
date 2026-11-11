//! TinySearch web-search module. Published digests are added only from its
//! release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description:
        "Web search, grounded answers and page contents across providers through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.5.7",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.5.7",
    // Verbatim from the published v0.5.6 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinysearch-0.5.7-archlinux-arm64.tar.gz",
            sha256: "89bc202c0aed9da5614027072b25d100316500ad47d4ea4c7855bec0a3573e97",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinysearch-0.5.7-archlinux-x86_64.tar.gz",
            sha256: "b08cdaa026de879f2f5b475c876ba7aa97452d0e21e5afff81e908d924997f04",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinysearch-0.5.7-fedora-43-arm64.tar.gz",
            sha256: "40a01b7894c5624a6684efdd3bedeebbe069c3e1e7d4486da8e7745ed441ab15",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinysearch-0.5.7-fedora-43-x86_64.tar.gz",
            sha256: "269d1a0dc715a71d840605bf8b5d6346d84358dbaed7fc23eed0bd76cb6010ad",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinysearch-0.5.7-fedora-44-arm64.tar.gz",
            sha256: "f919014992a530b01b195f02f7658c93316a900c501928edd3df83b350dc1ea2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinysearch-0.5.7-fedora-44-x86_64.tar.gz",
            sha256: "006c0afce4199f378b1ac0d19f6be81a936d738719a055e5f6c0da5bcfb100bf",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysearch-0.5.7-macos-15-arm64.tar.gz",
            sha256: "6fb4f3771451017e19d71c1890a53a7a7ffc30eec5561dda34f0cb0f0283e3ff",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysearch-0.5.7-macos-15-x86_64.tar.gz",
            sha256: "a77aba9bbdcf6538dbfcbc52be2d25a9b85444c3e71b8ebb02077029b92eeb8a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysearch-0.5.7-macos-26-arm64.tar.gz",
            sha256: "0a232532ce3492c12a7ca4632a30f0e3b0612a319ad42e7dd1b5fa574d918f9c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysearch-0.5.7-macos-26-x86_64.tar.gz",
            sha256: "8cddfa5dea4d1bdfc8c3279e995869cae00d9be93552c05f0e9cf6e4b65602a5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysearch-0.5.7-ubuntu-22.04-arm64.tar.gz",
            sha256: "96d8b9024ab13f6b36bfcbc7875f5084ee0b724f6d433b8c2cedaba6464e961e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysearch-0.5.7-ubuntu-22.04-x86_64.tar.gz",
            sha256: "28aeeeb66376ae1efa6260b25a3c6f8e29f60744c1dfcaf3858dcdf60b1e8619",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysearch-0.5.7-ubuntu-24.04-arm64.tar.gz",
            sha256: "364df629c689a7da4a5c4e9f553df947032f129bb8a6879fdf0f2906f2fef4c8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysearch-0.5.7-ubuntu-24.04-x86_64.tar.gz",
            sha256: "80b3eff3095160b126f44949c500818a1765ea41512c2ca1ea9c3b361875ecb8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysearch-0.5.7-windows-11-arm64.zip",
            sha256: "1d406d05a5d7631614cf8de4251cc933f332aaf84ffec71fbeb5067bbede7a00",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysearch-0.5.7-windows-2022-x86_64.zip",
            sha256: "0e4e3cf49ebf1c71f1be1b78d8390c937d5c2e2e10e008516006fe5b8dd1a9e0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysearch-0.5.7-windows-2025-x86_64.zip",
            sha256: "eb40f64fc0c54f5699b4fd400457b93d5e766e73a553d13b97c2d4570cff6ce6",
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
