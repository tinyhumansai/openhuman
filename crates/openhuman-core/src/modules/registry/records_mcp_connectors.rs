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
    version: "0.9.0",
    release_url: "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.9.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinymcp-0.9.0-archlinux-rolling-x86_64.tar.gz",
            sha256: "57a72dac5b7e69444ba10688d5e3516cb4d5c0760663a59774c94060952c744f",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinymcp-0.9.0-fedora-43-arm64.tar.gz",
            sha256: "6e5b4623debc566d1c1ed7dacbc1d81330e9d63723fcfd67753cb8af73e073cd",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinymcp-0.9.0-fedora-43-x86_64.tar.gz",
            sha256: "b5e27c097d3f694d40343ea61c4acc4b3aa0fed13630a9d63dcd339c00823fd0",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinymcp-0.9.0-fedora-44-arm64.tar.gz",
            sha256: "1908e9fa46bf3b0ecc3917c45e9b98064a0d162833c5331831b896a32526ff3f",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinymcp-0.9.0-fedora-44-x86_64.tar.gz",
            sha256: "7937d3a6bc549cffdfb70f89e21dc4a26057c0700ed2a02cb08cfa5252869655",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymcp-0.9.0-macos-15-arm64.tar.gz",
            sha256: "bceb6a3b50f78285e47d319951e92d279cc99da153dc660e671b6df9190dd197",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymcp-0.9.0-macos-15-x86_64.tar.gz",
            sha256: "0310c91e11dc44c93609e2003057a18b20919b0aee739e9b99ab8911350af334",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymcp-0.9.0-macos-26-arm64.tar.gz",
            sha256: "6083044b9e2b38733014570fa91068f74ac23120cc4ef790c934984bc1812640",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymcp-0.9.0-macos-26-x86_64.tar.gz",
            sha256: "6d8c90efb63743523fa901d74b0e8e6939d30bacd0a96d77650a1af61602ac9d",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymcp-0.9.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "53bbdc087a444e0deb1e770ad4ea9cc23f77ffc4db6072124da819543dfabfd4",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymcp-0.9.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "c1c1255bedd611ba32f70beb82c02452cba7dbe9a08de5d0fd3a2dde0216003d",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymcp-0.9.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "b1fb9348917a4f828768fd41e92fa7b52eb010cb90022cb3be66800c06a95857",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymcp-0.9.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "38681621198eb5bfaf170154c619ae8c3dcfae1d233ba82c2cad039d165fe557",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymcp-0.9.0-windows-11-arm64.zip",
            sha256: "6e8c65385acb52ff116ba08e29a895434114e46d3379a019953d7013bc22e38b",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymcp-0.9.0-windows-2022-x86_64.zip",
            sha256: "4a821cabaeba893ecb2358e3eef92e3392e18def07e9b59a4865ae1e8a232f57",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymcp-0.9.0-windows-2025-x86_64.zip",
            sha256: "4a18d8a76f9be30bf64224c5a2e7165f70716bd584e3b8fbaa88f5e70854905b",
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
