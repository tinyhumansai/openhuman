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
    version: "0.2.5",
    release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.2.5",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinydocs-module-0.2.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "7bef240f241cdc852871efa233454cd97febbbb7de6613c3de6c103e82b0fe62",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinydocs-module-0.2.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "f61a645d4105ae5ece37d0f19f5494fa73be3df061bbb81e827cbed91cf53963",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinydocs-module-0.2.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "a942436faa90766a805f1b10ed9a4797471c552ba52869eeb8882331a6db5ad3",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinydocs-module-0.2.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "b68565e8810c4cf7cedfef98f8421635b7b86828666ee1a799061404f6c43feb",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinydocs-module-0.2.5-macos-26-arm64.tar.gz",
            sha256: "d60ac5afe285857b687f1263f43716b27134ba309288a4b1849a7c6c0ac96760",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinydocs-module-0.2.5-macos-26-x86_64.tar.gz",
            sha256: "0feb2543a09a2f06a8b056bb8b728b28b0f2b86d85e1968e4113ff2a0d54475b",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinydocs-module-0.2.5-macos-15-arm64.tar.gz",
            sha256: "8352d968c5148980d945ea5db9e7229313528b15ffe9d7361a6d1ebbf251a7e4",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinydocs-module-0.2.5-macos-15-x86_64.tar.gz",
            sha256: "557023747a4b258a571776a653c71efba67099d4ae2bf4f52a521e150a3080c5",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinydocs-module-0.2.5-windows-2025-x86_64.zip",
            sha256: "a0b29c083a224a086ef4eacf64fbd6ac08565f1a71cdb7430976febf207ad69b",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinydocs-module-0.2.5-windows-2022-x86_64.zip",
            sha256: "bf8ec85f1f6255218452e009bb7b8c83be6557ec03f7721c6e08dd350d404ba2",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinydocs-module-0.2.5-windows-11-arm64.zip",
            sha256: "86007934b4bdc75a296116a5399027ea8af7efae25c7eac6bf35fbaf4e695f64",
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
/// The pinned v0.20.0 release (contract 1.14) carries the native wallet, Web3,
/// and x402 bus operations, including mnemonic word-count validation. Secret-bearing
/// operations remain confidential and require both attestation and a matching
/// pinned digest.
pub(crate) const TINYWALLET: ModuleRecord = ModuleRecord {
    id: "tinywallet",
    description: "Transaction building and assembly for Bitcoin, EVM, Solana and Tron",
    bus_name: tinywallet_bus::names::BUS_NAME,
    object_path: tinywallet_bus::names::OBJECT_PATH,
    version: "0.25.0",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.25.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.25.0-archlinux-arm64.tar.gz",
            sha256: "83e2ce705bdcff6d996ef7ace91d64576e52e4853d5bfb5076c01551c6011f0e",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.25.0-archlinux-x86_64.tar.gz",
            sha256: "7cfb6230fcbfb22b9614ff6d0c1416c24adcd3ff4f6fe3cf006cedf028ac295f",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.25.0-fedora-43-arm64.tar.gz",
            sha256: "24ac4bccd1329c38278ff2e063f6498ee327e8112c2e1dd0286c50aa0e4dbede",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.25.0-fedora-43-x86_64.tar.gz",
            sha256: "6bb86fef30e6ddc911513665b328aadb038e126463dfa8e9f983522b611e8903",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.25.0-fedora-44-arm64.tar.gz",
            sha256: "267fedcb7e32631f4d99de7de510c7b63fd357ae8d8a0b9f871808e8ee10a888",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.25.0-fedora-44-x86_64.tar.gz",
            sha256: "5b344bee2fef069cae6467549c69a5b6b9c4974ca11fd7488a389841b6fab4f2",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.25.0-macos-15-arm64.tar.gz",
            sha256: "be5311eb2ce8eca6d9a94b699c15ea109647677e08521173a15c6e89d5f1460a",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.25.0-macos-15-x86_64.tar.gz",
            sha256: "5526ed961a57b23202cc5667f5d2efea6786c42e136c718176dc2df7f032b6ac",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.25.0-macos-26-arm64.tar.gz",
            sha256: "dee094e31a0fce8319fe9944de6829f5fc63c6b6b45db0dddabf3ab176716d75",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.25.0-macos-26-x86_64.tar.gz",
            sha256: "947a510e1ee11aa0ce0b44fa47752fd145464bc43e588ebb768e4a85b9acefa6",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.25.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "41f9b85c2fc6f54eb30c04513689a64c38921bbf7197db21612b669cb84c7630",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.25.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "f2ebed9429a3bbbe2782d518006bf930ed066c3c198b7c4e70f6c9706286fe50",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.25.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "083afca7df12883563839d8fad3ca0a6e4f27e0056d1a3577b94c1ffe5c4be6b",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.25.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "e17ddb91e63d9fb44e9670c1a782e3653ebaf06261e77fb38e617c04fbb2f309",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.25.0-windows-11-arm64.zip",
            sha256: "bcf1585d5047ceb30c411ddabcde7ada87a63f7f42a96fca34be5fb0961e3587",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.25.0-windows-2022-x86_64.zip",
            sha256: "191941fc6676e141718a425600ff60cfba51c3035b9c90fe4a9b806a2e3d17fd",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.25.0-windows-2025-x86_64.zip",
            sha256: "150cc6367cee5bd8599c47007518f89a626162502ce06f5164ec55e5cdd7f328",
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
