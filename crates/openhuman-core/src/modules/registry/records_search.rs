//! TinySearch web-search module. Published digests are added only from its
//! release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description:
        "Web search, grounded answers and page contents across providers through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.5.3",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.5.3",
    // Verbatim from the published v0.5.0 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinysearch-0.5.3-archlinux-arm64.tar.gz",
            sha256: "6eb3cd19b8f68f7b8e1edc1f0857168cf0b9a81890066e04af35d4dd5f68b244",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinysearch-0.5.3-archlinux-x86_64.tar.gz",
            sha256: "d4edd917a33167125c2447e9b19c8651505faa35574fde60845fe365cc54012d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinysearch-0.5.3-fedora-43-arm64.tar.gz",
            sha256: "73540bbecd0ad3c3e73d6b5e94ad8f3b065788c70cef1a98b6c6075009cdace3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinysearch-0.5.3-fedora-43-x86_64.tar.gz",
            sha256: "9d471e89b094a27018dc683f07aba1a4fbf742012ffadfb0b5c0d4838d8fd782",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinysearch-0.5.3-fedora-44-arm64.tar.gz",
            sha256: "117bc4ac083825bac7e9949399f4fbaa68c2627b4cd842566440f6736ef98b54",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinysearch-0.5.3-fedora-44-x86_64.tar.gz",
            sha256: "b0c1185f547b336f2c4b303f9bd066c3c3f59c4b33dee5d4a7289275ec2d758c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysearch-0.5.3-macos-15-arm64.tar.gz",
            sha256: "aa9e45ba9ddf153a94c095e1f00eee1916db56eadfd51717d81eb4b24d6d9a71",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysearch-0.5.3-macos-15-x86_64.tar.gz",
            sha256: "1f813a23ec398ea3ce5facbe41298ff62c4f77f5c4bac7789b7bff3ab1ef91b3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysearch-0.5.3-macos-26-arm64.tar.gz",
            sha256: "91e160247223623eb075cc593cf580d01766866c30e385fb0957b32b747d6dd3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysearch-0.5.3-macos-26-x86_64.tar.gz",
            sha256: "f53f7a008f8d016d0e1f0a1e11d23e74d53f5524049fa4516aa51a3c154b08ed",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysearch-0.5.3-ubuntu-22.04-arm64.tar.gz",
            sha256: "8b5d0ecf1b77a1882ddb5786bcae1d306557b64b4df06573ec06d63bb9407ab6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysearch-0.5.3-ubuntu-22.04-x86_64.tar.gz",
            sha256: "6c787c80d2312cba304dc3ede0ec50a165daf8428403e643b72cce90d4eb7c76",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysearch-0.5.3-ubuntu-24.04-arm64.tar.gz",
            sha256: "131133c1d91b0e814ec8fc8be73ca1bf159c306afc870c62ea6b59badfe59f8e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysearch-0.5.3-ubuntu-24.04-x86_64.tar.gz",
            sha256: "78b5632d9c207f9b78ee83627bf35ae73172d0fb1fb7ea47273058f4c1f4c0b1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysearch-0.5.3-windows-11-arm64.zip",
            sha256: "5d964256f724ea1d47fc1856f8f14d671b808bd16992f8862bd0a605807b62b7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysearch-0.5.3-windows-2022-x86_64.zip",
            sha256: "545e050f74024a42e4cf665f8c9ade65f68cde23dbeab033ba7f1215e17fc906",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysearch-0.5.3-windows-2025-x86_64.zip",
            sha256: "e39460b6e41c3266f4d563bc8b7a16da074458c161e1a5cedfa34c41117ec12f",
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
