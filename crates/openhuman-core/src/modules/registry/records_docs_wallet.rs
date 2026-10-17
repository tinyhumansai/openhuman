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
    version: "0.2.4",
    release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.2.4",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinydocs-module-0.2.4-ubuntu-24.04-x86_64.tar.gz",
            sha256: "2e9d3772a4434ea825914788f865476bc532fdc885097b35a95211202716fd9e",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinydocs-module-0.2.4-ubuntu-24.04-arm64.tar.gz",
            sha256: "9b63791a363a25f3f26d2bb155bb782b2ceb95c2f74ce6910fc650c2614c5a86",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinydocs-module-0.2.4-ubuntu-22.04-x86_64.tar.gz",
            sha256: "ac2f0aa79ae402103726b6f5adf23f90762071e23576e5dc6701293d49ad9427",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinydocs-module-0.2.4-ubuntu-22.04-arm64.tar.gz",
            sha256: "ff8bb22f796fcab37a0a4345f7e09b64cae99d81a5622f86cd940729f83994af",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinydocs-module-0.2.4-macos-26-arm64.tar.gz",
            sha256: "5b6105f050812c3331bdac8a52dbac21c734c5239b96642b4c2bd26dcd3fa727",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinydocs-module-0.2.4-macos-26-x86_64.tar.gz",
            sha256: "2b8eab7e244e720afa47525e4207e77cbbceb77cea416defe8be2a0c0abf661a",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinydocs-module-0.2.4-macos-15-arm64.tar.gz",
            sha256: "1038a30888e002b9c8cd1083be36b13fc99903a847060ad1b690eb0c80b6081b",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinydocs-module-0.2.4-macos-15-x86_64.tar.gz",
            sha256: "3ced33ab2ef2279848f1db10e6387cb557f3fab9f1221b25724c4fbc3709a9b2",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinydocs-module-0.2.4-windows-2025-x86_64.zip",
            sha256: "1b7fefa4a704f38dec8221182ca9f94f07e5dceaf8b66bd8acc72876fe39cb0f",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinydocs-module-0.2.4-windows-2022-x86_64.zip",
            sha256: "2145b031a68ac2b62daccfda2c723781073db38130e6dbed42055fb21dd23933",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinydocs-module-0.2.4-windows-11-arm64.zip",
            sha256: "2e2b91ea44f03cd026487cfbf94b7e3e3ea8525bb05b14d31bec56a8eb8ce4d9",
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
    version: "0.23.3",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.23.3",
    assets: &[
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.23.3-archlinux-arm64.tar.gz",
            sha256: "e2adef42c2fcc9794cad1eefd0d5676cf2961a7772d0d3d5ca03890094f89924",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.23.3-archlinux-x86_64.tar.gz",
            sha256: "2a35c9b79cfc15e1917e7e1957588511ffc0ec21b2ad167cbf9e53213ca4511c",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.23.3-fedora-43-arm64.tar.gz",
            sha256: "4038195b53b79eda71a7716ab3e64b63904911915b7f44a967ceff274074a947",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.23.3-fedora-43-x86_64.tar.gz",
            sha256: "419a9a57d9ef245a852c49b32be4c07f61d58dd6315ae8c23d27ba1493615368",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.23.3-fedora-44-arm64.tar.gz",
            sha256: "0a84fb19f6c1386b35b71677b52f30bf5524f0689a9afc9e79d17b1889d24ad7",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.23.3-fedora-44-x86_64.tar.gz",
            sha256: "4a4b8e13468ade4ef23303af53e9f5121cd3547faa9cf8accb086c0290ab98fb",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.23.3-macos-15-arm64.tar.gz",
            sha256: "aee3d767944171ed68cab981092d4ae5fa0678dc6d85f3e450bc618c42682af9",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.23.3-macos-15-x86_64.tar.gz",
            sha256: "cc617f3443a15c69e1c3a2b3f94b1129262a4c2dbc76d3e76ae07b1153024bab",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.23.3-macos-26-arm64.tar.gz",
            sha256: "a3e16793b25abafbb2a26ed6b2223ce499f3f16da66bc01898307ca6fc049581",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.23.3-macos-26-x86_64.tar.gz",
            sha256: "f2dc9871afcffdf7bac9f8fcbf9f4e13776e992ad94833855fd9d7944acc132d",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.23.3-ubuntu-22.04-arm64.tar.gz",
            sha256: "58e0105f0c151bbe3aa44aa0e29539d6762a71be73bff9875f8e1d389c13500b",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.23.3-ubuntu-22.04-x86_64.tar.gz",
            sha256: "4d9d732760e95e9b8bb614a3282746053b4b7e2f3c599a395215836505c15c91",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.23.3-ubuntu-24.04-arm64.tar.gz",
            sha256: "cec6a9eb6ca864140187463e29f65a7f374cb192d75edfcbead8a101b7bb7fff",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.23.3-ubuntu-24.04-x86_64.tar.gz",
            sha256: "f2ae10459327d55d837b2016fff448fb855cf57eeb139d7e9a173066c9b00921",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.23.3-windows-11-arm64.zip",
            sha256: "21d7c857b98359d27715c70ed9983ae4dd4138648b1aa55fa7a726c5896dc528",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.23.3-windows-2022-x86_64.zip",
            sha256: "ba0c2b7c3ba8e7dbe51c854643616d27c6a9433600265f7e7d0a6cf9a03a7a7a",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.23.3-windows-2025-x86_64.zip",
            sha256: "ce3748d7d885356040d2121357ad5b8945f28d05bde1ec65540c06524a9c328b",
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
