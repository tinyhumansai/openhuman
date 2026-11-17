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
    version: "0.20.6",
    release_url: "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.20.6",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-arm64",
            archive: "tinymcp-0.20.6-archlinux-arm64.tar.gz",
            sha256: "af521ed1dcc7bf781a5def1d99356b529bb5bfafc38295d4fa370884b4f4946c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinymcp-0.20.6-archlinux-x86_64.tar.gz",
            sha256: "8968cdeca99b4282c0ee17b04817552a51b8341589d437ba4c76782872ab6fbb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinymcp-0.20.6-fedora-43-arm64.tar.gz",
            sha256: "c5dc43b7f923ae23009f449c9ec62fbe3b474834b775e3c22b6266fd23cd891b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinymcp-0.20.6-fedora-43-x86_64.tar.gz",
            sha256: "598ab75907d76a829e41e8d3efe78e9f65bfb74e0fd565b07d861ed9c7b9bed5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinymcp-0.20.6-fedora-44-arm64.tar.gz",
            sha256: "7cf36b64af0b22723f4cea25ba5e9d08219fbeb26885fddf002f5669f963da93",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinymcp-0.20.6-fedora-44-x86_64.tar.gz",
            sha256: "388e9ee9da761a608dcb4eeb433b56d5bace5c099e4fe0fa155889777903a74e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymcp-0.20.6-macos-15-arm64.tar.gz",
            sha256: "de7ea2933772e1addf207480850e7eeab4c18d8a410d1b62e4c02a3e42cfc26b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymcp-0.20.6-macos-15-x86_64.tar.gz",
            sha256: "f2c56a2fe9b3762e4879ccb1d7bac4a101e2c758ed2726bf9b6aa5baf6dffeb7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymcp-0.20.6-macos-26-arm64.tar.gz",
            sha256: "b2117c4dc28c0846c395b0e7564354476b55fdff867c5698f38a967e29ea03d3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymcp-0.20.6-macos-26-x86_64.tar.gz",
            sha256: "ef26b7c7fed734c0d31a6d40259e4c500c5ed6bfde367dcc8fcfee7d0a32bd71",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymcp-0.20.6-ubuntu-22.04-arm64.tar.gz",
            sha256: "e0eaed91578641ba6548fae2a98a95aa6077eddfd0c6f82186df3ba10ce71f39",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymcp-0.20.6-ubuntu-22.04-x86_64.tar.gz",
            sha256: "f8634b838e8329ffbd056a109abd9ad8b1bd8e89b6bb78bbb85c52d5e56f66a9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymcp-0.20.6-ubuntu-24.04-arm64.tar.gz",
            sha256: "06cbb7c1e1040d8fa5fd1a8f11b57405e389d08478df3c66b161ab471984d7f5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymcp-0.20.6-ubuntu-24.04-x86_64.tar.gz",
            sha256: "b295e155e819ac56dd2c1e1fd8c895012ea442d77a35b552f1096703a2c7cc37",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymcp-0.20.6-windows-11-arm64.zip",
            sha256: "37fbec7c0d8c040ac6278cff59c75aeb3f4cefb3f4cfb05cab576e8f8d6712b5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymcp-0.20.6-windows-2022-x86_64.zip",
            sha256: "41572a71adde15fcadf32d64687da34cf31947123b4e132acd7189e432f50c07",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymcp-0.20.6-windows-2025-x86_64.zip",
            sha256: "df9a8467b16782ccba2634910714d24f940e2366243537efa24edc88b0712434",
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
    version: "0.22.2",
    release_url: "https://github.com/tinyhumansai/tinyconnectors/releases/tag/v0.22.2",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyconnectors-0.22.2-archlinux-arm64.tar.gz",
            sha256: "ce5dee403176a0db69dde0e4ec469c1cae6a94cc541f24ee77803fa51eef433f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyconnectors-0.22.2-archlinux-x86_64.tar.gz",
            sha256: "b39002d0f9109a31f963c695d6f732b3838c0836d7e64569da65070284a1bb4b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyconnectors-0.22.2-fedora-43-arm64.tar.gz",
            sha256: "255872a4e95f989189782bbbae70abf913d0ba91f2a5baa41afc12792289810b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyconnectors-0.22.2-fedora-43-x86_64.tar.gz",
            sha256: "5a5722b4d738bc8d4d0e863f9bb844fe314eb239d13ba278614a7439f9e6225e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyconnectors-0.22.2-fedora-44-arm64.tar.gz",
            sha256: "fa5adf822910cb12f9c2e57a68db0801043d15d1656fb4933280533fb3cd78de",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyconnectors-0.22.2-fedora-44-x86_64.tar.gz",
            sha256: "c6ed50f9345fe68eda2b08c101353940cf3fccbc62a5639c2de88b309f5b20cb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyconnectors-0.22.2-macos-15-arm64.tar.gz",
            sha256: "1d8f39e3628ecdfc7a6ae07cc9a07c508b1b9dc78c432969b8c76dfbeaaa6f70",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyconnectors-0.22.2-macos-15-x86_64.tar.gz",
            sha256: "8e9d4f6b354c89bef0af1c81be61c6b58edc08598d86aec8aa014a2cfd887184",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyconnectors-0.22.2-macos-26-arm64.tar.gz",
            sha256: "159ba103cc67187190adb02b0f9797a09ebd823b45d061cd95fbae95fa3adfa0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyconnectors-0.22.2-macos-26-x86_64.tar.gz",
            sha256: "0f37a851f64ab578e4cddf65699f2e9b7465d9a5831a5e5fa1baf4c40315cca0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyconnectors-0.22.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "935f50877c0150cc2997c321cebf3c1da80197eebedae9d3f83f7a6444f45960",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyconnectors-0.22.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c66081ad0f1d8a4d77c8e5602cfe7ce840f8e5bf08620ab2cb3cebe423b5adff",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyconnectors-0.22.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "1e104653ea94fc304a631bf85f2360f02a9c39fac9dccf5eea4eb08e427ac780",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyconnectors-0.22.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "52437710952ba485f675e12b970cb7ec63ab22a47a14122a3a37bea3c3d90db9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyconnectors-0.22.2-windows-11-arm64.zip",
            sha256: "6d01fa4e03bb296440dcd046e5cb82b704a2c36f42d5e55f602398c219c75719",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyconnectors-0.22.2-windows-2022-x86_64.zip",
            sha256: "4727cc00d3835c219097d5ac70884f9f4b3be9bab474a00b7fd06c115a9eb0e1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyconnectors-0.22.2-windows-2025-x86_64.zip",
            sha256: "e9961fc3d12a8137475f226b5b3a9f2be7dbc093b32807b53dd3ef78faf02498",
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
