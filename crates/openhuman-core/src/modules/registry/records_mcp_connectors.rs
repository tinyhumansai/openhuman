//! Registry records for the `tinymcp` and `tinyconnectors` modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

/// The `tinymcp` module: the Model Context Protocol client.
///
/// Owns both transports (Streamable HTTP and a subprocess over stdio), the
/// statically declared server set a host puts in its own configuration, the
/// dynamic registry of user-installed servers with its SQLite store, the
/// reconnect supervisor, the browser sign-in flow, and the write-audit log.
///
/// Lazy, because dialing an MCP server is something most sessions never do: a
/// host with no installed servers and no configured ones would otherwise pay a
/// download and a `dlopen` for a capability it never reaches. That differs from
/// the module's own `lazy = false` export hint, which speaks for a host whose
/// servers should be connected the moment it comes up — this host decides when
/// that moment is, and does so on the first ask.
///
/// **What stays out of the module is host policy**, and the split is the same
/// one the contract's own documentation draws: the prompt-injection scan over
/// remote tool definitions, the `mcp_clients` RPC surface, the
/// agent-facing tools, and the proxy *scoping* decision all belong to this
/// application's threat model, not to a protocol client. `tinymcp-bus` carries
/// the vocabulary; this table says which bytes may speak it.
pub(crate) const TINYMCP: ModuleRecord = ModuleRecord {
    id: "tinymcp",
    description: "Model Context Protocol client: transports, registry, and the write-audit log",
    bus_name: tinymcp_bus::names::INTERFACE,
    object_path: tinymcp_bus::names::OBJECT_PATH,
    version: "0.20.2",
    release_url: "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.20.2",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-arm64",
            archive: "tinymcp-0.20.2-archlinux-arm64.tar.gz",
            sha256: "130292bd749ce4e075642966e827494dc3f645cd310bfd993579b65645d757eb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinymcp-0.20.2-archlinux-x86_64.tar.gz",
            sha256: "2395b184ef39b614914c4cf9a928733617a5b441c6de78e461dcc6c6989f21c4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinymcp-0.20.2-fedora-43-arm64.tar.gz",
            sha256: "1e3c747a255de2503476f600dfd18598f4f857c74998f96c97fe8e3ae2f3eff5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinymcp-0.20.2-fedora-43-x86_64.tar.gz",
            sha256: "8e295d0576100535dd4feebdbceb6114fce6670656ee3f965f26ca20ef6347c3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinymcp-0.20.2-fedora-44-arm64.tar.gz",
            sha256: "86948bdb4d2a9928bc972b4490af5ab289dc7bca33c7198c9df674ccce966be3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinymcp-0.20.2-fedora-44-x86_64.tar.gz",
            sha256: "2652a53066843e4a7d379bc5584e95566f2fa80dd29c9c916408022ca37b5bd0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymcp-0.20.2-macos-15-arm64.tar.gz",
            sha256: "369c875740741a7f60da6a29e55e6aad97b65a067190c90aba89b68401721a04",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymcp-0.20.2-macos-15-x86_64.tar.gz",
            sha256: "0cd9af717a68851e1030656558664a48bc68cf75a2bd1e4cf6714195cf295f76",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymcp-0.20.2-macos-26-arm64.tar.gz",
            sha256: "fe69bcfae6634a6d8af67b6496f7f53eaa9361f14ee1706f0a0176be6c550670",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymcp-0.20.2-macos-26-x86_64.tar.gz",
            sha256: "3303a4bd0a40d1400f44691adf8c412c64999f804913839643c29589664e4ded",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymcp-0.20.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "cc96d0dbaffdb33de1727f17298ea7e7e9fb53a63199732554340133c5920b2d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymcp-0.20.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "f580249fe9f4b9e2587910e686a3bb0999c63ff461aaed9bd812ce7738a14a75",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymcp-0.20.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "386c0cd318da9e5c88c91fba21cfacbf2b5096768ce3fe609930eca73ee8c5d3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymcp-0.20.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "63b9bf334f3c1e0dbf7db795e33c18ab05849da72c83837bc58ac93b81e5668f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymcp-0.20.2-windows-11-arm64.zip",
            sha256: "bbd2c09d0cc4b8acd7af8ba985151d83c87c8b454748f35db44d22027099fa84",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymcp-0.20.2-windows-2022-x86_64.zip",
            sha256: "51e5c3a435204913692fe006ed6da0e425a122536f9e2f888ba1e9f1d5400da0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymcp-0.20.2-windows-2025-x86_64.zip",
            sha256: "e7611602514858e6c3e9bbf942adcd4ea3f9d16d3fcfea10444a5a21b1a77eda",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinymcp",
        manifest: "crates/tinymcp/Cargo.toml",
        package: "tinymcp",
        lib_name: "tinymcp",
        features: &[],
    },
};

