//! The set of modules this build knows how to load.
//!
//! # Why a compiled-in table
//!
//! A loaded module is trusted native code in this process: it shares the address
//! space, the privileges and the crash domain, and tinybus never unloads it.
//! Which modules may be loaded, and which bytes count as legitimate, are
//! therefore build-time decisions rather than runtime discovery. There is no
//! "module marketplace" here on purpose — a registry a server could add entries
//! to would be a remote-code-execution surface with a download step.
//!
//! # The digests are a second gate, not the only one
//!
//! tinybus fetches the release's own `checksum.toml`, compares it with the digest
//! the host supplies, hashes the downloaded archive, and only then extracts and
//! loads. The digests below are the host's half of that agreement. Pinning them
//! in the source is what makes the check auditable offline: a reviewer can read
//! this file against the release page, and a release re-cut under the same tag
//! stops matching rather than silently replacing what runs in-process.
//!
//! # Adding an entry
//!
//! Take the values verbatim from the release's `checksum.toml`. Do not compute
//! them from a local build — the point is to pin what the release publishes, and
//! a locally recomputed digest would agree with itself no matter what was served.

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
    version: "0.1.12",
    release_url: "https://github.com/tinyhumansai/tinydocs/releases/tag/v0.1.12",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinydocs-module-0.1.12-ubuntu-24.04-x86_64.tar.gz",
            sha256: "89a1c6f3ff386a2190bfa4efbef75d564651f75cd8136c8940ec4de950f69a05",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinydocs-module-0.1.12-ubuntu-24.04-arm64.tar.gz",
            sha256: "685b38dbb9b5beba0105b2991212882ba2d4cb74fa1f3613c9eb9b75de023f0b",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinydocs-module-0.1.12-ubuntu-22.04-x86_64.tar.gz",
            sha256: "35ac3d05202dfcb425c3d6448f1740656b5df3e6276ecd97ded973f92c356591",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinydocs-module-0.1.12-ubuntu-22.04-arm64.tar.gz",
            sha256: "3870486bd42fc729cc56b7dae9343aaa854de2b24d30f4a6386a8083db6ef32e",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinydocs-module-0.1.12-macos-26-arm64.tar.gz",
            sha256: "18ab086bd58d8fec2ac407981f2013d7284a8d7e0c07cdc51ee6fdde4535f431",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinydocs-module-0.1.12-macos-26-x86_64.tar.gz",
            sha256: "426711799118bae95a691d6a61920c4bc93b76e930cdbce4209e730aa8b9efa2",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinydocs-module-0.1.12-macos-15-arm64.tar.gz",
            sha256: "f0aa5d7076a1ce3cdf4c0cf4dd15e274bfbd7d4ccfced6793e55651a7499f3d4",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinydocs-module-0.1.12-macos-15-x86_64.tar.gz",
            sha256: "9fbc1aa2dfabe35e492aa6abea90515ab83ea13a191c87306401a499d432e5e3",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinydocs-module-0.1.12-windows-2025-x86_64.zip",
            sha256: "4870bb1084ad0435b44d1ec845c5d2f398e27e430bec00ec0f58e0664e5bfc3f",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinydocs-module-0.1.12-windows-2022-x86_64.zip",
            sha256: "f1fc72690dd59890d7a629002ab2ade0547b2a3ca23c5cc54f5d40ed0e8b24af",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinydocs-module-0.1.12-windows-11-arm64.zip",
            sha256: "c4f7bda63c17a5bbdb10d8e5bab04c9b0113fbd420f83465208b64a286f2127a",
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
/// **The signing key is never sent to this module.** It returns the bytes that
/// need signing and reassembles once this process has signed them — see
/// [`super::wallet`]. Nothing in its interface accepts key material.
const TINYWALLET: ModuleRecord = ModuleRecord {
    id: "tinywallet",
    description: "Transaction building and assembly for Bitcoin, EVM, Solana and Tron",
    bus_name: "ai.tinyhumans.tinywallet.Wallet",
    object_path: "/ai/tinyhumans/tinywallet/Wallet",
    version: "0.2.0",
    release_url: "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.2.0",
    assets: &[
        PlatformAsset {
            host_key: "ubuntu-24.04-x86_64",
            archive: "tinywallet-module-0.2.0-ubuntu-24.04-x86_64.tar.gz",
            sha256: "827ae2721f4173f76247d7728c1383bd54a590608abb994a0ca2b4742ff2bd85",
        },
        PlatformAsset {
            host_key: "ubuntu-24.04-arm64",
            archive: "tinywallet-module-0.2.0-ubuntu-24.04-arm64.tar.gz",
            sha256: "5e014a6eca418c94d85f333bd804853a115ab26552e29ee6c779bb87497b116b",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-x86_64",
            archive: "tinywallet-module-0.2.0-ubuntu-22.04-x86_64.tar.gz",
            sha256: "be87ddf38ee1c2033fd568d65b22e7171c4dc24ee264cade62e20632bc5defc1",
        },
        PlatformAsset {
            host_key: "ubuntu-22.04-arm64",
            archive: "tinywallet-module-0.2.0-ubuntu-22.04-arm64.tar.gz",
            sha256: "393e68fc9a5184b3b0d71731ed066e1a0e06186aa59fe5fb8ab4a5f60cca833c",
        },
        PlatformAsset {
            host_key: "macos-26-arm64",
            archive: "tinywallet-module-0.2.0-macos-26-arm64.tar.gz",
            sha256: "ce87e1c3b4e6bbb2d41735a8bb10001001d0555cb7841fec09df5fb4d0bd99a4",
        },
        PlatformAsset {
            host_key: "macos-26-x86_64",
            archive: "tinywallet-module-0.2.0-macos-26-x86_64.tar.gz",
            sha256: "bc700a2993c403140e262e82e74dafc58d71a0b252a0e7c8aa5aa3c5f81cf55d",
        },
        PlatformAsset {
            host_key: "macos-15-arm64",
            archive: "tinywallet-module-0.2.0-macos-15-arm64.tar.gz",
            sha256: "6e61acdd6afa48efc72069e25bf9c918905e960522e60244db9e738db8f0207a",
        },
        PlatformAsset {
            host_key: "macos-15-x86_64",
            archive: "tinywallet-module-0.2.0-macos-15-x86_64.tar.gz",
            sha256: "a63da64043fc960ed13747c30ff6e8e396ba0cba7099b0616b9c6dbc54cb4a8d",
        },
        PlatformAsset {
            host_key: "windows-2025-x86_64",
            archive: "tinywallet-module-0.2.0-windows-2025-x86_64.zip",
            sha256: "bd4b7156dd031ce1821d563759fad52e73aad30aa9e8388ec0d486a8b804161e",
        },
        PlatformAsset {
            host_key: "windows-2022-x86_64",
            archive: "tinywallet-module-0.2.0-windows-2022-x86_64.zip",
            sha256: "8fbc5438adb86078b1ea4d0c6a4223daf298029e2f5db2dcd5666d446d9d6dd8",
        },
        PlatformAsset {
            host_key: "windows-11-arm64",
            archive: "tinywallet-module-0.2.0-windows-11-arm64.zip",
            sha256: "59e705e458248e8225a9dd5a103b6165e3559699921c7ff78e5b26b938e779a2",
        },
    ],
    load: LoadPolicy::Lazy,
};

