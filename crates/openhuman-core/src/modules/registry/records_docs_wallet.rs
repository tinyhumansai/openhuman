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
    version: "0.2.8",
    release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.2.8",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinydocs-module-0.2.8-ubuntu-24.04-x86_64.tar.gz",
            sha256: "fe331ce5002fa9aa4ca0350a3797af931e02537a3c71aa422897b6f070f80097",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinydocs-module-0.2.8-ubuntu-24.04-arm64.tar.gz",
            sha256: "72dc75efeaa014189a6537923a66be8fff2542f2c25f69f3469a8652afe599d3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinydocs-module-0.2.8-ubuntu-22.04-x86_64.tar.gz",
            sha256: "ca64ac17081a431de3b5a586678d1f4f893664c85507ceafc4c94116f92ffc91",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinydocs-module-0.2.8-ubuntu-22.04-arm64.tar.gz",
            sha256: "5c47b41d165accf04b7725dcdbd88cee41db605e7889c9a4a78f6f2b99756b04",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinydocs-module-0.2.8-macos-26-arm64.tar.gz",
            sha256: "9e4dcc5ea6be35fe3dfd37dd84a767715c812fdf65233d5f5a3a32dea113090f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinydocs-module-0.2.8-macos-26-x86_64.tar.gz",
            sha256: "8c9d40ed25148c94b191448ad4ab82d5c2c975d41cae6cbd23555c07df5f1781",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinydocs-module-0.2.8-macos-15-arm64.tar.gz",
            sha256: "8e822acc2568cdc595ef2180a0701b6dcfecc63f46c3323a451c6f53f5ef5a70",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinydocs-module-0.2.8-macos-15-x86_64.tar.gz",
            sha256: "4907d66f5f43eba2fdf73159d67aac503572b62831fa82f77c967de30a8250e5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinydocs-module-0.2.8-windows-2025-x86_64.zip",
            sha256: "a57c597295ac47292f96b0fca39de7b2e4944f38771123eec240c8a15b7ff2ba",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinydocs-module-0.2.8-windows-2022-x86_64.zip",
            sha256: "e985d0bdaeee64fe1fb3819966063f6ecf6094080f4080c963a3f76bec364edf",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinydocs-module-0.2.8-windows-11-arm64.zip",
            sha256: "49f0f570cf59385f6c9f125bb064a9f08313f2f1f2402b43700ee5c0a192d686",
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
/// The pinned v0.29.0 release (contract 1.15) carries the native wallet, Web3,
/// and x402 bus operations, including mnemonic word-count validation and the
/// `NativeWalletTokenBalances` SPL token-balance read. Secret-bearing
/// operations remain confidential and require both attestation and a matching
/// pinned digest.
pub(crate) const TINYWALLET: ModuleRecord = ModuleRecord {
    id: "tinywallet",
    description: "Transaction building and assembly for Bitcoin, EVM, Solana and Tron",
    bus_name: tinywallet_bus::names::BUS_NAME,
    object_path: tinywallet_bus::names::OBJECT_PATH,
    version: "0.30.0",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.30.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.30.0-archlinux-arm64.tar.gz",
            sha256: "50eb1628fe7ad96111fcdbd09aa893451f7d6e19b6d5a1b63233967c1daafb0b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.30.0-archlinux-x86_64.tar.gz",
            sha256: "ffa384b7fa01bb8c288353de0ad50039557d7d6d55a969f3af6fae2a65d926d9",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.30.0-fedora-43-arm64.tar.gz",
            sha256: "6da862f1055c90af4d1713ea4ecee1a4e1fe9d5172c97e7b04f07f3e6a01195b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.30.0-fedora-43-x86_64.tar.gz",
            sha256: "cf57707e2798a60ba6ff2174d734c9e5317fd4a5f627d5015529763f42e4f7c5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.30.0-fedora-44-arm64.tar.gz",
            sha256: "805d1f76005b0942255ee132a40c65417d77618e27f58f12f860485d67c3a838",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.30.0-fedora-44-x86_64.tar.gz",
            sha256: "cca69dd1c96f74a7176e7cf9ce63fa6e6871752b1c2bcbb72e1fa44b876f85c6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.30.0-macos-15-arm64.tar.gz",
            sha256: "c11846bb1d59fc20037f93482c7f8a89ab597fcf77be8605fafd1792d2f634dd",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.30.0-macos-15-x86_64.tar.gz",
            sha256: "b08e40f1e3f2fa73ec8c5f3a0a7e22317723aee6663a6f4a49b5ab9260082c58",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.30.0-macos-26-arm64.tar.gz",
            sha256: "8a6838b9921d44044bae77c2c711299f5ed2959d54f7417e1f88a96a7e916f9b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.30.0-macos-26-x86_64.tar.gz",
            sha256: "2368ccf882122bfce205960ea84a78ed89d2b274f8512c714f4c1ed23b5bd500",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.30.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "14dccd485fdf6679e3e5f0829cd87560aabeb230b8a340c2cd015719344c6c58",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.30.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "06fe9666e4c839676f2ead3aa1a4b3527a2d8b499c946f632cd1cc5eda6c9dab",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.30.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "8c4fe25c0d605c36c936f28286f2b91beeecccbf948746dffb422b59648479ad",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.30.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "df654794d21da8cc3e6447e2e944fb39f127918609f7c81a49a3dddbea821e7c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.30.0-windows-11-arm64.zip",
            sha256: "7dacf69ed8f9b5ca02f0930f4c80d01c445661ac7a43a3440aef1714f9f670fe",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.30.0-windows-2022-x86_64.zip",
            sha256: "46ebd65cb38650cc4eb697d1fc4c96e5cdea692a9e63f4be6fa7947b75f64535",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.30.0-windows-2025-x86_64.zip",
            sha256: "57e32f30e61f5d0ed3f32a3b5bb072eaaf253c575cea719abe63bf7c529f506f",
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
