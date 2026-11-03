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
    version: "0.20.5",
    release_url: "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.20.5",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-rolling-arm64",
            archive: "tinymcp-0.20.5-archlinux-arm64.tar.gz",
            sha256: "2bbc95f522647321d33106d1527e4c3195e920c31ba2416f435d43676939bb1a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-rolling-x86_64",
            archive: "tinymcp-0.20.5-archlinux-x86_64.tar.gz",
            sha256: "1aaa41c55dda694886b6c045f6795c26786c4936888f39a8fdef225125e09d6c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinymcp-0.20.5-fedora-43-arm64.tar.gz",
            sha256: "c7028fe80ff726b97589beeefe3aeea003e1490fcdccd707caa6fdc84785013e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinymcp-0.20.5-fedora-43-x86_64.tar.gz",
            sha256: "d9cee4abc57e6321ca664cee5ced27b9a310658d48cc423ffece55ac857e0c61",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinymcp-0.20.5-fedora-44-arm64.tar.gz",
            sha256: "6a44d66ad55ffec2d53e0cd706849cd7474bead7a9aee7d649404a4da8c52692",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinymcp-0.20.5-fedora-44-x86_64.tar.gz",
            sha256: "a6c2853bc344377619290c64ffa32a1a9f9b20ed74356db8ee492e0aa944fd4e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymcp-0.20.5-macos-15-arm64.tar.gz",
            sha256: "a037577bb462b98862dd3d76b52d5fdac05e1baaa46644db98070c2b213c3310",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymcp-0.20.5-macos-15-x86_64.tar.gz",
            sha256: "a8611f7b9858f1a97eac413a38a1d4d3dd9c27bffaf48f93d744365f66be3642",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymcp-0.20.5-macos-26-arm64.tar.gz",
            sha256: "9e079804323e8421dc2924c6a88d551e4baf867b5d865e0122f478238a47025d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymcp-0.20.5-macos-26-x86_64.tar.gz",
            sha256: "a501ff4f7e70012bff07e8ab0a1c4ff72f98b6e08330e900a8bd9fe64fb949b0",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymcp-0.20.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "a134d7401747314cf3a0a1cef5d749e6a74397d115976b7dcc6c1aabc27bef6d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymcp-0.20.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "a59292015abdad2f9511b8ba4befccfb50a48e09a694fb4835fe82899f96889f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymcp-0.20.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "6b37bcc8a3c597981cbb021b0a484629bb757e0bdd14cc056aeb111506f40d51",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymcp-0.20.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "3a240f35b6adf00107db204e96b3ba9286c9bc1bf7125a47059321dc2d9ef26b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymcp-0.20.5-windows-11-arm64.zip",
            sha256: "2d72ebb7199b0a85e6b7de3386e3ad42e9ee89a6f15eac372fba534f2379517b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymcp-0.20.5-windows-2022-x86_64.zip",
            sha256: "f184c044b726394461b0ef0a53c18bf6bf35b7ebb50302101b4086dce3941a47",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymcp-0.20.5-windows-2025-x86_64.zip",
            sha256: "8e6412b981599e8676fb51444f23898d9017972a6aed42137232aaff4f7c9754",
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
    version: "0.22.1",
    release_url: "https://github.com/tinyhumansai/tinyconnectors/releases/tag/v0.22.1",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinyconnectors-0.22.1-archlinux-arm64.tar.gz",
            sha256: "91d5a9c342245f5c8c3bcbe3bffeb1622df0b2838646bb2c43890de35af684a3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinyconnectors-0.22.1-archlinux-x86_64.tar.gz",
            sha256: "5215fcf05382c9c0e96d0696849a56d81d16644684aa23885a0bb98f3b17626f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinyconnectors-0.22.1-fedora-43-arm64.tar.gz",
            sha256: "9574021fde72759ddba04adaa07acff4a67130e7e093fb3fc02d899ea3793498",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinyconnectors-0.22.1-fedora-43-x86_64.tar.gz",
            sha256: "39a1ef3420ff2084124659e344eb3f758485c314e6a53ae4d5c9cafdb9d288c8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinyconnectors-0.22.1-fedora-44-arm64.tar.gz",
            sha256: "bbe6472272efd0bec44976ab4ed7eaed04c4bde931db23d9386f7e85ce66fef8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinyconnectors-0.22.1-fedora-44-x86_64.tar.gz",
            sha256: "499de6644a213cd17ac73e8e83b7c5ce44e5f39dd3abfed12e5f0a67232bb09d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyconnectors-0.22.1-macos-15-arm64.tar.gz",
            sha256: "2c95a98e0fdd45ac31640d939a9c6a7d18e8fdd9d03fe43dcbf17ed20433a00e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyconnectors-0.22.1-macos-15-x86_64.tar.gz",
            sha256: "fd1cffc2480468cb3acbeee22d574035c7b671014e306c375d2a830411827d19",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyconnectors-0.22.1-macos-26-arm64.tar.gz",
            sha256: "c1d22420d9f0986423f24271df7b5ae93221ea322123b041be6dd3cb4081d373",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyconnectors-0.22.1-macos-26-x86_64.tar.gz",
            sha256: "7a8c5b00b84ba5f73feada42ffd3f68ad117c10fc5324208e5c4bcc705b03532",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyconnectors-0.22.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "37dad4d685ee8c055f26a93693447137c9343b1c3d3fd00c61db771c60f2097e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyconnectors-0.22.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "9ab94e32d65a0ca411ceb0f5567f895e5255d7c64a2502e7d7788bec0ba59c93",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyconnectors-0.22.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "c980f3659fa79c1401e1a5ecd95c058e0a11578bea5f8a258d803a5b0ee61b5e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyconnectors-0.22.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "3a34cdadfd783178d8db2ac2af3b89994e4f73087d321fd1c353b763262c7d9a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyconnectors-0.22.1-windows-11-arm64.zip",
            sha256: "ced31db483eeabf5f799789ddcb0b073209495620b52959e05952d64b1f06f33",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyconnectors-0.22.1-windows-2022-x86_64.zip",
            sha256: "3c998167fc1d4356f8977b5e3e8aa2b0e54ff0b1e1092b577de75f591e8d08bd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyconnectors-0.22.1-windows-2025-x86_64.zip",
            sha256: "34f14dfe26b18462be18b805a04359b3a7635d65795c1fd0d352a91d80d186df",
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
