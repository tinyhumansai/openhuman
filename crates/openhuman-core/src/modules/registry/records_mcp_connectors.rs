//! Registry records for the `tinymcp` and `tinyconnectors` modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

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
    bus_name: "ai.tinyhumans.tinymcp.Mcp",
    object_path: "/ai/tinyhumans/tinymcp/Mcp",
    version: "0.6.0",
    release_url: "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.6.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymcp-0.6.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "f32a322f180e24cc942e6744f9be9458eebb2e97c1be67795eb51c03b6cecb42",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymcp-0.6.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "cbf0dfc454198b62b6d44830f215abac78100cb211b103984b6b25dbc3eeba41",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymcp-0.6.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "8ffe34e4fa3d7076cef9dcb755182824bdaaadf6f9182dea098c72e99f5a6081",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymcp-0.6.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "7ec7ad75767910696fe472acfd17808cb4b7259b7b3d477cefe133313524bd75",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymcp-0.6.0-macos-26-arm64.tar.gz",
            sha256: "4b0c76f358d80bd7f108fd6c66e8cd881a97271eafa393960360fcd8559a1a7c",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymcp-0.6.0-macos-26-x86_64.tar.gz",
            sha256: "7934a57ca96aa1525cdd14fa4bd31c47d24b38f957401795569f7f2b92cda761",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymcp-0.6.0-macos-15-arm64.tar.gz",
            sha256: "6fd14c70d5ea1bd140d97b8512f754032afe6e794dfcff6b42d4b925769ce9fe",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymcp-0.6.0-macos-15-x86_64.tar.gz",
            sha256: "7430221adcb2d8bcda3c010f600df0fcdbe2acf5854d657562601ad561a3fad4",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymcp-0.6.0-windows-2025-x86_64.zip",
            sha256: "fab6aebe81bc57f86e1c024d77ea0a01fa08f28204ff186bb57c3f581520d62c",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymcp-0.6.0-windows-2022-x86_64.zip",
            sha256: "b490eee140645da47b6eb5edf579af92e3889f25e40fb76ace579afd4e506a66",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymcp-0.6.0-windows-11-arm64.zip",
            sha256: "af8eb16669df093e6dda87a8099e78ce67ec90c8262691249a47b93862480e29",
        },
    ],
    load: LoadPolicy::Lazy,
};

pub(crate) const TINYCONNECTORS: ModuleRecord = ModuleRecord {
    id: "tinyconnectors",
    description: "OAuth connector integrations: accounts, actions, and triggers",
    bus_name: "ai.tinyhumans.connectors.Composio",
    object_path: "/ai/tinyhumans/connectors/Composio",
    version: "0.14.0",
    release_url: "https://github.com/tinyhumansai/tinyconnectors/releases/tag/v0.14.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyconnectors-0.14.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "836bffa2fe4f935c8504f608e08bdf023326d1285fc977fd37149f2799899478",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyconnectors-0.14.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "b7dbafe019aca715e55444a37b5ff4fc469ef2bcc675dfa987e1de626e70fd22",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyconnectors-0.14.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "a29cb970b385cf8cd40998baa3b6f040447196d019a8d3413d67ca64a6509687",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyconnectors-0.14.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "e961477b9cac1892beef8b498ba4f44daa4b39b8dddf277ec49496e3869427b5",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyconnectors-0.14.0-macos-26-arm64.tar.gz",
            sha256: "5a24f1eabb0f246a5954e4d6f93cc0feeaed4fe6bfbf1fceb7399078e0e5734a",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyconnectors-0.14.0-macos-26-x86_64.tar.gz",
            sha256: "5df7dbc74b2f39174e57695c5556ab4c232b775897e7bd534be015ead01ef51b",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyconnectors-0.14.0-macos-15-arm64.tar.gz",
            sha256: "ea8010163f279f14e25a4ad884b4665e24385c560dd2caf46e8c80220556ba11",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyconnectors-0.14.0-macos-15-x86_64.tar.gz",
            sha256: "508e44cc9cd273dcbd2d269295ab0b7009efb1a27d3244ba9adde6b297306db1",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyconnectors-0.14.0-windows-2025-x86_64.zip",
            sha256: "5a5e62e8d11bbdc792c9d8fd2a9b59bcea9add80978457ea0faa7c058cd48546",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyconnectors-0.14.0-windows-2022-x86_64.zip",
            sha256: "1560ed909f338a5bf1118d644d44e1fbeee5c26461241df9f24630e3f9ba3cbe",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyconnectors-0.14.0-windows-11-arm64.zip",
            sha256: "0bdfb0424ad62fc93ba66a7d8dd7063a39ae8656b78d2e65f4f1a6425aec3279",
        },
    ],
    // Lazy: a user with no connected accounts should not pay to load it, and
    // most sessions never touch a connector. Safe even signed out — the module
    // loads without configuration and still answers the capability members.
    load: LoadPolicy::Lazy,
};
