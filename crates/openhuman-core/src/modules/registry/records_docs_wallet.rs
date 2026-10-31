//! Registry records for the `tinydocs` and `tinywallet` modules.

#[cfg(any(feature = "documents", feature = "web3"))]
use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset, SourceSpec};

#[cfg(feature = "documents")]
/// The `tinydocs` module: document synthesis, bounded extraction, image facts,
/// and PDF rendering.
///
/// Lazy, because a user who never asks for a document should not pay a download,
/// a `dlopen`, and the resident cost of a library that is never unloaded.
pub(crate) const TINYDOCS: ModuleRecord = ModuleRecord {
    id: "tinydocs",
    description: "Document synthesis, Office/PDF extraction, image inspection, and PDF rendering",
    bus_name: tinydocs_bus::names::BUS_NAME,
    object_path: tinydocs_bus::names::OBJECT_PATH,
    version: "0.2.6",
    release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.2.6",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinydocs-module-0.2.6-ubuntu-24.04-x86_64.tar.gz",
            sha256: "82b40886299ef4633fffb808ba7119b5ac3850dfcb0d6b31e2512c6f1dfb64d7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinydocs-module-0.2.6-ubuntu-24.04-arm64.tar.gz",
            sha256: "456447ed7976612d84745ee666f11f86f7c16fc77a5e80b9789b6abd4541987e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinydocs-module-0.2.6-ubuntu-22.04-x86_64.tar.gz",
            sha256: "23a23de90f748a70345b90d2e653d8df8dfe2cef1aef8ce019ff9a35b19be093",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinydocs-module-0.2.6-ubuntu-22.04-arm64.tar.gz",
            sha256: "2e4ff859b2e04c449d35403303cf3914eba8cceb67949153b2d6ad6a81ab0858",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinydocs-module-0.2.6-macos-26-arm64.tar.gz",
            sha256: "ae92d9a9311526e4a55387ad86a4954ab31e5e0cb8200eba67185298d26dcf63",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinydocs-module-0.2.6-macos-26-x86_64.tar.gz",
            sha256: "7d2bda6f1da48409329b03524dd48eb247b1beffb6082b3a9a1872b7f71faab9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinydocs-module-0.2.6-macos-15-arm64.tar.gz",
            sha256: "7eeba8deafb5ae22c2d02f2e40006f1eabe62e3770b0276f0929eff714a16758",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinydocs-module-0.2.6-macos-15-x86_64.tar.gz",
            sha256: "8ea25137471da5515333091a63117bf080b12c506993a1993b613ddabd5d956f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinydocs-module-0.2.6-windows-2025-x86_64.zip",
            sha256: "3b1e1c370a249fb61bc262c64d8b9720b1ec327a1211917451056af02d1820f2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinydocs-module-0.2.6-windows-2022-x86_64.zip",
            sha256: "8e2f7725dcb6021eae0b2c2067d9a1fede012183082d3c1f86b4d491cbdc7069",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinydocs-module-0.2.6-windows-11-arm64.zip",
            sha256: "8f3d9e49a4ee40430a21355d891938adc4aecfa4c6592dd51dd695df0d1bc721",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinydocs",
        manifest: "crates/tinydocs-module/Cargo.toml",
        package: "tinydocs-module",
        lib_name: "tinydocs_module",
        features: &[],
    },
};