pub(crate) const TINYCONNECTORS: ModuleRecord = ModuleRecord {
    id: "tinyconnectors",
    description: "OAuth connector integrations: accounts, actions, and triggers",
    bus_name: tinyconnectors_bus::names::INTERFACE,
    object_path: tinyconnectors_bus::names::OBJECT_PATH,
    version: "0.22.0",
    release_url: "https://github.com/tinyhumansai/tinyconnectors/releases/tag/v0.22.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyconnectors-0.22.0-archlinux-arm64.tar.gz",
            sha256: "0661f0a56ec90a77fe463dc318570b5a1e449409ab315af9194fa7dd9bc8fcf7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyconnectors-0.22.0-archlinux-x86_64.tar.gz",
            sha256: "46fd93121bc93e144cb58d76084f477875158321f0b376d6bcc99e7d83f81939",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyconnectors-0.22.0-fedora-43-arm64.tar.gz",
            sha256: "d1529cc6cb012b1b8822fcc12314a5f8c1fab071253355eafde1a6da5c277199",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyconnectors-0.22.0-fedora-43-x86_64.tar.gz",
            sha256: "b0ddba48640ad92ccebc986e1fbc700f3cf0459d7b78a6de6101d0e3dd8db035",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyconnectors-0.22.0-fedora-44-arm64.tar.gz",
            sha256: "255b22015a2429323b05a4713564b1d3253657dcbb39ec2c5fefbaea40f524e4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyconnectors-0.22.0-fedora-44-x86_64.tar.gz",
            sha256: "97f011d44f06d7f10d3ed79219cc3dcdb0dd0fa902d93a2e28819adc64f83bc6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyconnectors-0.22.0-macos-15-arm64.tar.gz",
            sha256: "71e1f21119d461a26bd5b3d58bd6c3a726bc0f12624bb352ea34cc91b0666c82",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyconnectors-0.22.0-macos-15-x86_64.tar.gz",
            sha256: "856e821f651284ae6233fdfefa8f78cba01e1ffc9bd809c09e52e121926daf80",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyconnectors-0.22.0-macos-26-arm64.tar.gz",
            sha256: "8d4f65247b87a37cea3ad3483c1af7084310c9d1c26cba2ab204a7c97d673840",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyconnectors-0.22.0-macos-26-x86_64.tar.gz",
            sha256: "b3f2c2b37123bb55db986706c52ca534105d393454cbb9b78380159dfb3a1bc9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyconnectors-0.22.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "e797288750066bbbf59846c38d6dd92d6b1f83dfbfdd959b1694f69c9fc7cbb3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyconnectors-0.22.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c6a25a8a7605f0d88e5969b7da7ddd3cb8a6b33abbd9eee380a895e0e992c036",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyconnectors-0.22.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "195d3dd23f8b40fe378e28e41089920c105baa357668a9dc9cf275a3e89134cb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyconnectors-0.22.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "7493f7bf799d0e7f9667e21a956aab826141cefad738e2bfd3d71d928b9b4142",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyconnectors-0.22.0-windows-11-arm64.zip",
            sha256: "57d11f8f8f413ce61dc3a4ca0b93f1bde3fb0cd62dcc3a583775be6d5c76323a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyconnectors-0.22.0-windows-2022-x86_64.zip",
            sha256: "5fb9b593b3545aeeb63dba116a2eb79946aea536182c08a6cabe59632a6d8f69",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyconnectors-0.22.0-windows-2025-x86_64.zip",
            sha256: "15b2a32e869768f58e9205fefee88193f0fc0847080e8ad90b52314905f83aef",
            library_sha256: None,
        },
    ],
    // Lazy: a user with no connected accounts should not pay to load it, and
    // most sessions never touch a connector. Safe even signed out — the module
    // loads without configuration and still answers the capability members.
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinyconnectors",
        manifest: "crates/tinyconnectors/Cargo.toml",
        package: "tinyconnectors",
        lib_name: "tinyconnectors",
        features: &[],
    },
};
