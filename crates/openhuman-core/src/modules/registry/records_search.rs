//! TinySearch web-search module. Published digests are added only from its
//! release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description:
        "Web search, grounded answers and page contents across providers through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.5.5",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.5.5",
    // Verbatim from the published v0.5.4 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinysearch-0.5.5-archlinux-arm64.tar.gz",
            sha256: "f4209ef6ac68bbf4f99f4ef292ca834c3f5b42372e60f2c8c759fafb825ffca8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinysearch-0.5.5-archlinux-x86_64.tar.gz",
            sha256: "20afa85169f18c4d24c8582f2d6df325b93429d0a861758428629831e6166898",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinysearch-0.5.5-fedora-43-arm64.tar.gz",
            sha256: "fda8f4d7fc082956cf438a26510ae090c84aef1ac009b95038c728e03d744043",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinysearch-0.5.5-fedora-43-x86_64.tar.gz",
            sha256: "c06b3bf3e8802260d482902657fdc3845719957111cb78383b166c8b15a56aee",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinysearch-0.5.5-fedora-44-arm64.tar.gz",
            sha256: "306c44a1631c0a0356459a01ea2cf81a3098dca2644b6b5a07025e1a39d742ac",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinysearch-0.5.5-fedora-44-x86_64.tar.gz",
            sha256: "df294e5eadbf8efc80ea3bffcfc9404d6b5272df74c1b23d39138ed94b2a606b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysearch-0.5.5-macos-15-arm64.tar.gz",
            sha256: "413a0e9e9ef74f49fa03255bf07ca385a3a24baf7ea2e9615e4ff49d605591ac",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysearch-0.5.5-macos-15-x86_64.tar.gz",
            sha256: "4a145266bd1637f8b1498f9a7fcbfe910a327f6f2c684950f16c9046ae5f0de3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysearch-0.5.5-macos-26-arm64.tar.gz",
            sha256: "f185771e44b14f8f81f65d9d02b10d5df732d4440fa2a9310cad874ac4f0aa49",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysearch-0.5.5-macos-26-x86_64.tar.gz",
            sha256: "c7b1914099ab3ac123b856957f892192ee533adf70d42e0ae2d46aaa200e73c3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysearch-0.5.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "82fa5d85273d28d62c1c01c86a259c004783def932d57e7fa8a3fc4f35a6dd9c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysearch-0.5.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "4b7f0b0938bff89884b310bc514a2cf7d691f4d1dc528a7b67333894732bec4e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysearch-0.5.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "8e11743212d8b8ecf43c7215d4c621085942477db26d3ec05e0e9383de65a52a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysearch-0.5.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "ae8cc46c8222af1208d16b40c24c32e9da69cb90d6bfb82e4f42f522464e3dee",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysearch-0.5.5-windows-11-arm64.zip",
            sha256: "be3fb673735eb944843564cdbc1e848feefb6d487e800fa87bbbb7ac2a042f27",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysearch-0.5.5-windows-2022-x86_64.zip",
            sha256: "2ee7cc3bcdd302548fdb13559ef335c67318674db45f8dad8682621202efe78b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysearch-0.5.5-windows-2025-x86_64.zip",
            sha256: "f30e7f6091867c555efd6b3a140801cfad915ed1a6c7b1f689fedb8843962090",
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
