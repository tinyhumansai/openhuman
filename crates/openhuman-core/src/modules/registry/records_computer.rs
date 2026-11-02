//! Native computer-use module (desktop and browser control). Published digests are added only from its release manifest.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};
use tinycomputer_bus::names;

pub(crate) const TINYCOMPUTER: ModuleRecord = ModuleRecord {
    id: "tinycomputer",
    description: "Permission-aware desktop and browser observation and control",
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.15.1",
    release_url: "https://github.com/tinyhumansai/tinycomputer/releases/tag/v0.15.1",
    // All seventeen published archives were checked against checksum.toml
    // and GitHub asset digests before pinning. Linux permission probes are
    // part of the shipped voice surface alongside macOS and Windows.
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinycomputer-0.15.1-archlinux-arm64.tar.gz",
            sha256: "6ad069503afe943824ae566a2057d2c3494d080f3c545ebd9c43b46648f09821",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinycomputer-0.15.1-archlinux-x86_64.tar.gz",
            sha256: "5056d62922f5ef240b4d98fb1ceaf32cb12872e1505e18b5b716daf71ec7cf07",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinycomputer-0.15.1-fedora-43-arm64.tar.gz",
            sha256: "aa0f965bc364b746ecc9463ad6d0e9aea6ea3c2d12f3c3b9a751802cc8b76e0e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinycomputer-0.15.1-fedora-43-x86_64.tar.gz",
            sha256: "017ce21785624ad5422883060c854af372f62b625fcdd2a3aaef386b52cb9958",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinycomputer-0.15.1-fedora-44-arm64.tar.gz",
            sha256: "3d084b6f73010230ec1b3cd94b92118751094713ec1ee9ec9fe9fcf0379dc882",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinycomputer-0.15.1-fedora-44-x86_64.tar.gz",
            sha256: "a4b24930c6b1a8144554f8dd37f5746d4b8fca0f47ba1a36b740f523095f260c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinycomputer-0.15.1-macos-15-arm64.tar.gz",
            sha256: "4f783bdcebc4b16af54aa85b963dda5b0083a2c7c8b20a4b8e9807e1351f9c31",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinycomputer-0.15.1-macos-15-x86_64.tar.gz",
            sha256: "58725f5838a3788d6a970b86e91dca5796b84589b1e9b1fd357e9ac6465a9252",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinycomputer-0.15.1-macos-26-arm64.tar.gz",
            sha256: "ef10caa01120d71b9045d70cca3b559ac95c9d7e05734b8d2215d8e0eb5d0559",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinycomputer-0.15.1-macos-26-x86_64.tar.gz",
            sha256: "55f93c8f9893055e092c8a3ef711757f3bd76afd38eca84ad70ca47b83c07ebd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinycomputer-0.15.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "f2e37604107fcbac023abf5ca0e54acf556668deb6225bb9a325e119aac4fc13",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinycomputer-0.15.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "4e7ee56bda5a6e4755ac0610f4da22cac71c5df2d3a3321dd83dfd0bc19427df",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinycomputer-0.15.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "88be9bc3f75fc233b286dd18667cd88e5a1f75a40579950b0f5335b1457f83ff",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinycomputer-0.15.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "cc839ff33bec552e8e1550a203f914af70772258d2070929e45b57cd8a00e9bb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinycomputer-0.15.1-windows-11-arm64.zip",
            sha256: "c312551895d5c5e2b2bd9e42d02177e94c637c88ef0f9c1c0e8ae891096d4507",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinycomputer-0.15.1-windows-2022-x86_64.zip",
            sha256: "34b11d7bb2e291b853215492f8d8263a353fba42605535e08d2718403129d5f5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinycomputer-0.15.1-windows-2025-x86_64.zip",
            sha256: "016dfb8b620e6f1582230cf20075d5060e96b84b7bd2b19ad171afedae7e0d5c",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinycomputer",
        manifest: "crates/tinycomputer/Cargo.toml",
        package: "tinycomputer",
        lib_name: "tinycomputer",
        features: &[],
    },
};
