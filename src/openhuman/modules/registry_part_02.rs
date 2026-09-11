
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
    version: "0.9.0",
    release_url: "https://github.com/tinyhumansai/tinyconnectors/releases/tag/v0.9.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyconnectors-0.9.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "4c9d8d6bcfa55fa5dd2a9d42c48d5be6c039357ca2b0e2519bb6018843ae2eab",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyconnectors-0.9.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "da7a3281e935f69575cb79b1a7a4cd2243a010655114a2e9b4f6d56fd2f6ca58",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyconnectors-0.9.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "375a03a9cad4ed0b8687d1601cefcedce8db0f183bfd66b32bf7531945d6a9bb",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyconnectors-0.9.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "43512f888b2fe302c8ddc9e36f1eaa755f35261f1270045b50ea9d58d53572b1",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyconnectors-0.9.0-macos-26-arm64.tar.gz",
            sha256: "60c9facc643bd306899e43b88b922c0ee51bb02dfe8d49aff7022f44a5f279e1",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyconnectors-0.9.0-macos-26-x86_64.tar.gz",
            sha256: "99759d644e5cd2818146a67c976a26e0c80cf05bd0b55faa063b4cc3842c89f9",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyconnectors-0.9.0-macos-15-arm64.tar.gz",
            sha256: "0aa345d7d5fe3d0bbe095d865c22427e61d1f6f2b6413d7f47456a16a1eb056c",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyconnectors-0.9.0-macos-15-x86_64.tar.gz",
            sha256: "ad2d6590b00225512f5bdd0d96b33bb7adacde709fe788d260837bf619bfa343",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyconnectors-0.9.0-windows-2025-x86_64.zip",
            sha256: "55dfc1f709b6f027688bfd043b9edfcfe000fa20aaf7d429f92cc6bc02233737",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyconnectors-0.9.0-windows-2022-x86_64.zip",
            sha256: "fc24cc1cf77e9e97f6f1ae21976d945cd5ee6c8e6edce656b6d676c423140ff6",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyconnectors-0.9.0-windows-11-arm64.zip",
            sha256: "f5c7cede5b8425119bee391c9470053213f23c9f7e406d2d2ddc76b70549d99e",
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
