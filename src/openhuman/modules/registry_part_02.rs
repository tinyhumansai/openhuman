
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
/// remote tool definitions, the `mcp_clients` / `mcp_setup` RPC surface, the
/// agent-facing tools, and the proxy *scoping* decision all belong to this
/// application's threat model, not to a protocol client. `tinymcp-bus` carries
/// the vocabulary; this table says which bytes may speak it.
const TINYMCP: ModuleRecord = ModuleRecord {
    id: "tinymcp",
    description: "Model Context Protocol client: transports, registry, and the write-audit log",
    bus_name: "ai.tinyhumans.tinymcp.Mcp",
    object_path: "/ai/tinyhumans/tinymcp/Mcp",
    version: "0.3.2",
    release_url: "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.3.2",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymcp-0.3.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "8bb03dcec777fbd52fedf678dafc04e44afeabc453b3459aace76e721bde7450",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymcp-0.3.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "cdb06140a3d763c6137dc8470a6896f30707909bc6ac896088391fece220e284",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymcp-0.3.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "879de1fb22e4b0b9383638ef00d207ed580a23c6b1fbc85a96b9d405c7e4273d",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymcp-0.3.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "324a448f1fd3b564f9c3892fe48f96415cd1c3a33f2c234e3c805410136fe7e2",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymcp-0.3.2-macos-26-arm64.tar.gz",
            sha256: "dd952d4bdf865e9a8b5b358267f7f0c0895d15e9d657c5fc82f15f48f0b281eb",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymcp-0.3.2-macos-26-x86_64.tar.gz",
            sha256: "11d284c1f9b194c5ac3865656e19b4d4ed3ca91a70e28359f14d13ac73101b1c",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymcp-0.3.2-macos-15-arm64.tar.gz",
            sha256: "fc86f823719d305de6abc321d88a5b455517c4a6945135af15f7fbc2a3fca403",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymcp-0.3.2-macos-15-x86_64.tar.gz",
            sha256: "7aec3ab842a7b2c6162d98021873416705b0da5c685ae0c0d8d3792684c0a530",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymcp-0.3.2-windows-2025-x86_64.zip",
            sha256: "d0defc7df1f4bf4084ebaa1c373316e44f51d27f0ce32b35ac26937905fddda1",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymcp-0.3.2-windows-2022-x86_64.zip",
            sha256: "71a35710fa45dc07c4f3d24074a189e5cd2ebff678276a57c7b25d907353fe3e",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymcp-0.3.2-windows-11-arm64.zip",
            sha256: "cd31640774b27adf0472aaf0ad43f3a3c3e9d1b716624b40816c71240abdfa9e",
        },
    ],
    load: LoadPolicy::Lazy,
};

const TINYCONNECTORS: ModuleRecord = ModuleRecord {
    id: "tinyconnectors",
    description: "OAuth connector integrations: accounts, actions, triggers, and record sync",
    bus_name: "ai.tinyhumans.connectors.Composio",
    object_path: "/ai/tinyhumans/connectors/Composio",
    version: "0.7.1",
    release_url: "https://github.com/tinyhumansai/tinyconnectors/releases/tag/v0.7.1",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyconnectors-0.7.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "31f0cfb402b0788b59d35a9f40d8a1ed514f9da0084259f61a58c03e4fe0d8ff",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyconnectors-0.7.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "137e37be064e764585750f90dc310ea1c374aa454f46db73546a14355fc0cc97",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyconnectors-0.7.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "ca18131395fa146dc30cd662dee62b986c2a4b4bbdaa99ba17c9f62e628102cb",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyconnectors-0.7.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "828878f7012897ac85b35a6caaabb5ad867aaca267afb7d15c2bd17d4a4fd907",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyconnectors-0.7.1-macos-26-arm64.tar.gz",
            sha256: "64d057a49178863a2e4289b890117bd921dc915d6044e69786c90aca19878287",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyconnectors-0.7.1-macos-26-x86_64.tar.gz",
            sha256: "4a908f1b598634bca38adaba93272ae1bd4b2b30b6472cf0f551bb36fd0637d9",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyconnectors-0.7.1-macos-15-arm64.tar.gz",
            sha256: "6c1b8ed910fd1d14bb570c4df6dbe6976605aa9c74b4551e867ace87e605520e",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyconnectors-0.7.1-macos-15-x86_64.tar.gz",
            sha256: "f0a13813fe460e6a8cc359b0cee6b8a854bb03c9fe0acc96c9d190a64035a5c2",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyconnectors-0.7.1-windows-2025-x86_64.zip",
            sha256: "71356fb76f975736e2b7a74961eef4a2ad59842265575bfbfdb7e576836ce9df",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyconnectors-0.7.1-windows-2022-x86_64.zip",
            sha256: "5740c96c0a546f133e48ef3409550d17aee2fb4877bbd799ba1954867c29ba6a",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyconnectors-0.7.1-windows-11-arm64.zip",
            sha256: "f63002ffeaa7dd6c7b0fa096ab2e175bfba2d26d23eccef8b0bbd54e00171b4e",
        },
    ],
    // Lazy: a user with no connected accounts should not pay to load it, and
    // most sessions never touch a connector. Safe even signed out — the module
    // loads without configuration and still answers the capability members.
    load: LoadPolicy::Lazy,
};

/// Every module this build can load.
pub const ALL: &[ModuleRecord] = &[
    TINYDOCS,
    TINYWALLET,
    TINYMEMORY,
    TINYJUICE,
    TINYVOICE,
    TINYRUNTIME,
    TINYRUNTIME_NODEJS,
    TINYRUNTIME_PYTHON,
    TINYMCP,
    TINYCONNECTORS,
];

/// The record for `id`, if this build knows it.
#[must_use]
pub fn find(id: &str) -> Option<&'static ModuleRecord> {
    ALL.iter().find(|record| record.id == id)
}
