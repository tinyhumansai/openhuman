//! Registry records for the `tinydocs` and `tinywallet` modules.

use crate::modules::types::{LoadPolicy, ModuleRecord, PlatformAsset};

/// The `tinydocs` module: document synthesis, bounded extraction, image facts,
/// and PDF rendering.
///
/// Lazy, because a user who never asks for a document should not pay a download,
/// a `dlopen`, and the resident cost of a library that is never unloaded.
pub(crate) const TINYDOCS: ModuleRecord = ModuleRecord {
    id: "tinydocs",
    description: "Document synthesis, Office/PDF extraction, image inspection, and PDF rendering",
    bus_name: "ai.tinyhumans.tinydocs.Documents",
    object_path: "/ai/tinyhumans/tinydocs/Documents",
    version: "0.2.0",
    release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.2.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinydocs-module-0.2.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "2db0db216f6bd25e07d66cd13485672dd2edcd84bed9c4f5ee383f603ced7c95",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinydocs-module-0.2.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "9d77120bceac02ac607192f12884023c8f10a18baecef18acb3694ecb8b42217",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinydocs-module-0.2.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "d3a84df5f375a9774183dd5c1fd344ac373752b54473a6d2fb611c7cab5142d2",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinydocs-module-0.2.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "fa094051f186e085db8bdc304215aa5553c0f29c958b78cc337623815e62c39a",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinydocs-module-0.2.0-macos-26-arm64.tar.gz",
            sha256: "94553eef6029b61ad135d406000e7ccfb03857e68be03fed990d8912b9b1995f",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinydocs-module-0.2.0-macos-26-x86_64.tar.gz",
            sha256: "b01bba79f64731534ca93c89d045cfdd8cd8d6ce34249810a99e76735e4e3b59",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinydocs-module-0.2.0-macos-15-arm64.tar.gz",
            sha256: "b92f0b31d942f56784bf9fd6897736595f5c1bf39db34a7115e5d9ddafb0fe72",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinydocs-module-0.2.0-macos-15-x86_64.tar.gz",
            sha256: "2f6ff91ff580462a234e7476bb0dc3c6fda975786e2ea920b0a85ece3df760d8",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinydocs-module-0.2.0-windows-2025-x86_64.zip",
            sha256: "e799aca9485cff97eceb4a3f8f6882c5e92abb17e639a68d2f774fcf57f035c2",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinydocs-module-0.2.0-windows-2022-x86_64.zip",
            sha256: "5a271fa3a9c472cdd531d259db92dc04512199ac1cd586fdf31c3af14c3299fb",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinydocs-module-0.2.0-windows-11-arm64.zip",
            sha256: "15541820df3ff69621485846d328495d42d06e7d9b2e73c42eea5af2242a9a4c",
        },
    ],
    load: LoadPolicy::Lazy,
};

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
/// The pinned v0.13.0 release carries the native wallet, Web3, and x402 bus
/// operations. Secret-bearing operations remain confidential and require both
/// attestation and a matching pinned digest.
pub(crate) const TINYWALLET: ModuleRecord = ModuleRecord {
    id: "tinywallet",
    description: "Transaction building and assembly for Bitcoin, EVM, Solana and Tron",
    bus_name: "ai.tinyhumans.tinywallet.Wallet",
    object_path: "/ai/tinyhumans/tinywallet/Wallet",
    version: "0.13.0",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.13.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.13.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "284983d01c548109b3e9d5bbca1951787912f713b4a98920b9b45cfd034b6760",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.13.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "18ef8ffe518f731dc594d1606554ca12c840219e54f230382937061c395ab860",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.13.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "92d0d0fe9ab20fdf4c44aa58d762b7e28b6a08876899a5b2d368df3feabcd2d5",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.13.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "4d0e17d778c0c84df17e4c5b7fa60aa9296efecc5726902008946409b3a3c9e1",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.13.0-macos-26-arm64.tar.gz",
            sha256: "e01a70efbcece8e134abfa84c70181425802415525303d7ba2a7798afbab1041",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.13.0-macos-26-x86_64.tar.gz",
            sha256: "c48f1f7807d0b25c2b54dca97237f6e64d02ec3a9396370cd8e3b1857419296a",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.13.0-macos-15-arm64.tar.gz",
            sha256: "95f69f2e17ff1785d032f6e1017e12de5c6dc09af9633e67fc9c225dfcef9ce6",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.13.0-macos-15-x86_64.tar.gz",
            sha256: "f3114694e2d666ea1b030db952c589f08d79c60ab79aadb844327e273c5faaf5",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.13.0-windows-2025-x86_64.zip",
            sha256: "7b79a25a1fccef17a4c6e0d400f49646c3a5f321821bb03ddc23e76d6030717a",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.13.0-windows-2022-x86_64.zip",
            sha256: "0b7126be04b23dd5fad883d7f1dd33dd65c1042af68f93593590ea07c5daa95e",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.13.0-windows-11-arm64.zip",
            sha256: "483d3fda2c46de71c8efd5dfa4ec828c0b6c86bb164c8362c1356f85cb8ec231",
        },
        PlatformAsset {
            host_key: "archlinux-arm64",
            archive: "tinywallet-module-0.13.0-archlinux-arm64.tar.gz",
            sha256: "ad2222bd1d2dd680568f486e40389c0b963cf5ba2d170f9d658985f31cabb511",
        },
        PlatformAsset {
            host_key: "archlinux-x86_64",
            archive: "tinywallet-module-0.13.0-archlinux-x86_64.tar.gz",
            sha256: "d67f541330f7712106238b90d993550d2e2e55f33d3c8c9f50669a1a51c1e981",
        },
        PlatformAsset {
            host_key: "fedora-43-arm64",
            archive: "tinywallet-module-0.13.0-fedora-43-arm64.tar.gz",
            sha256: "a2d62b03f2e99008ee3fe7ea84290fd050e12016ce36e89fda24d5449e795261",
        },
        PlatformAsset {
            host_key: "fedora-43-x86_64",
            archive: "tinywallet-module-0.13.0-fedora-43-x86_64.tar.gz",
            sha256: "0c8c5dbf2aa7b7caa371aa6366c7cfdd232fda41e2dbd894d2ef344c7c2cfa0a",
        },
        PlatformAsset {
            host_key: "fedora-44-arm64",
            archive: "tinywallet-module-0.13.0-fedora-44-arm64.tar.gz",
            sha256: "272b535910fc4bc84099887b7ccfbdd9d41a6d187616086c174f86cb77f21757",
        },
        PlatformAsset {
            host_key: "fedora-44-x86_64",
            archive: "tinywallet-module-0.13.0-fedora-44-x86_64.tar.gz",
            sha256: "3dcad60a65d4f0bbb1af838468a2f52d648d22b2fbc849ae0548a30ed9806646",
        },
    ],
    load: LoadPolicy::Lazy,
};
