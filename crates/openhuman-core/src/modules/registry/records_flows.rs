//! TinyFlows module: scheduled job runtime and workflow authoring through its bus contract.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYFLOWS: ModuleRecord = ModuleRecord {
    id: "tinyflows",
    description: "Scheduled jobs, workflow Catalog/Runs execution, and authoring through TinyFlows",
    bus_name: tinyflows_bus::BUS_NAME,
    object_path: tinyflows_bus::OBJECT_PATH,
    version: "0.9.5",
    release_url: "https://github.com/tinyhumansai/tinyflows/releases/tag/v0.9.5",
    // Verbatim from the published v0.9.5 checksum.toml.
    assets: &[
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyflows-module-0.9.5-macos-26-arm64.tar.gz",
            sha256: "c6e31c3c8654c28b8d0b6dfe91ca1971a00f19dae14bf9903242e769225ef9d1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyflows-module-0.9.5-macos-26-x86_64.tar.gz",
            sha256: "6531b27fddc55f8febee2113086b80ad12227aad4beeb48e0f47143da24271ed",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyflows-module-0.9.5-macos-15-arm64.tar.gz",
            sha256: "2b98c6b44ed84a430d071592805f05891b1475653df4dacd96ff3f524a29a99f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyflows-module-0.9.5-macos-15-x86_64.tar.gz",
            sha256: "d5fcafcaa1885276921d26a0e65fa0d60991ea7964a7007641de21b555dfb343",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyflows-module-0.9.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "f7c5e7438c64ce76c0249f81a75669b25f430250abfeff1d6831a782a2539a8b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyflows-module-0.9.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "55779f03a89cd3c5d76bd5bb26e6d9837b2f5e3153c83f3aa11656222a4f4582",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyflows-module-0.9.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "e6f96edfbab60421e3dc6db5d51fb49fb0da36d049cc359a5db5091b3b9861b6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyflows-module-0.9.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "f09c8f4104cd17d00c5eb5f577c8048ba7b76a985e894146df5cc6b552b3e97a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyflows-module-0.9.5-windows-2025-x86_64.zip",
            sha256: "6221186d10e28a56ab8d5881e8d525f0db70cc4ccb0d5681f5893b11c7950920",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyflows-module-0.9.5-windows-2022-x86_64.zip",
            sha256: "b8eec6404312c71286650132a41d33758af2efa49c611a131befd383b1c77aca",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyflows-module-0.9.5-windows-11-arm64.zip",
            sha256: "67d4b8c8ad87c4a7c416ddaff4c96c79bde0fc2e43624306b219e85c8c332587",
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
