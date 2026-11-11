//! Registry record for the `tinyvoice` module.

#[cfg(feature = "voice")]
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

#[cfg(feature = "voice")]
/// The `tinyvoice` module: the host-agnostic half of the voice pipeline.
///
/// STT hallucination detection used by the voice transcription pipeline.
///
/// Lazy: transcript screening loads the module only when voice transcription
/// needs it.
pub(crate) const TINYVOICE: ModuleRecord = ModuleRecord {
    id: "tinyvoice",
    description: "Voice transcript hallucination detection",
    bus_name: tinyvoice_bus::names::BUS_NAME,
    object_path: tinyvoice_bus::names::OBJECT_PATH,
    version: "0.4.3",
    release_url: "https://github.com/tinyhumansai/tinyvoice/releases/tag/v0.4.3",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyvoice-module-0.4.3-archlinux-arm64.tar.gz",
            sha256: "d17a2610a57e023f1492301fd49bbf9ccfe14382add281f4cc0f0ea11b82bcb3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyvoice-module-0.4.3-archlinux-x86_64.tar.gz",
            sha256: "3cef1a9d35c9637a1a31513f1d172717d1293eb7958ee380f773475f33d8979d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyvoice-module-0.4.3-fedora-43-arm64.tar.gz",
            sha256: "565c95f7a677f8d7c619c16ec0bc31c3abbc1b5863fa1f1c8616d25607c71788",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyvoice-module-0.4.3-fedora-43-x86_64.tar.gz",
            sha256: "3d8ef34b8120edd13496a3b51178048ab489f3281fa9bb36890999ea10c01228",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyvoice-module-0.4.3-fedora-44-arm64.tar.gz",
            sha256: "21bdaf1e19c1ca858124a38eff712bc7d2b49c6a2806902e93525029de5fae2a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyvoice-module-0.4.3-fedora-44-x86_64.tar.gz",
            sha256: "ec84a82551cc61bd03a305d7004b763e03dda56b9cfe70d2ab9dc57395f3f1af",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyvoice-module-0.4.3-macos-15-arm64.tar.gz",
            sha256: "181b588bef33710e2ae3e35bd17da3d3f1390ca6278548cdfbc9321e3a0f2470",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyvoice-module-0.4.3-macos-15-x86_64.tar.gz",
            sha256: "ee08802d3b39d2afcbc8cc445ae5c61584d3099df0d1efa2458cfa4648f0ad28",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyvoice-module-0.4.3-macos-26-arm64.tar.gz",
            sha256: "af827579dded0d0c6b307f332d6907be258831b2b21dd8595a807052d5879acf",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyvoice-module-0.4.3-macos-26-x86_64.tar.gz",
            sha256: "b253819adf530d0a97c0299850f4aa72574a4dc4147afce8677240b3abf06cbc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyvoice-module-0.4.3-ubuntu-22.04-arm64.tar.gz",
            sha256: "dc3a472a525cfe91940bcb30696af3d5c5a2f1a2378d40d282b4834c28187750",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyvoice-module-0.4.3-ubuntu-22.04-x86_64.tar.gz",
            sha256: "f69566e150bc27b6bbf93eaf5fbf49221e2264bc983ea9f67c5fa96f039b2a3b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyvoice-module-0.4.3-ubuntu-24.04-arm64.tar.gz",
            sha256: "9cfe3429144f54d12cc3150b3e88ee77748015099b746cabd659bc78dcae8e30",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyvoice-module-0.4.3-ubuntu-24.04-x86_64.tar.gz",
            sha256: "06d1756f6b470034e83f40d61bdda7d353e9be9df9cdfbd5e6160bfcc300ef1f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyvoice-module-0.4.3-windows-11-arm64.zip",
            sha256: "789ba0cbe7f46888493786d76e31b4c5e8c1c20375563c9632ff332f6718f972",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyvoice-module-0.4.3-windows-2022-x86_64.zip",
            sha256: "1c6259d330e5b134a829d4dbdfb8daa2d29f3754f51ab3507a3b28e51f63e9d2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyvoice-module-0.4.3-windows-2025-x86_64.zip",
            sha256: "b2311f4895ba74b5ec359202d06dc951c0430311f1e38d2dd659e2b030748059",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyvoice",
        manifest: "crates/tinyvoice-module/Cargo.toml",
        package: "tinyvoice-module",
        lib_name: "tinyvoice_module",
        features: &[],
    },
};
