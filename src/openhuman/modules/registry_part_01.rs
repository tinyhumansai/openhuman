use super::types::{LoadPolicy, ModuleRecord, PlatformAsset};

/// The `tinydocs` module: `.docx` / `.pptx` synthesis and `.pdf` extraction.
///
/// Lazy, because a user who never asks for a document should not pay a download,
/// a `dlopen`, and the resident cost of a library that is never unloaded.
const TINYDOCS: ModuleRecord = ModuleRecord {
    id: "tinydocs",
    description: "Document synthesis (.docx, .pptx) and PDF text extraction",
    bus_name: "ai.tinyhumans.tinydocs.Documents",
    object_path: "/ai/tinyhumans/tinydocs/Documents",
    version: "0.1.15",
    release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.1.15",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinydocs-module-0.1.15-ubuntu-24.04-x86_64.tar.gz",
            sha256: "15a425fb336559bc87722ea0208fe911afd915eda33df9155ce08fe1e737d3a1",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinydocs-module-0.1.15-ubuntu-24.04-arm64.tar.gz",
            sha256: "b1ea63930860624ee1b221cb8669e4435f5551692634ad3911ad9d94d9407aa5",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinydocs-module-0.1.15-ubuntu-22.04-x86_64.tar.gz",
            sha256: "a0f6991e9feeb29a2cbf7a802e46e27232c028517f00606c56a57383c00ba83b",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinydocs-module-0.1.15-ubuntu-22.04-arm64.tar.gz",
            sha256: "c9e66d54c39108cf125d37ba6c1d36cde6f702901a0ba5dcb007e8c6f8f8fd63",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinydocs-module-0.1.15-macos-26-arm64.tar.gz",
            sha256: "b1fdcce5debb275220b1deaea4b90c84daea212496d08a8af95fd7c1d02aa32c",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinydocs-module-0.1.15-macos-26-x86_64.tar.gz",
            sha256: "cbef909e15ef48151feb90746b23c42a2a54ee0cabaa48ee1324c8f5e584f768",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinydocs-module-0.1.15-macos-15-arm64.tar.gz",
            sha256: "979fc81cc8c73ca52afddb2bb363d126f2cabdc0875b0fe954b4bd2d58a308a9",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinydocs-module-0.1.15-macos-15-x86_64.tar.gz",
            sha256: "3094a320870183ca77629f4132ab99f2a411276d3111002b0dd25028904e2517",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinydocs-module-0.1.15-windows-2025-x86_64.zip",
            sha256: "8e7ee6ad41d5b4d39dc54c18802079d4a0ba5757acbf3b3c3b8aef3e0754afdf",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinydocs-module-0.1.15-windows-2022-x86_64.zip",
            sha256: "70bcf4ab948d19c99965c914de44fa9819e57a12cc984217ce3b1851b7584888",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinydocs-module-0.1.15-windows-11-arm64.zip",
            sha256: "f43dea943fab9179c827df5718562a7263ea73f35d915660f9d9fdeadc7de347",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinywallet` module: transaction building and assembly for four chains.
///
/// Lazy for the same reason as [`TINYDOCS`], and more so: most sessions never
/// touch a wallet, and this artifact carries `bitcoin` and a native `secp256k1`
/// build that would otherwise be resident for all of them.
///
/// **This host sends it the recovery phrase, over confidential calls, and never
/// derives or signs itself.** All four chains — Bitcoin, EVM, Solana and Tron —
/// derive and sign inside the module. This binary does not link the root
/// `tinywallet` crate at all — it takes `tinywallet-bus`, the wire contract,
/// which carries no `key` gate — nor does it link `k256`; see the note on the
/// `tinywallet-bus` dependency.
///
/// The phrase is only sent to a module tinybus has attested *and* whose digest
/// matches one of the entries below — `super::wallet::attested_proxy` checks
/// this table itself rather than trusting that some check happened.
///
/// The contract also exposes `ExportKey` for downstream hosts that must drive
/// a signer locally; OpenHuman itself does not call it.
///
/// Three releases got here, and the order mattered. v0.2.3 changed no method at
/// all — it was the same module rebuilt against a bus that could attest it.
/// Attestation used to be recorded only from a `modules.toml` beside the
/// artifact, and a release download extracts into a temporary directory that has
/// none, so this module could never be an attested recipient however carefully
/// the digest below was pinned (tinybus#15 fixed that). Only then was it safe
/// for v0.3.0 to add methods that take a secret, and for v0.4.0 to add
/// `SignMessage` for the Solana and x402 encodings the wire contract does not
/// model. Adding them earlier would have made them unreachable in production and
/// reachable in a developer's tree, which is the worst of both.
const TINYWALLET: ModuleRecord = ModuleRecord {
    id: "tinywallet",
    description: "Transaction building and assembly for Bitcoin, EVM, Solana and Tron",
    bus_name: "ai.tinyhumans.tinywallet.Wallet",
    object_path: "/ai/tinyhumans/tinywallet/Wallet",
    version: "0.5.1",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.5.1",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.5.1-ubuntu-24.04-x86_64.tar.gz",
            sha256: "60ae46bc18b08671d7646ad01affa7f878a4b035a050a47a80353bca5dffc276",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.5.1-ubuntu-24.04-arm64.tar.gz",
            sha256: "9f516cb30d36314c72b49c30ad81661da2435a40a064e99583658ba99ed9db2a",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.5.1-ubuntu-22.04-x86_64.tar.gz",
            sha256: "88b63685cab8a622416f24f1ad569153f249d6d74732ff33c79e4021cf64a611",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.5.1-ubuntu-22.04-arm64.tar.gz",
            sha256: "6c86be45fd260690a93f36024abc9d4f777c30233c70b0363bc23bd25dc4fdfb",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.5.1-macos-26-arm64.tar.gz",
            sha256: "4e517d4a3440c2aad852cf5ec23d12863dc9748ecad256dc1578b9f21f4f0ace",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.5.1-macos-26-x86_64.tar.gz",
            sha256: "94a6270d07aa0f0788312383552c1baec26ba2db6856c1ab425ddafc4d61b3bb",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.5.1-macos-15-arm64.tar.gz",
            sha256: "95e1f1905e0c358ae03b448c11b8c8a413d67951fd02b9f0d78f811f7e5e3037",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.5.1-macos-15-x86_64.tar.gz",
            sha256: "55973b9a5b2c0cea8ddd380a3b9ece65c09485faeea2d1cc1e6846b8e888828a",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.5.1-windows-2025-x86_64.zip",
            sha256: "96037a6c660e31b747898976feba5c73bd05a47a34b1c9d1f125c215cffde5ec",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.5.1-windows-2022-x86_64.zip",
            sha256: "256282d33a832f19cfae3309194f08a602676f4f1c9e52fb2ced998216882f4c",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.5.1-windows-11-arm64.zip",
            sha256: "8b56db13123abe43035d339521323cb74969fedc5d57df2d591153dd3afec4d6",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The complete TinyMemory engine, loaded eagerly so its capabilities are
/// available when the kernel assembles its RPC and tool surfaces.
const TINYMEMORY: ModuleRecord = ModuleRecord {
    id: "tinymemory",
    description: "Local memory engine: store, ranked recall, and portable export",
    bus_name: "ai.tinyhumans.tinymemory.Memory",
    object_path: "/ai/tinyhumans/tinymemory/Memory",
    version: "1.13.6",
    release_url: "https://github.com/tinyhumansai/tinymemory/releases/tag/v1.13.6",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinymemory-module-1.13.6-ubuntu-24.04-x86_64.tar.gz",
            sha256: "7c6c940d89e10c1115a467b38630f9e2f318d7a63e3dd94d2a7b4901229c7836",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinymemory-module-1.13.6-ubuntu-24.04-arm64.tar.gz",
            sha256: "8b20a0d170c38452a6af8e6443a5a9eea46ebda6faa16884ea44cab5725caeb2",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinymemory-module-1.13.6-ubuntu-22.04-x86_64.tar.gz",
            sha256: "d96be20eb93b0d5ed512c643dce6fdb2f53908a8d311c3cfe8d50ff6e7c7dd33",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinymemory-module-1.13.6-ubuntu-22.04-arm64.tar.gz",
            sha256: "b9650c369b7b3efe425276245a874046c3486ee5d943fcf5c2b4369999b6563e",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinymemory-module-1.13.6-macos-26-arm64.tar.gz",
            sha256: "5915a34d4e086800f4367bb574a02254e664e1f804bb08f216c3d5e1750c896a",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinymemory-module-1.13.6-macos-26-x86_64.tar.gz",
            sha256: "58dd9148870a1680d498d2c3fb203578eae3ba6319c5d534ac0e2f578fe7a345",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinymemory-module-1.13.6-macos-15-arm64.tar.gz",
            sha256: "fd382bb59afe0864b172b5579393cf99dc3d8255749153462bc3546e662c2a8c",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinymemory-module-1.13.6-macos-15-x86_64.tar.gz",
            sha256: "b140239f7ff6d6502cc7d2b339d4fdbe77eb9e81a532e0ea404afafc57dc0392",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinymemory-module-1.13.6-windows-2025-x86_64.zip",
            sha256: "58828f22658323c3805b17eabbd70340038e425412764594e800af782d0689c9",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinymemory-module-1.13.6-windows-2022-x86_64.zip",
            sha256: "8fe3bc0310b6751aae79af87da4ce1c19976d0d3b80b68082b9a295c1718b02c",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinymemory-module-1.13.6-windows-11-arm64.zip",
            sha256: "bfeee710e7c8fa8893bf29e1ad7a766f494cbd368fda55047d88242be7021f5a",
        },
    ],
    // Eager, unlike the two codecs above. A codec that is never asked for should
    // not be paid for, but a memory driver's absence changes what the kernel
    // offers rather than merely delaying it: capabilities are read at bind time
    // and the RPC surface and agent-tool list are filtered from them. Resolving
    // that during a user's first recall would mean the first recall is the one
    // that behaves differently.
    load: LoadPolicy::Eager,
};

/// The `tinyjuice` content-aware tool-output compression engine.
///
/// Lazy because the host's compaction policy can disable it, and a session that
/// never produces compressible tool output should not pay the download or
/// resident native-library cost.
const TINYJUICE: ModuleRecord = ModuleRecord {
    id: "tinyjuice",
    description: "Content-aware tool-output compression and recoverable caching",
    bus_name: "ai.tinyhumans.tinyjuice.Compression",
    object_path: "/ai/tinyhumans/tinyjuice/Compression",
    version: "0.2.5",
    release_url: "https://github.com/tinyhumansai/tinyjuice/releases/tag/v0.2.5",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyjuice-module-0.2.5-ubuntu-24.04-x86_64.tar.gz",
            sha256: "e46e1b9338c20ce3b42403ace7bd7fe563a553ea3472f2111292a3fe200f6e67",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyjuice-module-0.2.5-ubuntu-24.04-arm64.tar.gz",
            sha256: "fb2098d392d37001af728263fd375ab4663bc301be45e8ce4c713ee2b070badf",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyjuice-module-0.2.5-ubuntu-22.04-x86_64.tar.gz",
            sha256: "946b78e860a1d8c913d83b913daef4966cfccaa2eb8814d4107bbbd63813033d",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyjuice-module-0.2.5-ubuntu-22.04-arm64.tar.gz",
            sha256: "6eaae41654850e1fa695f3bbba6eb2c9531cd97eb3e5f163d8290510f3e668e5",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyjuice-module-0.2.5-macos-26-arm64.tar.gz",
            sha256: "c1c154619827ca8c9f4f7b73c05dbbf992e98242de5e46bfe15dd82aa1c21746",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyjuice-module-0.2.5-macos-26-x86_64.tar.gz",
            sha256: "b02842c52ef0f92849d08d839a01096a4b9b2900e364f4bd4f32076c2c4d2bad",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyjuice-module-0.2.5-macos-15-arm64.tar.gz",
            sha256: "f22a74aaf6972b7062ceb83ce79671a80f26135a2ee507a74a9c250dcf6bf250",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyjuice-module-0.2.5-macos-15-x86_64.tar.gz",
            sha256: "f0b428fac0c8a7352faf284e27ba73fab949686b404373be9792ce0fa40471eb",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyjuice-module-0.2.5-windows-2025-x86_64.zip",
            sha256: "0c37efc18feeaf84388d9a818ea821c5b92a937ae49f5fd2d2ed7e7436615e1e",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyjuice-module-0.2.5-windows-2022-x86_64.zip",
            sha256: "56685bba88c800f7bfb0d6a61dba1d4c31f63ae6eb8ad9df12116a39739a3df2",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyjuice-module-0.2.5-windows-11-arm64.zip",
            sha256: "3c0956cb1e7f0123b0a53c8f7e1f66a7eb672b090d5a8add611c29844726331c",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinyvoice` module: the host-agnostic half of the voice pipeline.
///
/// Wake-word gating, fast-path command routing, STT hallucination detection,
/// and the capture-side audio work (downmix, resample, silence gate, WAV
/// framing).
///
/// Lazy, and more clearly so than the others: voice is opt-in twice over — a
/// user has to enable dictation or always-on listening before any of this runs
/// — so a session that never speaks should not pay a download or a `dlopen`.
///
/// **The VAD deliberately does not come through here.** A segmenter is driven
/// once per 20 ms frame from inside a `cpal` callback, and a bus round trip at
/// that cadence would cost more than the sixty-line state machine it replaces.
/// `voice::always_on` keeps its own; see [`super::voice`].
const TINYVOICE: ModuleRecord = ModuleRecord {
    id: "tinyvoice",
    description: "Wake-word gating, command routing, hallucination detection, capture audio",
    bus_name: "ai.tinyhumans.tinyvoice.Voice",
    object_path: "/ai/tinyhumans/tinyvoice/Voice",
    version: "0.1.6",
    release_url: "https://github.com/tinyhumansai/tinyvoice/releases/tag/v0.1.6",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyvoice-module-0.1.6-ubuntu-24.04-x86_64.tar.gz",
            sha256: "40686bfb1840024d1a49bb2959e454fde44a54ec35b0e86a160fbcb031d4242a",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyvoice-module-0.1.6-ubuntu-24.04-arm64.tar.gz",
            sha256: "da691e1007691a7f4a4b377ee5f0ca03553c57f0ef887d188708d764e5b5a24b",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyvoice-module-0.1.6-ubuntu-22.04-x86_64.tar.gz",
            sha256: "b494fe15f2270718b93d1b56b1e848694ca88b6efcbdd9a134594ef042660ed7",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyvoice-module-0.1.6-ubuntu-22.04-arm64.tar.gz",
            sha256: "e641ea4eea89fa0f90f00577445351fb7a2762a6b473443c17240e5d5c6bcde9",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyvoice-module-0.1.6-macos-26-arm64.tar.gz",
            sha256: "3ff9a2d4b7b7e055cf15f4566c7b84032b90bc913ebf022cb69bc6d884fe6945",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyvoice-module-0.1.6-macos-26-x86_64.tar.gz",
            sha256: "e77f2104f3b6e6230a449d220fd1fdc475d57bc329e7ddeac6223b9cb3b50422",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyvoice-module-0.1.6-macos-15-arm64.tar.gz",
            sha256: "4ba67976463dd164471c4bbe34bf7f3ad7280fb80c4e660c6692bea62bdca20f",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyvoice-module-0.1.6-macos-15-x86_64.tar.gz",
            sha256: "39795777b8c27f8726473a8a26a6372430ac586046cd5a6c0ccc633815d83ea9",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyvoice-module-0.1.6-windows-2025-x86_64.zip",
            sha256: "22c3c48e918156ed3680e8cdcebf493f37cab99c01ec5f44ff3d316c93942c02",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyvoice-module-0.1.6-windows-2022-x86_64.zip",
            sha256: "0e7cef580404005c5736a6e5ab0a9430bc30ee017955536365dbbb2e06da5b6b",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyvoice-module-0.1.6-windows-11-arm64.zip",
            sha256: "0bc68da8a0f937436f74384184402ca502cc92d261d67b6d55f41e2e43582874",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinyruntime` module: the runtime router.
///
/// Resolves a language runtime, installs one when the host has none, reuses one
/// when it does, and runs code on a bounded pool of warm interpreter processes.
/// It is a router: on its own it knows no languages, and it routes to the two
/// provider records below.
///
/// Lazy, because a host that never runs a skill, a flow step, or a `node_exec`
/// should not pay a download and a `dlopen` for the ability to.
///
/// The digests below are v0.2.2's, taken verbatim from that release's
/// `checksum.toml`. Until it existed this record carried no assets at all and
/// the module was reachable only from a developer build named by
/// `modules.local` or found on `OPENHUMAN_MODULE_PATH` — so on any machine that
/// had not built it, the runtime domain was a set of tools that could not run.
const TINYRUNTIME: ModuleRecord = ModuleRecord {
    id: "tinyruntime",
    description: "Language runtime resolution, installation, and pooled execution",
    bus_name: "ai.tinyhumans.runtime.Runtime",
    object_path: "/ai/tinyhumans/runtime/Runtime",
    version: "0.2.4",
    release_url: "https://github.com/tinyhumansai/tinyruntime/releases/tag/v0.2.4",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyruntime-0.2.4-ubuntu-24.04-x86_64.tar.gz",
            sha256: "c0b24429d345a8a62b448de420d98fbef2f31c070ff5d5b1b378d7417ef528ae",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyruntime-0.2.4-ubuntu-24.04-arm64.tar.gz",
            sha256: "4b23925dbd0f6646ec67927909a7e86582332d70a785ddb707f3b043fe5f9664",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyruntime-0.2.4-ubuntu-22.04-x86_64.tar.gz",
            sha256: "f67923a9a84d3924ccb2965fba68fe0961f71c10e1f61888f958deded3b6ba83",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyruntime-0.2.4-ubuntu-22.04-arm64.tar.gz",
            sha256: "61b89164633fa5c234a454559f54ac14c41ad35a7c04f779c3c65daae69f3401",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyruntime-0.2.4-macos-26-arm64.tar.gz",
            sha256: "43342102c5c85af1b0e839b8e3c732db47aa5e04bcc9b6d2c4d6657e4a118efc",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyruntime-0.2.4-macos-26-x86_64.tar.gz",
            sha256: "dd37d959c0d3d721e2654e7c3af4ccd4f1f771b4ac9a83db13dbae761331d10b",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyruntime-0.2.4-macos-15-arm64.tar.gz",
            sha256: "ae4e7577e2031e196dce6b604b821450bb0a0d8610f82a68887a6f0ea2e3db31",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyruntime-0.2.4-macos-15-x86_64.tar.gz",
            sha256: "90168f2be16717f10cf1a27337343df697b0d502d52250342184b0c16380c20a",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyruntime-0.2.4-windows-2025-x86_64.zip",
            sha256: "7556dc37998c73fa0141aa8b737245b44e06473fc115cdde0eaa861804b1dfe1",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyruntime-0.2.4-windows-2022-x86_64.zip",
            sha256: "b1f4679705f83e7627407fe108470b81d9dcfb0f978d5eb11de49e8ff1b9fdc9",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyruntime-0.2.4-windows-11-arm64.zip",
            sha256: "68bc59305c230f3d7b97722ecc009a3a25d55398398a51261049c190c52571a9",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinyruntime-nodejs` module: the Node.js half of the router's knowledge.
///
/// Answers which host interpreters count, which archive nodejs.org publishes for
/// this machine, where the binaries land, and what a warm Node worker is. It
/// installs nothing itself.
///
/// It implements the shared `ai.tinyhumans.runtime.Provider` interface but
/// serves at its own object path, because two modules cannot claim one bus name
/// and tinybus derives the path from the name.
///
/// Lazy, and loaded by the same call that loads the router: a language is only
/// worth its `dlopen` when something asks for that language.
///
/// Released alongside the router and pinned the same way — see [`TINYRUNTIME`].
const TINYRUNTIME_NODEJS: ModuleRecord = ModuleRecord {
    id: "tinyruntime-nodejs",
    description: "Node.js runtime provider for tinyruntime",
    bus_name: "ai.tinyhumans.runtime.nodejs.Provider",
    object_path: "/ai/tinyhumans/runtime/nodejs/Provider",
    version: "0.2.2",
    release_url: "https://github.com/tinyhumansai/tinyruntime-nodejs/releases/tag/v0.2.2",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyruntime-nodejs-0.2.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "60bebfacfaccc5c899044fe542a07b1b2ef74ffeeca5d7f53ef0338b6dab4865",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyruntime-nodejs-0.2.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "ff9114e32db29de2a43df83e7d8b330926d5862cdb50ca20adc863d5d99becaf",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyruntime-nodejs-0.2.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "3f25a17d41226fa8cc56cd9f5f5bd447bff4b9f55c1bd68d7bf8ebbf10575aaa",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyruntime-nodejs-0.2.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "ec271b78487caaea5c5ae1951568a838be49b5df4d362d8855cb27ba243a8c44",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyruntime-nodejs-0.2.2-macos-26-arm64.tar.gz",
            sha256: "394d160e8de754e09121a52ae6a4b5a7b440c0035fb52cbdaa2dfe7ee523b7b0",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyruntime-nodejs-0.2.2-macos-26-x86_64.tar.gz",
            sha256: "bbde43f8d839aacb34f735bbde2e8f56207a1a49fb5b07732a3be7b486243ce3",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyruntime-nodejs-0.2.2-macos-15-arm64.tar.gz",
            sha256: "83ea9c8ea1b43dc4e98cb585e98d254080c2070092b3c1458f19012df5ea3cd8",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyruntime-nodejs-0.2.2-macos-15-x86_64.tar.gz",
            sha256: "6bdb686d1e857d6c28a49ab2ab87785d8c4fecbf7ef62ad218d7b3e159e2339a",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyruntime-nodejs-0.2.2-windows-2025-x86_64.zip",
            sha256: "36aab2547fbb7f336e15ecb66768661a4bd35f3da6179fc3efcd47bbb8d0df96",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyruntime-nodejs-0.2.2-windows-2022-x86_64.zip",
            sha256: "0beaf8ee4765b10f1d12d0ee0c872209935fa48184424842aa6fd299a6e3f5a8",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyruntime-nodejs-0.2.2-windows-11-arm64.zip",
            sha256: "d47571781dc17edfb0438943fbe2026417d33414904667ade0f9cb6de27e5733",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// The `tinyruntime-python` module: the Python half of the router's knowledge.
///
/// Answers which host interpreters count, which standalone build to install, and
/// what a warm Python worker is. It installs nothing itself.
///
/// Released alongside the router and pinned the same way — see [`TINYRUNTIME`].
const TINYRUNTIME_PYTHON: ModuleRecord = ModuleRecord {
    id: "tinyruntime-python",
    description: "Python runtime provider for tinyruntime",
    bus_name: "ai.tinyhumans.runtime.python.Provider",
    object_path: "/ai/tinyhumans/runtime/python/Provider",
    version: "0.2.2",
    release_url: "https://github.com/tinyhumansai/tinyruntime-python/releases/tag/v0.2.2",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinyruntime-python-0.2.2-ubuntu-24.04-x86_64.tar.gz",
            sha256: "8d020d8af32f2735e646e164124a84027d260638a1d3cfa392e7c97de179eca6",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinyruntime-python-0.2.2-ubuntu-24.04-arm64.tar.gz",
            sha256: "49fb3458636a8247b9735d80a573538bec8c73f8323e9ad0e2eaf5715b88edf1",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinyruntime-python-0.2.2-ubuntu-22.04-x86_64.tar.gz",
            sha256: "4f7e23f6f20df2820489f3cde4445e319c5b4c5285bb37e113112f7d83d37a57",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinyruntime-python-0.2.2-ubuntu-22.04-arm64.tar.gz",
            sha256: "89ca7864016bd62d2b247fc791b800acf7bbe8903bf40a12da2396e1396a9f63",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinyruntime-python-0.2.2-macos-26-arm64.tar.gz",
            sha256: "2d091cbb29dc9d06996f290eaea8f03cf027e8fc9cff72824b9eae86d7ce5483",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinyruntime-python-0.2.2-macos-26-x86_64.tar.gz",
            sha256: "b0ec8c06202bf148463a087920387d3f243761756a570a334af16b9ba473267f",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinyruntime-python-0.2.2-macos-15-arm64.tar.gz",
            sha256: "5577ed48e84d35ec07d0de8db29c840e0addcd5e54792a02b714e883a65a7ed8",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinyruntime-python-0.2.2-macos-15-x86_64.tar.gz",
            sha256: "e08fb6a06a47fd3a1e4e9ae1b6a52f42f3b78655c5f91f4e5dbd7448d6db19a4",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinyruntime-python-0.2.2-windows-2025-x86_64.zip",
            sha256: "e22d5120ae58f9562a9861cd2c84a4d88ac692fa12d283ae047aafbe1a71adcc",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinyruntime-python-0.2.2-windows-2022-x86_64.zip",
            sha256: "41f27a63ad1e5cc2559ed2fa11d698a775dad55763c7b5e5c884a3ef14f1a811",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinyruntime-python-0.2.2-windows-11-arm64.zip",
            sha256: "0e96e8c0dbf1cfd497c8691928659c9f0bb3bf42a77eaa02bce59547f63b929e",
        },
    ],
    load: LoadPolicy::Lazy,
};
