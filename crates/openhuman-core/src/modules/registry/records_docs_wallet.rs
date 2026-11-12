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
    version: "0.29.0",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.29.0",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.29.0-archlinux-arm64.tar.gz",
            sha256: "90d8cb352a9d6b6403549f053446139c0361d36354ca7c37e0bc0296eb544cae",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.29.0-archlinux-x86_64.tar.gz",
            sha256: "8e9bc6e2d443807cbbb1759cb85896abbcc0d3294a0ec58007fe0c56aecb5a25",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.29.0-fedora-43-arm64.tar.gz",
            sha256: "71e8ec37af5be258c111169e441fed5f4a46d9b3934c477569ec1185ec7ceba5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.29.0-fedora-43-x86_64.tar.gz",
            sha256: "3e7438212776adbe82a54a5c48ff744cd26ee73d0dee66bce7de42b968608a16",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.29.0-fedora-44-arm64.tar.gz",
            sha256: "7612aeeca130c741b38673279268494e0cf49891dd95574e14310e64634a6840",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.29.0-fedora-44-x86_64.tar.gz",
            sha256: "9807227a79989d6c17be18212462751e244fb1b5f2bbe1224e0054b3f37cc188",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.29.0-macos-15-arm64.tar.gz",
            sha256: "228b25d3ea7869436cf009bc228dc94f8f40fcf1d7ea81c622651c14b6bbc216",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.29.0-macos-15-x86_64.tar.gz",
            sha256: "0b173d7ebed6ce23c7224a55a32c9b9716471a054119b02869abc2add15dc391",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.29.0-macos-26-arm64.tar.gz",
            sha256: "61ee2de3ca95cea7f8253de3e47f4da9ae42bd4ac406c65374aaf4cd22b742f3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.29.0-macos-26-x86_64.tar.gz",
            sha256: "5c7593c4bb15f806a83c25dee97e26bf86a73571046d6523164885804d0c9c7d",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.29.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "54085d6e1391f98fb44b86735df0f147318502153c4d3e96c5586421835cb5aa",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.29.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "4d2016fac3c2001dae9ff152dad91057741aa6c031093b10fce4a4f3e1654045",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.29.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "dedb29838c947d2da479c9998a38d772de875bd914eae0d0a8b81377bb7b894c",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.29.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "74036ca124938a556687ff760d83fb3ad2eebb9e91a2a9cb93873a537f8e85a3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.29.0-windows-11-arm64.zip",
            sha256: "5fd451a1166e64af8f5936239f22ca88deadb6ff96058e7b9295887fa8315999",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.29.0-windows-2022-x86_64.zip",
            sha256: "b272c66ade706b7cb3c443fbd05c549abc2ef024f044b1cdfeba19f61367db78",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.29.0-windows-2025-x86_64.zip",
            sha256: "2a6b49b81af30bbe6975c7274a9c2f8cef2a3bde10a3cebae1f4c8b9c1bf5333",
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