#[cfg(feature = "web3")]
/// The `tinywallet` module: native wallet, Web3 quote, and x402 operations.
///
/// Lazy for the same reason as [`TINYDOCS`], and more so: most sessions never
/// touch a wallet, and this artifact carries `bitcoin` and a native `secp256k1`
/// build that would otherwise be resident for all of them.
///
/// **This host sends it the recovery phrase, over confidential calls, and never
/// derives or signs itself.** All four chains — Bitcoin, EVM, Solana and Tron —
/// derive and sign inside the module. Web3 quotes and x402 payments, budgets,
/// and ledger state also live there. The host supplies credential custody,
/// authenticated transports, and exact-facts approval callbacks. This binary
/// depends on `tinywallet-bus` for the wire contract rather than implementation
/// crates.
///
/// The phrase is only sent to a module tinybus has attested *and* whose digest
/// matches one of the entries below — `super::wallet::attested_proxy` checks
/// this table itself rather than trusting that some check happened.
///
/// The contract also exposes `ExportKey` for downstream hosts that must drive
/// a signer locally; OpenHuman itself does not call it.
///
/// The pinned v0.26.0 release (contract 1.14) carries the native wallet, Web3,
/// and x402 bus operations, including mnemonic word-count validation. Secret-bearing
/// operations remain confidential and require both attestation and a matching
/// pinned digest.
pub(crate) const TINYWALLET: ModuleRecord = ModuleRecord {
    id: "tinywallet",
    description: "Transaction building and assembly for Bitcoin, EVM, Solana and Tron",
    bus_name: tinywallet_bus::names::BUS_NAME,
    object_path: tinywallet_bus::names::OBJECT_PATH,
    version: "0.27.0",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.27.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.27.0-archlinux-arm64.tar.gz",
            sha256: "739b73d4fe246b108d3d8c15b96e358219e3357554663054bde9b4cfcc1e1547",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.27.0-archlinux-x86_64.tar.gz",
            sha256: "30ad7f2de3d10d584d677412cd1fa6fc124083fb28ad5c58fcdb63a8c3d88f79",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.27.0-fedora-43-arm64.tar.gz",
            sha256: "e2a185ff78c0f636a1ccbc7a5d6d91e23121394fa5d64683058f33e24b414771",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.27.0-fedora-43-x86_64.tar.gz",
            sha256: "28f03bfde6eb63dde25e0f6fafaf2ce25e453afdf56a5c9a3277d47bf2fe5400",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.27.0-fedora-44-arm64.tar.gz",
            sha256: "574cf3773f405e282c3d42f4fb8ba8e3037e7abcd06ee63c719a134c100f9748",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.27.0-fedora-44-x86_64.tar.gz",
            sha256: "af05c0582364306a0bf885df059bea601044cfd40c40c7a14af043499053dc71",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.27.0-macos-15-arm64.tar.gz",
            sha256: "554375d3e45eb9332454323937c7923e34a75934a99b2185bff35503b7abeec5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.27.0-macos-15-x86_64.tar.gz",
            sha256: "ad4a098b29dedd350bd60bf4d0194fa89cc2f79bff47447f7d52b440f702730a",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.27.0-macos-26-arm64.tar.gz",
            sha256: "762c4fa1a283df5d499610314a6d35b002663e00f4b57105d56047a5d0cb83a8",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.27.0-macos-26-x86_64.tar.gz",
            sha256: "5973882752b659ac5ef9906fcc418e55796e3cc1375b3be141e16a624bc666c1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.27.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "927227ee8d8a23a12966ccb2de972fcab07e80fb6527dad840170e4ffd64f087",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.27.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "a031e81172452657a506cd769a41f8702c06d80a75b72ebf6dc9a0f4a8317c25",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.27.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "6e055459e513707e58b4d07c836a3659a453c3753ee8c632e858930c150b66f4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.27.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "cab0be708a52e8713df7cc57dc13717e3899ce3c8ec57819741b37dc463ccfc7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.27.0-windows-11-arm64.zip",
            sha256: "9e458762f740e166b9b028336b6faed3762dfb71bf46eb57c71ad7e3ec2dc9d3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.27.0-windows-2022-x86_64.zip",
            sha256: "ff389c8951c0721b31bb8eb4a71e86424f053923eab10464c8ee68bbb1ec25a1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.27.0-windows-2025-x86_64.zip",
            sha256: "b01f58d66e73f73b510bf755fb89d1fa77a5caad00033405511ce12b84086632",
            library_sha256: None,
        },
    ],
    load: LoadPolicy::Lazy,
    source: SourceSpec {
        submodule: "tinywallet",
        manifest: "crates/tinywallet-module/Cargo.toml",
        package: "tinywallet-module",
        lib_name: "tinywallet_module",
        features: &[],
    },
};
