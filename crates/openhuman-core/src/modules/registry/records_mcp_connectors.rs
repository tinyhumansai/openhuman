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
    version: "0.18.5",
    release_url: "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.18.5",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-arm64",
            archive: "tinymcp-0.18.5-archlinux-arm64.tar.gz",
            sha256: "0a3760807e225282677caba6a157e5c6cc16cfe509d22a8acc372a9a4c77f9d1",
        },
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinymcp-0.18.5-archlinux-x86_64.tar.gz",
            sha256: "84f3bb050943bc040204b316c3540e31b25457b4f36112f0248f3a2bbd3f85c5",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinymcp-0.18.5-fedora-43-arm64.tar.gz",
            sha256: "fae7790e4df4418eb8fff18818f5df04d69e2c307d3613b0fc177ad0d1810804",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinymcp-0.18.5-fedora-43-x86_64.tar.gz",
            sha256: "b8fe422717db38f63619bd108b2b63e9a3f3d40e8cb2ab653076d76d55f8bcf8",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinymcp-0.18.5-fedora-44-arm64.tar.gz",
            sha256: "cc3f6cf33631e28921dd4483dd390988ab9a86806b0ce26274fc92fe0c56c53c",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinymcp-0.18.5-fedora-44-x86_64.tar.gz",
            sha256: "f92f83766f9f277ad8a8c4c9844cda5b1bcabc6404d8a133153cfeb941454f87",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymcp-0.18.5-macos-15-arm64.tar.gz",
            sha256: "c706d86b21edcd0bfd6a9b01ac055f475904ba82a003be4dbdd8c715237901b9",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymcp-0.18.5-macos-15-x86_64.tar.gz",
            sha256: "b675c9777403c7215ce850ec79cdf3607d244631a0dd20eca7be4967455e548e",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymcp-0.18.5-macos-26-arm64.tar.gz",
            sha256: "1296c0c405553eb30e4682dc304e657ebab2b7856a49f684e5a7aac6a287658b",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymcp-0.18.5-macos-26-x86_64.tar.gz",
            sha256: "4c34fb291190b4317ecf895869c38baa14689693e89d9af344642a222d683157",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymcp-0.18.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "404ec212c3b3143308ac7f892f558fbd1440e863e1f5a7cfeeb19d54c2bfcfad",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymcp-0.18.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "bf569fb43def4f743c420701a4e9e39fc64679e1dceadac9e87894135d1b5013",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymcp-0.18.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "b1525e225a1481600624bd317c01cb2c13ccc4034860103fe775ff6ecda40803",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymcp-0.18.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "f8c0b9dfc9a3a4b98314f5acf298c61b5ef96b3ce8605bad923a6bd642aefde6",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymcp-0.18.5-windows-11-arm64.zip",
            sha256: "305bb0401e92752672c4db1eecedadf2c6bb720a868ab2316f97287048081b49",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymcp-0.18.5-windows-2022-x86_64.zip",
            sha256: "6457801f2919cfe5e47dfe3577f3f163ad3f7567a814f99e51a37dee254f9c86",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymcp-0.18.5-windows-2025-x86_64.zip",
            sha256: "2c2e2487c6b3fff2975df7b5928e688c46e42b9260afef02f72a0a77e1bc118a",
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
    version: "0.19.3",
    release_url: "https://github.com/tinyhumansai/tinyconnectors/releases/tag/v0.19.3",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyconnectors-0.19.3-archlinux-arm64.tar.gz",
            sha256: "1b223604c366769a67d0345d12da9b1b679722b54062157c68fba08b608f3ec9",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyconnectors-0.19.3-archlinux-x86_64.tar.gz",
            sha256: "b7c83dbf1a4f40a343fe687708412f5b33c137cf384ebea5d8b9b6223d890f58",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyconnectors-0.19.3-fedora-43-arm64.tar.gz",
            sha256: "386552936cc9964ec4a006664656fa1ac58aaccd104dbf8fd71626824805dac4",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyconnectors-0.19.3-fedora-43-x86_64.tar.gz",
            sha256: "7d810750a28d64ff332448095c91e57064da1dfdd70028ac378c86c88df8ca2c",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyconnectors-0.19.3-fedora-44-arm64.tar.gz",
            sha256: "a7094a70e28f99dbe4312be4c0d5b49337b5b35e605bd07c7dde323e247fcdf6",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyconnectors-0.19.3-fedora-44-x86_64.tar.gz",
            sha256: "39c4938e9daba8643910e2b4f8573c0559d1cf8a15b44bd99e615a9dab926081",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyconnectors-0.19.3-macos-15-arm64.tar.gz",
            sha256: "644a1c58c2ab7911ea2808c23b75b96e3ae312af1d12f97cca55f71971f551b9",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyconnectors-0.19.3-macos-15-x86_64.tar.gz",
            sha256: "53297c0fffad47ac19dba05a9e360af1601cf4de779e7db0d9e06d2059e91099",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyconnectors-0.19.3-macos-26-arm64.tar.gz",
            sha256: "96290a9f9b9b07fffbe3f333478002d7f46e5c2a563542d73ddac47420e597a4",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyconnectors-0.19.3-macos-26-x86_64.tar.gz",
            sha256: "990a0414fb4a2fc834efeedcd5e4e51e7057aed928f9c51aa4fae2c4a43a7f0d",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyconnectors-0.19.3-ubuntu-22.04-arm64.tar.gz",
            sha256: "f5ab8ced9095150df3ab0f3b84b4fe505a61d1fec64de535355d59a7ee13954f",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyconnectors-0.19.3-ubuntu-22.04-x86_64.tar.gz",
            sha256: "26daaa208a624c1afc5520fdd3d715eca032b956324ad5918431171097381620",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyconnectors-0.19.3-ubuntu-24.04-arm64.tar.gz",
            sha256: "ded63f30313e3196c0f75e6c6d211920d799df5e39b4d0657c4901de419db96e",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyconnectors-0.19.3-ubuntu-24.04-x86_64.tar.gz",
            sha256: "15cfa5718aeabe2882503c618e22c5067f050fbc3aaa4be31e541071700900f5",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyconnectors-0.19.3-windows-11-arm64.zip",
            sha256: "02a7e82e452c9190db68b5f2eef49b6fad73bf067dbb8f13ab3688c003dedf61",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyconnectors-0.19.3-windows-2022-x86_64.zip",
            sha256: "cf0d99f72732d934b24428cfa2ef414a649d41c612de330ea76af4e037641822",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyconnectors-0.19.3-windows-2025-x86_64.zip",
            sha256: "271bd3855a1565fedd041a02bbca2555d418bcd16d764b55385f6896d8e9682b",
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
