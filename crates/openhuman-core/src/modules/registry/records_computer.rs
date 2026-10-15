//! Native computer-use module (desktop and browser control). Published digests are added only from its release manifest.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};
use tinycomputer_bus::names;

pub(crate) const TINYCOMPUTER: ModuleRecord = ModuleRecord {
    id: "tinycomputer",
    description: "Permission-aware desktop and browser observation and control",
    bus_name: names::INTERFACE,
    object_path: names::OBJECT_PATH,
    version: "0.14.0",
    release_url: "https://github.com/tinyhumansai/tinycomputer/releases/tag/v0.14.0",
    // All seventeen published archives were checked against checksum.toml
    // and GitHub asset digests before pinning. Linux permission probes are
    // part of the shipped voice surface alongside macOS and Windows.
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinycomputer-0.14.0-archlinux-arm64.tar.gz",
            sha256: "ff8497d68b8bef43e7db2ac93d2fe6bf6a1c6b57a65074a4819985f8cfc2425c",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinycomputer-0.14.0-archlinux-x86_64.tar.gz",
            sha256: "84cce40fe362fcfdfcba03b394e2a6b4288510685b3fac3f725c641173183c72",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinycomputer-0.14.0-fedora-43-arm64.tar.gz",
            sha256: "15b581be59e7aeb2a6a0b9b3122f2d8fe5112b2a6fe57964300609f8c0d83ba4",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinycomputer-0.14.0-fedora-43-x86_64.tar.gz",
            sha256: "31fc8a1d96eb8837fa35b100a5d84634cdf36ede71cce898dcc83a55a5562df4",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinycomputer-0.14.0-fedora-44-arm64.tar.gz",
            sha256: "f409e84a62551e9e7a37a1e409d0e2b5470154fa5df560076a107db4b51ff450",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinycomputer-0.14.0-fedora-44-x86_64.tar.gz",
            sha256: "0c259e4aeb7bcb770869f394f7e9a33d38381233bf58e7d275af43547b8b0245",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinycomputer-0.14.0-macos-15-arm64.tar.gz",
            sha256: "68c6f255e9035d95886f285ba3a5890815b2b90caa41fa7fb36706c59e7871da",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinycomputer-0.14.0-macos-15-x86_64.tar.gz",
            sha256: "b01327bf4887d6976cbbc51cd8e31d3d2dca95ff13ab550670859ad501be02d8",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinycomputer-0.14.0-macos-26-arm64.tar.gz",
            sha256: "05872f842caa298380a54ea9108aeca04152a70c78fe7177936f61d21b43bc38",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinycomputer-0.14.0-macos-26-x86_64.tar.gz",
            sha256: "0e7da08863c15970885e6ec5b75f4052759030c71dfbcc57805485f5941d6b44",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinycomputer-0.14.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "2ed418322976dc9cb126cd89e383cba1dce5b635e96606864ffa9a7959ae0565",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinycomputer-0.14.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "408da80c206ccee33354f1c88665bc335fd58f896ab7b572956e4054a2c7c494",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinycomputer-0.14.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "886131a512e42a673a8a797d03fe79bc5ae28d5c4e59018eea13970cd8719615",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinycomputer-0.14.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "fe79e93bf45d1ba3cd4212d04872530462a99d3d220da182070d54817e8291c6",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinycomputer-0.14.0-windows-11-arm64.zip",
            sha256: "f6fba78852f886b66851205f74af0610e75270bf6db26ee7966c5a25f27884fc",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinycomputer-0.14.0-windows-2022-x86_64.zip",
            sha256: "6cebf513dea3d5f4da32247104e4188a7565cb002bda6d93e15a8ccab8aa655f",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinycomputer-0.14.0-windows-2025-x86_64.zip",
            sha256: "acd13f6578117a042a5768dc9381be9cf5f78dac968df11c9b4f1e9c6d3c1e97",
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
