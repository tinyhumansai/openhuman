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
    version: "0.26.0",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.26.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.26.0-archlinux-arm64.tar.gz",
            sha256: "b59ec08e44abf5079a19978b8adc3f6f2514a2bd8925c20429d554fe610a4c7e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.26.0-archlinux-x86_64.tar.gz",
            sha256: "b15f03b5c6ed054c79cdf6518c1c6674cfced37556f52d189d30ad86814090e5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.26.0-fedora-43-arm64.tar.gz",
            sha256: "f1be27c80bcc0cf93846ab12b9ba1151d17752875cb6603c23da1ee63d9b8a95",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.26.0-fedora-43-x86_64.tar.gz",
            sha256: "f22abcef72703f4ad3594f33eff9c16d3aa9850aefe2457635fab9f42b7add9f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.26.0-fedora-44-arm64.tar.gz",
            sha256: "39a5517ee0be1b3e8614f8691c5deb0b51817752007bd74dd781d877dae8313b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.26.0-fedora-44-x86_64.tar.gz",
            sha256: "a48390bf558d1b14147790d880b792cf5f0e1aea7c79d38217f7d7ea53a2bc9d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.26.0-macos-15-arm64.tar.gz",
            sha256: "04bc74a5187ac62b7416d88f847f9aa4b49914b8da864f82a19305fb43ace6f7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.26.0-macos-15-x86_64.tar.gz",
            sha256: "526119531f5132973c620ebceaf21e237cc39306a4638a31e88239b0816a6122",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.26.0-macos-26-arm64.tar.gz",
            sha256: "ad070c81c067578516e76c56edf68256ebb063826c98bfbcce2db16f2ab3ad0c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.26.0-macos-26-x86_64.tar.gz",
            sha256: "6c784834d21b3b0a94623f84dd3a132efd499788db8ae46844d7e3b42c0b4512",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.26.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "bd404a0beecf749f6293685d3799abdf1ab701ec8b4fe6625bc7a14d9c890b79",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.26.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "87b7c1f9a76db09ab0dcbda1b02caef22e355c5c731bba7b53f8571aaa6a19fb",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.26.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "9752e813ef204ae59759b8fee54cafe2b38b8b120dfe0ec451e28a7d20b9fdef",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.26.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "90bf726fb55fc0a6cd47e2e8a838d2e2ee7bf5217de19450a88479a089b85a17",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.26.0-windows-11-arm64.zip",
            sha256: "6eb5c829b6a38ae6732475b19d0bb7c106dc490ba55a0365208976c96eb275a2",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.26.0-windows-2022-x86_64.zip",
            sha256: "ca46336d7cad9d9728f0a7de58d69d2755bda23eeed617481c543102a1fb99f4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.26.0-windows-2025-x86_64.zip",
            sha256: "c66b49e30adafe29033823b823785a68121ca6177b12a9ec1b989f1c68f2bde4",
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
