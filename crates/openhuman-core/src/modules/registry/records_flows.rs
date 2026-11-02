//! TinyFlows module: scheduled job runtime and workflow authoring through its bus contract.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYFLOWS: ModuleRecord = ModuleRecord {
    id: "tinyflows",
    description: "Scheduled jobs, workflow Catalog/Runs execution, and authoring through TinyFlows",
    bus_name: tinyflows_bus::BUS_NAME,
    object_path: tinyflows_bus::OBJECT_PATH,
    version: "0.9.0",
    release_url: "https://github.com/tinyhumansai/tinyflows/releases/tag/v0.9.0",
    // Verbatim from the published v0.9.0 checksum.toml.
    assets: &[
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyflows-module-0.9.0-macos-26-arm64.tar.gz",
            sha256: "5b3747231ad717342a55fd51f50e51b7a0d917ea776a5631d361030b6dce53d2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyflows-module-0.9.0-macos-26-x86_64.tar.gz",
            sha256: "8c51d3469a449a5819fee44f359458bc890f3af77e00318d40b6fc529e2e78e6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyflows-module-0.9.0-macos-15-arm64.tar.gz",
            sha256: "13699ad57e71a90245a889a0d3168cd2996d495d45fa8be119806396f26bfe06",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyflows-module-0.9.0-macos-15-x86_64.tar.gz",
            sha256: "4c5ee2bc248551916ba9f8b2199fbc8009a1486b89f987bdf9ba71ecd7083d6d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyflows-module-0.9.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "9a280dc919bb3aa5631808d958aaf70fd12d0dc00cacad01f889262577cef8cc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyflows-module-0.9.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "833fc554c39ab6f01597cb73a6d859464bc81dfa94b3118a5cd58bbeb6484aca",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyflows-module-0.9.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "77e8deb1197ecb58870e1db85c5c94d60ae9da77649c95face27a4a3dde4b146",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyflows-module-0.9.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "67f7df072b4c25393308f06078a4f9fc7fbb0111a282d7cb26bd364d702d1d4d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyflows-module-0.9.0-windows-2025-x86_64.zip",
            sha256: "28caaea85c38f81b7020b05c6f2ac1bd3d2d01d1be0adab7343b939103b35359",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyflows-module-0.9.0-windows-2022-x86_64.zip",
            sha256: "8226887661e0dcf1a78858a61771f90a4256b8be3a302d5571544f97f5238ee7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyflows-module-0.9.0-windows-11-arm64.zip",
            sha256: "1daec664a8c6f92c13ece028c1c5311d9ef2471f12f4eedb2898d1f6ef709c6c",
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