/// Every module this build can load.
pub const ALL: &[ModuleRecord] = &[TINYDOCS, TINYWALLET];

/// The record for `id`, if this build knows it.
#[must_use]
pub fn find(id: &str) -> Option<&'static ModuleRecord> {
    ALL.iter().find(|record| record.id == id)
}

#[cfg(test)]
mod tests {
    use super::{find, ALL};
    use crate::openhuman::modules::platform::candidates_for;

    #[test]
    fn ids_and_bus_names_are_unique() {
        // Two records claiming one bus name is a conflict tinybus would only
        // surface at load time, on whichever one happened to be second.
        for (i, record) in ALL.iter().enumerate() {
            for other in &ALL[i + 1..] {
                assert_ne!(record.id, other.id, "duplicate module id");
                assert_ne!(record.bus_name, other.bus_name, "duplicate bus name");
            }
        }
    }

    #[test]
    fn every_object_path_matches_its_bus_name() {
        // tinybus derives a module's object path from its bus name by replacing
        // dots with slashes, and admission compares the two. A mismatch here is
        // a module that downloads and then refuses to load.
        for record in ALL {
            assert_eq!(
                record.object_path,
                format!("/{}", record.bus_name.replace('.', "/")),
                "{} object path does not match its bus name",
                record.id
            );
        }
    }

