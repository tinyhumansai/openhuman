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
    version: "0.4.1",
    release_url: "https://github.com/tinyhumansai/tinyvoice/releases/tag/v0.4.1",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyvoice-module-0.4.1-archlinux-arm64.tar.gz",
            sha256: "b6bb17e6097f1b089f014c2392e028a481b0c6f8543652d7fc8ae890c86777a7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyvoice-module-0.4.1-archlinux-x86_64.tar.gz",
            sha256: "fde049b4112de018799a91215cbc84572a9b4bd598bb44cfa61d8fcd1b96c9f9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyvoice-module-0.4.1-fedora-43-arm64.tar.gz",
            sha256: "58a9bd07922770610b7f42e6f3708940ca501f430d27cd3ebbb6115402fdb9c8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyvoice-module-0.4.1-fedora-43-x86_64.tar.gz",
            sha256: "e8c95d1d85f4d89a1b8f8fa1740f8e174c835dab3ad856a99184749ca846bca7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyvoice-module-0.4.1-fedora-44-arm64.tar.gz",
            sha256: "15370fb697a4ffdafe9264728b45efbbe155a91cf00aeba55471b0daa85c4768",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyvoice-module-0.4.1-fedora-44-x86_64.tar.gz",
            sha256: "c7ee0327548bd05079ba30af0108385bea723bb8080f846c4def31ddbab57fcd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyvoice-module-0.4.1-macos-15-arm64.tar.gz",
            sha256: "77332a814521ac6ae0a5fd36671088ab6596b981544130b852431fe1d08e9f49",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyvoice-module-0.4.1-macos-15-x86_64.tar.gz",
            sha256: "2b4556461e3a30cb06fa5b548cc7e9ee8aee63475de05d16d4809140c44d9f4c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyvoice-module-0.4.1-macos-26-arm64.tar.gz",
            sha256: "17c1a6a80c1ae7a46fb19ce129e4b0dffb3d6391747bf9d1a5df0fab6ac75c48",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyvoice-module-0.4.1-macos-26-x86_64.tar.gz",
            sha256: "1cf0fbd9f20ffc449a9df938cdc15be4d9c2d4077a839ddeba4423ed8ec2f175",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyvoice-module-0.4.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "001b8b76b920dcab616a1f504efb2893c0ce87a251513dfd8e52c6414b239c7f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyvoice-module-0.4.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c6f6a764747324a2789d9a6cd3764b6b5a5dc08a4a1ca305e157f65df762ae0d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyvoice-module-0.4.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "8cb7d71e60213896326b85a551dfdd8af2ff1ee8b1a6d6fab27a53ed0d457f57",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyvoice-module-0.4.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "7755ee1652735ac081bedbdc6dd6a80f8171639cd480f3a0d044230f5da2b0f3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyvoice-module-0.4.1-windows-11-arm64.zip",
            sha256: "ea575671e6d119886ec09ea43eed5bd35a7c9445aa8d54fa4621e26100856c6c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyvoice-module-0.4.1-windows-2022-x86_64.zip",
            sha256: "b472b4ef877bca793b45fca87bb5edbc43f119748cff85a3a25ea6d83815e254",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyvoice-module-0.4.1-windows-2025-x86_64.zip",
            sha256: "c8c4cd75139c09a31082747d9ad4c1b656d9f6d4f8572370ee8017779c3fc1c0",
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
