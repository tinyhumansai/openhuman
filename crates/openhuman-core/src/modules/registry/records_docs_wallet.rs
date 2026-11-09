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
/// The pinned v0.26.0 release (contract 1.14) carries the native wallet, Web3,
/// and x402 bus operations, including mnemonic word-count validation. Secret-bearing
/// operations remain confidential and require both attestation and a matching
/// pinned digest.
pub(crate) const TINYWALLET: ModuleRecord = ModuleRecord {
    id: "tinywallet",
    description: "Transaction building and assembly for Bitcoin, EVM, Solana and Tron",
    bus_name: tinywallet_bus::names::BUS_NAME,
    object_path: tinywallet_bus::names::OBJECT_PATH,
    version: "0.28.0",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.28.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.28.0-archlinux-arm64.tar.gz",
            sha256: "da90ff7b53c2a229896d5a4ab0e3121727488f37a78551b3207a9c78bc4a09a6",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.28.0-archlinux-x86_64.tar.gz",
            sha256: "1e80c400e50643d938e25b45aa0327de170f66f39fe65645a9d5ccb31b5f4a88",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.28.0-fedora-43-arm64.tar.gz",
            sha256: "fb2b630ae47e64bd10a975861e647a939bfaa55b664c8f0706cd096779d62c3b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.28.0-fedora-43-x86_64.tar.gz",
            sha256: "84cc92b8775d26def86ad5073ed7315a50aae4b5aa7134a1cf485b923a32c38b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.28.0-fedora-44-arm64.tar.gz",
            sha256: "14a1f4a044a7e5b246aa2df7209c31c165bf1305b38d888a540c2e438c11e202",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.28.0-fedora-44-x86_64.tar.gz",
            sha256: "d550966866d4b3446abef6f3b19fcf6968d914579a445c8b08fff276465b9029",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.28.0-macos-15-arm64.tar.gz",
            sha256: "c8c791b7464f1db2c6395f286cf63727b24bacb7606e318bb7dad5fa120387e1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.28.0-macos-15-x86_64.tar.gz",
            sha256: "b72c5f5b979302599b3a4f85b6783bd753c119e23ade819c0b1edf9ffd01d729",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.28.0-macos-26-arm64.tar.gz",
            sha256: "b8d189befa3358f854346f3903187ef23b990b71d314a4a58539531c9212ca82",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.28.0-macos-26-x86_64.tar.gz",
            sha256: "9eb8990e3401b8c6d30ca08a434c64f56ff82e6bc1b6e160d27aae4d30a8c24f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.28.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "c753826827230d2ebcb044eeae91f96d1f08d945f1ef37b26030ae0e0843d60c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.28.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "02cec7d7e33112266b988be1bab6213b71193f7a183db8783dba941e62ef7b46",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.28.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "5b8dc519829098029609e224b95b2149a71cdaad240148e33c0abac71ec5dc07",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.28.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "c2bb2acd7c15b061b66d564db738aa3eac6802544fba753a6715ade541faac86",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.28.0-windows-11-arm64.zip",
            sha256: "02598b083ce4d7e66c6d295785b5b42535861614741b2371462847760bbd2823",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.28.0-windows-2022-x86_64.zip",
            sha256: "fdf15c140e0de0492fb0fde449e38e8eef0653cc7af6ebbe71c56218f5d33c17",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.28.0-windows-2025-x86_64.zip",
            sha256: "a89031486e2aeb8f2f96fc314a5a291ebc4364e3b34dcdcd6a010b1a939b365e",
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
