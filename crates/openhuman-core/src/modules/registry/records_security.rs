//! Published TinySecurity admission pins copied verbatim from v0.2.2/checksum.toml.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

pub(crate) const TINYSECURITY: ModuleRecord = ModuleRecord {
    id: "tinysecurity",
    description: "Native security policy and immutable filesystem authorization scopes",
    bus_name: tinysecurity_bus::names::INTERFACE,
    object_path: tinysecurity_bus::names::OBJECT_PATH,
    version: "0.2.2",
    release_url: "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.2.2",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysecurity-module-0.2.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "15e15e19676089a344f75954d4a48df3dd2e0b26b84b93f1a3df9546687a0ee7",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysecurity-module-0.2.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "efda938512324708e8dd118483f082a3cdccd479465c21381178b16301fe498c",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysecurity-module-0.2.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c8705920c2a913dbe6ded085d4a294f9c38a21b993a7407037ac6a23825af405",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysecurity-module-0.2.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "25745c487ab411eca5a495fa06fdf0db4ba4dbe4a8e75b4fadcd7211a27f67d3",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysecurity-module-0.2.2-macos-26-arm64.tar.gz",
            sha256: "415e4e5e65dacbfe7bc0d5fff9e31d712e60cefa1bceae089c31a148a0a039f4",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysecurity-module-0.2.2-macos-26-x86_64.tar.gz",
            sha256: "17cc99c81a0f451e2fb2f8fcdd7d140e25f38e92a4c1abf499da5cd8368e952a",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysecurity-module-0.2.2-macos-15-arm64.tar.gz",
            sha256: "b9982f24ce251435030f6ef22dd36af45940eba23160d26c53f136eecac1e3ea",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysecurity-module-0.2.2-macos-15-x86_64.tar.gz",
            sha256: "c5eb37f160967bdf28c873d9a5a74cc217f3b6cf5539d68c6c58f2e1dc0f8542",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysecurity-module-0.2.2-windows-2025-x86_64.zip",
            sha256: "b576f789ad34b969166a77b952b15f5a2229c0c40ba35c60f3faa0630a85bd7e",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysecurity-module-0.2.2-windows-2022-x86_64.zip",
            sha256: "0ae898d66954edb24103843e88ce3487e6970b5a4560791f3880b69db90fc54d",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysecurity-module-0.2.2-windows-11-arm64.zip",
            sha256: "6423f1eb1c76cd32c1c65fc784ff539900554c8a9857a42a9f147a30d0691fc0",
        },
    ],
    load: LoadPolicy::Eager,
};
