//! TinyFlows module: scheduled job runtime and workflow authoring through its bus contract.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYFLOWS: ModuleRecord = ModuleRecord {
    id: "tinyflows",
    description: "Scheduled jobs, workflow Catalog/Runs execution, and authoring through TinyFlows",
    bus_name: tinyflows_bus::BUS_NAME,
    object_path: tinyflows_bus::OBJECT_PATH,
    version: "0.9.4",
    release_url: "https://github.com/tinyhumansai/tinyflows/releases/tag/v0.9.4",
    // Verbatim from the published v0.9.4 checksum.toml.
    assets: &[
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyflows-module-0.9.4-macos-26-arm64.tar.gz",
            sha256: "6cd6e4cc9c22504c2c14944d936d04316ca3301bc7c193baca8834161589f476",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyflows-module-0.9.4-macos-26-x86_64.tar.gz",
            sha256: "b22cbcac0c23b407a0d33959e0ed6f7605f04bf2a5703ce883f488d780be4c9a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyflows-module-0.9.4-macos-15-arm64.tar.gz",
            sha256: "39cabbe3da34773039ce8423a724f684f3d06c50dc08edd575ef5b3811664159",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyflows-module-0.9.4-macos-15-x86_64.tar.gz",
            sha256: "8a937a24f37afcb0cb3042dfe640caccbbbb3766f45b9c06f1d34d9c235a5346",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyflows-module-0.9.4-ubuntu-24.04-x86_64.tar.gz",
            sha256: "43a831d16cf0406c6148deda1023f9d3467483b275a65e30b282e8362ebbe01f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyflows-module-0.9.4-ubuntu-24.04-arm64.tar.gz",
            sha256: "7105663eb05118f5fd7d008bc56ec35e24463e36f5762ee7871018cb3219aca6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyflows-module-0.9.4-ubuntu-22.04-x86_64.tar.gz",
            sha256: "da04b73a434b5a11b97bc98518d81f27e96c8115431c2b3bbe75a7e67aae5055",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyflows-module-0.9.4-ubuntu-22.04-arm64.tar.gz",
            sha256: "eba07906b8ae276c8fee8c303e14a40e4a77797d89ffcc6c5046d63a3fda8ef2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyflows-module-0.9.4-windows-2025-x86_64.zip",
            sha256: "d2c0c1657c68e9dd89964bbc1a815afee3fa83e72fcdc342e5334814c2d45a3f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyflows-module-0.9.4-windows-2022-x86_64.zip",
            sha256: "53adbe5dab7ee8fa683026624a777b466d922212bd3118311e0a7d99b027d595",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyflows-module-0.9.4-windows-11-arm64.zip",
            sha256: "087ddd9bda9b405620712fd56dcfc0289ebb1c9f96ef068ba838a3f96848282e",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyflows",
        manifest: "crates/tinyflows-module/Cargo.toml",
        package: "tinyflows-module",
        lib_name: "tinyflows_module",
        features: &[],
    },
};