    #[test]
    fn every_digest_is_a_lowercase_sha256() {
        // An uppercase or truncated digest is refused by tinybus at download
        // time, which is a slow way to find a typo in this file.
        for record in ALL {
            for asset in record.assets {
                assert_eq!(
                    asset.sha256.len(),
                    64,
                    "{} / {} digest is not 64 characters",
                    record.id,
                    asset.host_key
                );
                assert!(
                    asset
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "{} / {} digest is not lowercase hex",
                    record.id,
                    asset.host_key
                );
            }
        }
    }

    #[test]
    fn every_asset_name_carries_its_host_key_and_a_known_extension() {
        // tinybus selects the asset by exact name and requires a `.tar.gz` or
        // `.zip` archive, so a name that does not match its key is a module that
        // loads the wrong platform's library.
        for record in ALL {
            for asset in record.assets {
                assert!(
                    asset.archive.contains(asset.host_key),
                    "{} asset {} does not name its host key {}",
                    record.id,
                    asset.archive,
                    asset.host_key
                );
                let windows = asset.host_key.starts_with("windows");
                assert_eq!(
                    windows,
                    asset.archive.ends_with(".zip"),
                    "{} asset {} has the wrong archive format for its host",
                    record.id,
                    asset.archive
                );
                if !windows {
                    assert!(asset.archive.ends_with(".tar.gz"));
                }
            }
        }
    }

    #[test]
    fn every_asset_name_carries_the_pinned_version() {
        // The digests and the version have to describe one release; an asset
        // left behind at an older version would download bytes the digest
        // beside it never matched.
        for record in ALL {
            for asset in record.assets {
                assert!(
                    asset.archive.contains(record.version),
                    "{} asset {} is not from version {}",
                    record.id,
                    asset.archive,
                    record.version
                );
            }
        }
    }

    #[test]
    fn the_release_url_is_a_tag_on_github() {
        // tinybus refuses a URL that is not a tag, because a branch URL names
        // bytes that can change under a digest that was checked once.
        for record in ALL {
            assert!(
                record
                    .release_url
                    .starts_with("https://github.com/tinyhumansai/"),
                "{} release url is not an upstream GitHub URL",
                record.id
            );
            assert!(
                record.release_url.contains("/releases/tag/"),
                "{} release url is not a tag",
                record.id
            );
            assert!(
                record.release_url.ends_with(record.version),
                "{} release url does not name version {}",
                record.id,
                record.version
            );
        }
    }

    #[test]
    fn every_host_the_platform_table_can_produce_has_an_asset() {
        // The two tables are written independently and would drift silently:
        // `platform` offering a key no release publishes turns a supported host
        // into an "unsupported host" at first use.
        let hosts = [
            ("linux", "x86_64", Some((2, 39))),
            ("linux", "aarch64", Some((2, 39))),
            ("linux", "x86_64", Some((2, 35))),
            ("linux", "aarch64", Some((2, 35))),
            ("macos", "x86_64", None),
            ("macos", "aarch64", None),
            ("windows", "x86_64", None),
            ("windows", "aarch64", None),
        ];
        for record in ALL {
            for (os, arch, glibc) in hosts {
                for key in candidates_for(os, arch, glibc) {
                    assert!(
                        record.asset_for(&key).is_some(),
                        "{} publishes no asset for {key}, which {os}/{arch} would ask for",
                        record.id
                    );
                }
            }
        }
    }

    #[test]
    fn find_resolves_known_ids_only() {
        assert!(find("tinydocs").is_some());
        assert!(find("not-a-module").is_none());
    }
}
