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
/// The pinned v0.30.1 release (contract 1.16) carries the native wallet, Web3,
/// and x402 bus operations, including mnemonic word-count validation, the
/// `NativeWalletTokenBalances` SPL token-balance read, x402 `upto` (Permit2)
/// payments, the prepare-time `network` pin and the confidential
/// `X402SignIn` (Sign-In-With-X). Secret-bearing
/// operations remain confidential and require both attestation and a matching
/// pinned digest.
pub(crate) const TINYWALLET: ModuleRecord = ModuleRecord {
    id: "tinywallet",
    description: "Transaction building and assembly for Bitcoin, EVM, Solana and Tron",
    bus_name: tinywallet_bus::names::BUS_NAME,
    object_path: tinywallet_bus::names::OBJECT_PATH,
    version: "0.30.1",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.30.1",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.30.1-archlinux-arm64.tar.gz",
            sha256: "f49630c0433fbdf4c048974904731118d2b633617753048e1b7c6818a24de719",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.30.1-archlinux-x86_64.tar.gz",
            sha256: "e2d9e21d437f222c848f52af975b0764106486cc2595b55e6d6b27924dd1641b",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.30.1-fedora-43-arm64.tar.gz",
            sha256: "51284db3fe59f60d28d09f3f8d1802539a478d7edf113d787fbf759d5ff019f3",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.30.1-fedora-43-x86_64.tar.gz",
            sha256: "320710a3cb9bfbf24a095234e2bc7bd06c3ba605571474979b671b6e4deae168",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.30.1-fedora-44-arm64.tar.gz",
            sha256: "49f0b1c6dbb454eb1aa9abd7f51e99fad6c92e245c6de9550ad95acd8012c14f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.30.1-fedora-44-x86_64.tar.gz",
            sha256: "3dd7588bcf6b2b5ad4720892f9513a12fd6e5f4b7c0be418b4bcf3e808ddfb4e",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.30.1-macos-15-arm64.tar.gz",
            sha256: "8577f4fe71cac23e37e7aaa9d60a328e3754a5df488540fdc55069d36d67a5e7",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.30.1-macos-15-x86_64.tar.gz",
            sha256: "c9be1266337f0272162906d846f6d2cd71106f6c4b1df50692c0b89c8c438bc4",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.30.1-macos-26-arm64.tar.gz",
            sha256: "e53d8d5c44d7e22e2dfcc0108edec488bb957f2b0acd882c28013d3cc0bb82ea",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.30.1-macos-26-x86_64.tar.gz",
            sha256: "e5886fa75211d9fc64da2d8d8c1ce58761c1f31050481a75d53153372599815f",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.30.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "7d56f1ccb7ea84335a4371e18484cb73aad1ff2fd744bebb155fa6ac93117ea1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.30.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "373f4d317ccdc6f1b5c12849ee5ddac03591b48b15f7ca9016222ea17851b720",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.30.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "a96ccc6975a7f69d310a752515ed3fd52cdee9d84bf8c686256b5cd881c95cb5",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.30.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "a90c058cf549f28c9092c629299a3bc7c8ed99114b11fad34ac30628d1de4fe1",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.30.1-windows-11-arm64.zip",
            sha256: "fbadd74049901eb74a2741ad8af28241f5a856445d6dd65906bef8d314287afc",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.30.1-windows-2022-x86_64.zip",
            sha256: "0e5f7ef529019c70939862c16944648145cb7aa8cb67007f4aafa50c27371d52",
            library_sha256: None,
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.30.1-windows-2025-x86_64.zip",
            sha256: "4e9b92a6c1480410fe0296c6d4905cd0234f169b2cb6db69042deab57a881323",
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
