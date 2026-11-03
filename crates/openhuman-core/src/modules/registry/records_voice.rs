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
    version: "0.4.2",
    release_url: "https://github.com/tinyhumansai/tinyvoice/releases/tag/v0.4.2",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyvoice-module-0.4.2-archlinux-arm64.tar.gz",
            sha256: "0cef54f6248019db21282fa76db4f9663fbe830c3b18c2804ab28833d60329e1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyvoice-module-0.4.2-archlinux-x86_64.tar.gz",
            sha256: "707838c2fb8c3e4ad861d56e37418fb71c1b21bc56826fb159fd46057f1af9f0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyvoice-module-0.4.2-fedora-43-arm64.tar.gz",
            sha256: "51c1d67747872016127c3633fda9ea7111eaa8142782e6b6da4ef15ed9c4a171",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyvoice-module-0.4.2-fedora-43-x86_64.tar.gz",
            sha256: "01aac694078952c8dea1090bc99b584930c4a8a043ff8defb0512110f4aa9433",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyvoice-module-0.4.2-fedora-44-arm64.tar.gz",
            sha256: "21bb4a50fe65aaa6a5d09f94d15933c3dd7878b7cf919daa06f2f0939ea421be",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyvoice-module-0.4.2-fedora-44-x86_64.tar.gz",
            sha256: "7113c298e9abbbfacc55a45e237b4962ce9b0256f9ffe12e901ee45cddb226d0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyvoice-module-0.4.2-macos-15-arm64.tar.gz",
            sha256: "1c0cf8391e8afb35a13763b6c05fe811c14a640fdf9a79e17bae7f32952009d0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyvoice-module-0.4.2-macos-15-x86_64.tar.gz",
            sha256: "72c6d9fb1aad0b4ce3f7cceaad45111d4450e9a1285140945dcc585a69e16bde",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyvoice-module-0.4.2-macos-26-arm64.tar.gz",
            sha256: "2b37df831a1196f5d379994a1b85e9f69015d7cde47564470afbb2ff31a85b77",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyvoice-module-0.4.2-macos-26-x86_64.tar.gz",
            sha256: "1f9ffd9307505693be31df2f6eaa0e6de32f104bd46640c06dc82bffa3e03fa7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyvoice-module-0.4.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "ad63d27d2c40f8388d93a9d3556bfc8539c4b5ffe8839efaa0c1b7a1b7a61d43",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyvoice-module-0.4.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c2523bf5066757ce0c81e7f5c97a4b902312007ce079606d005e008ca8be66a5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyvoice-module-0.4.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "78da28da3a483731bec8d53bc561219cc6dec793d66f27eb908755d88118461e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyvoice-module-0.4.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "1b542b8b39223525f40f02245ef21b7d995a039eef9cb8a2586bb46cce9668f9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyvoice-module-0.4.2-windows-11-arm64.zip",
            sha256: "734db41e5c5cd5580c5aa4fd7a9c3e5281aee62ea17c31d8c7037c7356360e82",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyvoice-module-0.4.2-windows-2022-x86_64.zip",
            sha256: "47701888511b5b8ac71a5f92affcb04334b43e479b43e2b5a0937f2134f4cc5e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyvoice-module-0.4.2-windows-2025-x86_64.zip",
            sha256: "46b29e886d3ec82b996ca7138a7a60577fe6236383df59cb34aedfd3c670e9b0",
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
