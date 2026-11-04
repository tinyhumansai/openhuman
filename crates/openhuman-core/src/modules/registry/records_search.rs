//! TinySearch web-search module. Published digests are added only from its
//! release manifest.
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

pub(crate) const TINYSEARCH: ModuleRecord = ModuleRecord {
    id: "tinysearch",
    description:
        "Web search, grounded answers and page contents across providers through TinySearch",
    bus_name: tinysearch_bus::names::INTERFACE,
    object_path: tinysearch_bus::names::OBJECT_PATH,
    version: "0.5.6",
    release_url: "https://github.com/tinyhumansai/tinysearch/releases/tag/v0.5.6",
    // Verbatim from the published v0.5.6 checksum.toml; the same host set as
    // the other native modules (macOS, Ubuntu, Windows).
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinysearch-0.5.6-archlinux-arm64.tar.gz",
            sha256: "0ca7234f29073cdb9d04c045cbcdf129704aac2a56fef01aa9b03dc2001eff41",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinysearch-0.5.6-archlinux-x86_64.tar.gz",
            sha256: "c423d5371ab388f8174b412de8e1fe16e342dd99cfc39075bcf4663db3a2f42a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinysearch-0.5.6-fedora-43-arm64.tar.gz",
            sha256: "047631f61b978cc335e90d74ab5258a8f450096b833a405309e455eceecc9c66",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinysearch-0.5.6-fedora-43-x86_64.tar.gz",
            sha256: "d574f27e5ae610878bc21712e16ef46df71406daba818c2986023ee0a067ce13",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinysearch-0.5.6-fedora-44-arm64.tar.gz",
            sha256: "bdb0cb4d934f377e1f2998905297dc24377b8554b856b4738cf7cd30878d7d26",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinysearch-0.5.6-fedora-44-x86_64.tar.gz",
            sha256: "0c91811c7529ed8d7ccde65029c6c548c6fa22472a18f329fc4fbc480fa66eee",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinysearch-0.5.6-macos-15-arm64.tar.gz",
            sha256: "71fd0fbd18eb708339dd5e7d4b9ff9bf3e7d8099d486600946968f1c96f4ad1c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinysearch-0.5.6-macos-15-x86_64.tar.gz",
            sha256: "164e961085145d00017233f08635f7256d17a212cfe44798320beabd77ba470b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinysearch-0.5.6-macos-26-arm64.tar.gz",
            sha256: "4ee87355ead3825d00e786c166105a839e87793f631df8d820dc255624c3ffba",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinysearch-0.5.6-macos-26-x86_64.tar.gz",
            sha256: "3f5c08072493ce252208be53c7da7776615e8eb26e29214cb16a3565801fae27",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinysearch-0.5.6-ubuntu-22.04-arm64.tar.gz",
            sha256: "5e133addabef60e5cb8f317527a3b6eff056a4793284135a372c4fe3d01f8d39",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinysearch-0.5.6-ubuntu-22.04-x86_64.tar.gz",
            sha256: "5478db74ca1ff351ccde956fa202fb8b253a63f44c09f5229d346a7fc895f64f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinysearch-0.5.6-ubuntu-24.04-arm64.tar.gz",
            sha256: "cada1784a55361f04c1ce6504f599b0abefb60a631eb273348af6131ab2b508b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinysearch-0.5.6-ubuntu-24.04-x86_64.tar.gz",
            sha256: "75e517367ddc3f4e76c7632566498e8abf5f317b7b84c115ce9f1287c65dca85",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinysearch-0.5.6-windows-11-arm64.zip",
            sha256: "814b702e24eed2cb7795678f361293f2d14ce3bae5a46d8c1a976c8bab81f3d6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinysearch-0.5.6-windows-2022-x86_64.zip",
            sha256: "db643ca392b7a90ffb30e3cfcae11630061ec750548bd67410ae21e4e51affee",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinysearch-0.5.6-windows-2025-x86_64.zip",
            sha256: "2fc0a71d2f6a3b5ac02207fe73dee48a53060f407a8c7dc543a721a5f5305fca",
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
