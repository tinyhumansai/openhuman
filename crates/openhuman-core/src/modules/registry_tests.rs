use super::{find, ALL};
use tinybus::module::platform::candidates_for;

#[path = "registry_computer_release_tests.rs"]
mod computer_release_tests;

#[path = "registry_security_release_tests.rs"]
mod security_release_tests;

#[test]
#[cfg(feature = "web3")]
fn tinywallet_registry_matches_the_verified_v0241_release_manifest() {
    let wallet = find("tinywallet").expect("compiled TinyWallet module");
    assert_eq!(wallet.version, "0.24.1");
    #[cfg(feature = "web3")]
    assert_eq!(tinywallet_bus::CONTRACT_VERSION, (1, 14));
    assert_eq!(
        wallet.release_url,
        "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.24.1"
    );

    let mut actual = wallet
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    assert_eq!(wallet.assets.len(), 17);
    let mut published = [
        (
            "archlinux-arm64",
            "tinywallet-module-0.24.1-archlinux-arm64.tar.gz",
            "f0e013d305d35744911e4e88ae391bd5b698ca9b5d4b5965ff2cf7e038022a55",
        ),
        (
            "archlinux-x86_64",
            "tinywallet-module-0.24.1-archlinux-x86_64.tar.gz",
            "e8feeacc13150230db39885471868f3cb983d32cdca3b54076cfa3b7b99ca291",
        ),
        (
            "fedora-43-arm64",
            "tinywallet-module-0.24.1-fedora-43-arm64.tar.gz",
            "a5eda11c12b04838101113bb691809cccaab4f02b63927ed87b9ae23354e8024",
        ),
        (
            "fedora-43-x86_64",
            "tinywallet-module-0.24.1-fedora-43-x86_64.tar.gz",
            "81806447a07fb1e5f2e2e11c44cc3bb673b9583836b468086abda33cac7d74c6",
        ),
        (
            "fedora-44-arm64",
            "tinywallet-module-0.24.1-fedora-44-arm64.tar.gz",
            "1b46b6df147080672ed2f87c9b560f3c4ab1d5edabe16f98e66e3cc1df0556fc",
        ),
        (
            "fedora-44-x86_64",
            "tinywallet-module-0.24.1-fedora-44-x86_64.tar.gz",
            "021973415b5e6ebde7f903e9ecd7727a3511fe22bf5f4452720eb6d6b8898a7f",
        ),
        (
            "macos-15-arm64",
            "tinywallet-module-0.24.1-macos-15-arm64.tar.gz",
            "018451ecd682ca8cdd45667d9d7a10e8200a666f1ce5e76c7784f1ed1fe2a59b",
        ),
        (
            "macos-15-x86_64",
            "tinywallet-module-0.24.1-macos-15-x86_64.tar.gz",
            "8c9887fe1a321e0f13b1af3f9129e6db02186fd791d6769e3a80bae964ddb240",
        ),
        (
            "macos-26-arm64",
            "tinywallet-module-0.24.1-macos-26-arm64.tar.gz",
            "d0896d7fbb8936542cdfc4c0d138a262982d599ec015401a6dbd1d4410a39d3a",
        ),
        (
            "macos-26-x86_64",
            "tinywallet-module-0.24.1-macos-26-x86_64.tar.gz",
            "19409a538aa0fd04f246ebcbaacd4aefc7c55279d49f3d066a44257432617314",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinywallet-module-0.24.1-ubuntu-22.04-arm64.tar.gz",
            "0b7da4811e31e2d7239c62fdb30c8ac4c45667b596b27033f4f3915655fccbe7",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinywallet-module-0.24.1-ubuntu-22.04-x86_64.tar.gz",
            "5b1e62a9fa88939cf1711dc0480a6298d240ce2e49cbd4a448a2f967a2f30255",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinywallet-module-0.24.1-ubuntu-24.04-arm64.tar.gz",
            "e50ffef6a61dfbfffc0be0ad32e8388ee72c8803f63b71f435ddc6c47c337650",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinywallet-module-0.24.1-ubuntu-24.04-x86_64.tar.gz",
            "11f66fa19923f1fd151252294b947becf6eb5edafe6a7750681daf74c028d28c",
        ),
        (
            "windows-11-arm64",
            "tinywallet-module-0.24.1-windows-11-arm64.zip",
            "ad8f432919fa78ca7a0828b8c3ad02fcd1b65855cbaf1f76355dcd0423ff7415",
        ),
        (
            "windows-2022-x86_64",
            "tinywallet-module-0.24.1-windows-2022-x86_64.zip",
            "03badd28042ff3a211a489bafb42ac1fa90bb320de66bb73ef5317c7b4c560d3",
        ),
        (
            "windows-2025-x86_64",
            "tinywallet-module-0.24.1-windows-2025-x86_64.zip",
            "53fc1c72e207769fa471373320b0ca73c067c2f6b32a65e73f8fc7ef18a0fa80",
        ),
    ];
    actual.sort_unstable();
    published.sort_unstable();
    assert_eq!(actual, published);
}

#[test]
fn tinybox_registry_matches_the_published_v0142_release_manifest() {
    let record = find("tinybox").expect("compiled TinyBox module");
    assert_eq!(record.version, "0.1.42");
    assert_eq!(tinybox_bus::CONTRACT_VERSION, (1, 22));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.42"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-arm64",
            "tinybox-0.1.42-archlinux-arm64.tar.gz",
            "5f276b8b3bc291710619775e5a8dffea3c65010040f8782c9e34ba0d2bc09fe6",
        ),
        (
            "archlinux-x86_64",
            "tinybox-0.1.42-archlinux-x86_64.tar.gz",
            "570b2a0c693a3404e085c633ed9085833131d256c08ea32afddfcb0c6229ae72",
        ),
        (
            "fedora-43-arm64",
            "tinybox-0.1.42-fedora-43-arm64.tar.gz",
            "1c19f32de81571691aca6c72cf8e23cda3f15a96849204f0b0ec2f19b5e95c52",
        ),
        (
            "fedora-43-x86_64",
            "tinybox-0.1.42-fedora-43-x86_64.tar.gz",
            "876d48362e1384b805fc80eda6bdd5840ae265f812d2a058e74bf6c6d7ce633d",
        ),
        (
            "fedora-44-arm64",
            "tinybox-0.1.42-fedora-44-arm64.tar.gz",
            "cb765100fa700d751c51746697d499005eb1338dbe5d0aaf3a2cc70975eb35e2",
        ),
        (
            "fedora-44-x86_64",
            "tinybox-0.1.42-fedora-44-x86_64.tar.gz",
            "d3ec5b0ab0b5bafb9ecc61701997aa1bfb84d5c4afdff64aa90c4500a087d030",
        ),
        (
            "macos-15-arm64",
            "tinybox-0.1.42-macos-15-arm64.tar.gz",
            "d87fec4edace1f14514b46d4a442d8a3e124eeb8890cc1dcf851fcd4e663c257",
        ),
        (
            "macos-15-x86_64",
            "tinybox-0.1.42-macos-15-x86_64.tar.gz",
            "aa5b5920e5463db8fc8b891c81bd1695de67373809b93aa331c2ba5800309b90",
        ),
        (
            "macos-26-arm64",
            "tinybox-0.1.42-macos-26-arm64.tar.gz",
            "1a06b12a46cbd48b8bd8ff2ef3a90b3df2421df8e13e0541ce12dea7dac21345",
        ),
        (
            "macos-26-x86_64",
            "tinybox-0.1.42-macos-26-x86_64.tar.gz",
            "0efcbf868c59e8ba933fc745bc8bb51cb5052f6574de4f22df50195a45ba65a4",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinybox-0.1.42-ubuntu-22.04-arm64.tar.gz",
            "9876bc55ceca33cceebeff78e9e81fb6edc3a8b1a876c71c727cce942833ead9",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinybox-0.1.42-ubuntu-22.04-x86_64.tar.gz",
            "63c40e2577b97ce0ba369657a153e725a8db89b0043abfc3cb874075c266ecd5",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinybox-0.1.42-ubuntu-24.04-arm64.tar.gz",
            "58b533cd33ed5d3ef83814dbde6a9e417bb83f2087a9717f1a98a0babdac6c7b",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinybox-0.1.42-ubuntu-24.04-x86_64.tar.gz",
            "2dcc913153c5c6a03b88cc41e8aa14151159b308799b9174eb9e1a27edf1cf36",
        ),
        (
            "windows-11-arm64",
            "tinybox-0.1.42-windows-11-arm64.zip",
            "19e781b93bcb0d401ff6b9349b62bfd00d05f92e82875eed36d0689d3484692b",
        ),
        (
            "windows-2022-x86_64",
            "tinybox-0.1.42-windows-2022-x86_64.zip",
            "e9f8cc22dfce616bdf57af076ca33390c37f1417b56a26d58d265b00564ee52d",
        ),
        (
            "windows-2025-x86_64",
            "tinybox-0.1.42-windows-2025-x86_64.zip",
            "8ace9208d6a89a6d4d2766a91d5f9be937e1aaf076b4a7ca3af5c965997167d3",
        ),
    ];

    assert_eq!(actual.len(), 17);
    assert_eq!(actual, published);
}
#[test]
fn tinymcp_registry_matches_the_published_v0200_release_manifest() {
    let record = find("tinymcp").expect("compiled TinyMCP module");
    assert_eq!(record.version, "0.20.0");
    assert_eq!(tinymcp_bus::CONTRACT_VERSION, (1, 27));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.20.0"
    );
    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-rolling-arm64",
            "tinymcp-0.20.0-archlinux-arm64.tar.gz",
            "553aff39b1980d26382950ffda719bd64ecef7416e148c7367c58cd869f95785",
        ),
        (
            "archlinux-rolling-x86_64",
            "tinymcp-0.20.0-archlinux-x86_64.tar.gz",
            "88eda15faa5be1b5fc3b337e30a10798e846dcc07e4255b131f3c18154c10c26",
        ),
        (
            "fedora-43-arm64",
            "tinymcp-0.20.0-fedora-43-arm64.tar.gz",
            "89aeb98105e462ab4ecad395eac54644e3435b378e6e294d8a03e877d0e24f2f",
        ),
        (
            "fedora-43-x86_64",
            "tinymcp-0.20.0-fedora-43-x86_64.tar.gz",
            "a8187a7737ca14e0c35e3d59a5ef8abfc40db130ce386bf0c709951c93206e91",
        ),
        (
            "fedora-44-arm64",
            "tinymcp-0.20.0-fedora-44-arm64.tar.gz",
            "741416ed3739be431f54b65e422a88f7cd73c67ea27b22e3e1d19ce644d0c556",
        ),
        (
            "fedora-44-x86_64",
            "tinymcp-0.20.0-fedora-44-x86_64.tar.gz",
            "dc6a7f664f7a995746a7f50f5ed0c3f4ef58d3e257c55e996a0479ce5581d5b4",
        ),
        (
            "macos-15-arm64",
            "tinymcp-0.20.0-macos-15-arm64.tar.gz",
            "3267b60a48c96ac3466942ebc6ffcf9c239a5c9913bf57c50348e46fd2cc5a16",
        ),
        (
            "macos-15-x86_64",
            "tinymcp-0.20.0-macos-15-x86_64.tar.gz",
            "ccc01999dba5b9b87c1cbaacb6e6e5569d518150410cf2177e272cd4d639dbeb",
        ),
        (
            "macos-26-arm64",
            "tinymcp-0.20.0-macos-26-arm64.tar.gz",
            "8939493522bedc2c7f667686bf6f839dce58b24595d3c03b8077868e714bee40",
        ),
        (
            "macos-26-x86_64",
            "tinymcp-0.20.0-macos-26-x86_64.tar.gz",
            "c5307a71d7f1d0e9b34aeda3b90f55361e673fb5f67e36be2597d3a1dbd160fb",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinymcp-0.20.0-ubuntu-22.04-arm64.tar.gz",
            "4d89f2207f05d12f957b978ec3b326e5f4310e3c8ef61315492582b9e17b8181",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinymcp-0.20.0-ubuntu-22.04-x86_64.tar.gz",
            "86dec3ee58d01f58c82c4213237196060fc7d05370d8e293b77d71d7d88c0334",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinymcp-0.20.0-ubuntu-24.04-arm64.tar.gz",
            "ffd648c2fdf9d1683923b14b5076b0513d31c4e05afa6fda9dcea7c70e5cae6f",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinymcp-0.20.0-ubuntu-24.04-x86_64.tar.gz",
            "b432bf96daa177ec2562a70ce11ff0616924931db141a58f0d4d24f988b3c94d",
        ),
        (
            "windows-11-arm64",
            "tinymcp-0.20.0-windows-11-arm64.zip",
            "ef0a831e9c8223f3d9e541ca9d020f835c71290fcde6f6668c3cdf2d72b24224",
        ),
        (
            "windows-2022-x86_64",
            "tinymcp-0.20.0-windows-2022-x86_64.zip",
            "6032d1a08865d917e203ef0cf4ed9f86460007200c6d2649d3604e3db5f6bd6d",
        ),
        (
            "windows-2025-x86_64",
            "tinymcp-0.20.0-windows-2025-x86_64.zip",
            "6669bba3014b067249abec1505b0dec057aced64c3b109406df8b10fb873f70b",
        ),
    ];

    assert_eq!(actual, published);
}

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
    // `.zip` archive. Most archive platform stems match their host keys;
    // TinyMCP's release uses the canonical `archlinux` stem for the rolling
    // Arch host keys.
    for record in ALL {
        for asset in record.assets {
            let archive_host_key = if record.id == "tinymcp" {
                asset
                    .host_key
                    .strip_prefix("archlinux-rolling-")
                    .map(|suffix| format!("archlinux-{suffix}"))
                    .unwrap_or_else(|| asset.host_key.to_owned())
            } else {
                asset.host_key.to_owned()
            };
            assert!(
                asset.archive.contains(&archive_host_key),
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
    for record in ALL.iter().filter(|record| !record.assets.is_empty()) {
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

/// Every host key `platform` can produce, across the supported triples.
fn every_host_key() -> Vec<String> {
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
    let mut keys: Vec<String> = hosts
        .into_iter()
        .flat_map(|(os, arch, glibc)| candidates_for(os, arch, glibc))
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

fn supported_host_keys(_record: &super::ModuleRecord) -> Vec<String> {
    every_host_key()
}

#[test]
fn a_record_that_pins_a_release_covers_every_host_the_platform_table_offers() {
    // The two tables are written independently and would drift silently:
    // `platform` offering a key no release publishes turns a supported host
    // into an "unsupported host" at first use.
    //
    // Scoped to records that pin a release at all. A record with no assets
    // is a module this build knows but has no published artifact for; it
    // loads from a developer build or the module search path, and asserting
    // release coverage for a release that does not exist would only assert
    // that it does not exist. The partial-coverage case — the one that is
    // actually a bug — is caught below.
    for record in ALL.iter().filter(|record| !record.assets.is_empty()) {
        for key in supported_host_keys(record) {
            assert!(
                record.asset_for(&key).is_some(),
                "{} publishes no asset for {key}, which the platform table would ask for",
                record.id
            );
        }
    }
}

#[test]
fn a_record_publishes_for_every_host_or_for_none() {
    // Partial coverage is the drift that hurts: it looks supported until a
    // user on the missing platform reaches the feature. All-or-nothing keeps
    // "not published yet" distinguishable from "published and incomplete".
    for record in ALL {
        let host_keys = supported_host_keys(record);
        let covered = host_keys
            .into_iter()
            .filter(|key| record.asset_for(key).is_some())
            .count();
        assert!(
            covered == 0 || covered == supported_host_keys(record).len(),
            "{} publishes assets for {covered} of {} host keys",
            record.id,
            supported_host_keys(record).len()
        );
    }
}

#[test]
fn find_resolves_known_ids_only() {
    assert!(find("tinymcp").is_some());
    assert!(find("not-a-module").is_none());
}

#[test]
fn tinychannels_registry_matches_scoped_runtime_release() {
    let record = find("tinychannels").expect("compiled TinyChannels module");
    assert_eq!(record.version, "0.1.38");
    assert_eq!(tinychannels_bus::CONTRACT_VERSION, 21);
    assert_eq!(
        tinychannels_bus::module_relay::MODULE_OPERATION_VERSION,
        (1, 21)
    );
    let expected = [
        (
            "macos-15-arm64",
            "tinychannels-module-0.1.38-macos-15-arm64.tar.gz",
            "1c092b81c7bde10a5d83f6f797c2dd80b9992b5c292ecee06c0c8fc866c395bd",
        ),
        (
            "macos-15-x86_64",
            "tinychannels-module-0.1.38-macos-15-x86_64.tar.gz",
            "b0302b04b0feb2cdc4ddaf6ede9063eb727db95f4a1f07dc3140c03b8deff546",
        ),
        (
            "macos-26-arm64",
            "tinychannels-module-0.1.38-macos-26-arm64.tar.gz",
            "ba84151bf95ef809703c87fef6c37f598da7c492f659a10ffb9893e0e8209ffd",
        ),
        (
            "macos-26-x86_64",
            "tinychannels-module-0.1.38-macos-26-x86_64.tar.gz",
            "421de74c0fb993ccfac3a927d4ac192d615716358f5ed72cd3ce0ab17e904ee4",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinychannels-module-0.1.38-ubuntu-22.04-arm64.tar.gz",
            "ec92e0622f2b10f6c102c5b2e808695d500eacf75c7d99c558407b085e6cee4d",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinychannels-module-0.1.38-ubuntu-22.04-x86_64.tar.gz",
            "f300153ac2cb857896550bf5e53919426f43eee684f259e5d7f761b22054f1c2",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinychannels-module-0.1.38-ubuntu-24.04-arm64.tar.gz",
            "a8e4cc8cfd5528e07ffe5e6c9963d21bfd3b124bbedab7ac6c2ecbfc3e8a2821",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinychannels-module-0.1.38-ubuntu-24.04-x86_64.tar.gz",
            "7a30f6fb0f71ef6627b246e91040a7938f94aa0519bfda93a043709aa0141e49",
        ),
        (
            "windows-11-arm64",
            "tinychannels-module-0.1.38-windows-11-arm64.zip",
            "26f919a77fefd8d65c7eb6a2231a35059cb2049b3b7877673073558e19d600dd",
        ),
        (
            "windows-2022-x86_64",
            "tinychannels-module-0.1.38-windows-2022-x86_64.zip",
            "ffeec695e7aad3c22dc885266badc08cf330c885425dacdbe5f35cd9ad72bfc3",
        ),
        (
            "windows-2025-x86_64",
            "tinychannels-module-0.1.38-windows-2025-x86_64.zip",
            "5ed7a3875d6e103fdd5a577e078e7288d0dfa7eb9c4b380ab4fe0e4246611aa9",
        ),
    ];
    assert_eq!(record.assets.len(), expected.len());
    for (key, archive, sha256) in expected {
        let asset = record.asset_for(key).expect("supported platform asset");
        assert_eq!(asset.archive, archive);
        assert_eq!(asset.sha256, sha256);
    }
}

#[cfg(feature = "voice")]
#[test]
fn voice_capture_release_pins_summary_contract_and_every_platform_digest() {
    let record = find("tinyvoice").unwrap();
    assert_eq!(record.version, "0.3.4");
    assert_eq!(tinyvoice_bus::CONTRACT_VERSION, (1, 10));
    let expected = [
        (
            "macos-15-arm64",
            "tinyvoice-module-0.3.4-macos-15-arm64.tar.gz",
            "fa2c380ce034e31f96f3faae4e7d7651e9f2df5614977cae7c5bf0339dd2a8ab",
        ),
        (
            "macos-15-x86_64",
            "tinyvoice-module-0.3.4-macos-15-x86_64.tar.gz",
            "29ec1e0097a6da61e2da5c6f6d2290ddb74217a72840fd8e22d51cacdd288f6f",
        ),
        (
            "macos-26-arm64",
            "tinyvoice-module-0.3.4-macos-26-arm64.tar.gz",
            "c9c729ba646a5069b30e4b84cdeb53f3ec274406f10c10955411862eacc246f1",
        ),
        (
            "macos-26-x86_64",
            "tinyvoice-module-0.3.4-macos-26-x86_64.tar.gz",
            "5ad4ca051265063338510d891d894c347d76356828605f8c5f793986acddfdda",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinyvoice-module-0.3.4-ubuntu-22.04-arm64.tar.gz",
            "bc31815f65fff0e44b47f0da8eac10a4a404fb516d0c093649ed4e1de005e925",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinyvoice-module-0.3.4-ubuntu-22.04-x86_64.tar.gz",
            "35e5529cd3e6ab3d6ca062bc110181526851ab13d59c34d8a3febd93d21262fa",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinyvoice-module-0.3.4-ubuntu-24.04-arm64.tar.gz",
            "1ed67f4f36538e5ad5052d796a5c2bc647b0edd2cf78366c8d9be521e4e516b7",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinyvoice-module-0.3.4-ubuntu-24.04-x86_64.tar.gz",
            "ba5417d7442d3cc8c800f89f0e2e765c4f30c30bf8257a8030022bc2416ca175",
        ),
        (
            "windows-11-arm64",
            "tinyvoice-module-0.3.4-windows-11-arm64.zip",
            "c9bb31d5bbaa0d614262a92ceaa793755bb651992f6b18c768062154b42dbebd",
        ),
        (
            "windows-2022-x86_64",
            "tinyvoice-module-0.3.4-windows-2022-x86_64.zip",
            "b0aff0e3c11fc825078fa62a4ead24cec41eb938ac6d45f6231e468e6bb1283a",
        ),
        (
            "windows-2025-x86_64",
            "tinyvoice-module-0.3.4-windows-2025-x86_64.zip",
            "a4acbbd05d8965de326e9f37241c28fa14cef696062b013720c96adbd56eb4be",
        ),
    ];
    assert_eq!(record.assets.len(), expected.len());
    for (key, archive, digest) in expected {
        let asset = record.asset_for(key).unwrap();
        assert_eq!(asset.archive, archive);
        assert_eq!(asset.sha256, digest);
    }
}

#[test]
fn every_record_names_a_source_manifest_that_exists_in_its_submodule() {
    let vendor = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor");
    for record in ALL {
        let spec = &record.source;
        assert!(!spec.package.is_empty(), "{}: package", record.id);
        assert!(!spec.lib_name.is_empty(), "{}: lib_name", record.id);
        assert_eq!(spec.submodule, record.id, "{}: submodule dir", record.id);
        let manifest = vendor.join(spec.submodule).join(spec.manifest);
        assert!(
            manifest.is_file(),
            "{}: source manifest {} is missing (is the submodule initialised?)",
            record.id,
            spec.manifest
        );
    }
}

#[test]
fn source_artifact_file_names_follow_the_platform_convention() {
    for record in ALL {
        let name = record.source.artifact_filename();
        assert!(name.contains(record.source.lib_name), "{}", record.id);
        if cfg!(target_os = "linux") {
            assert!(name.starts_with("lib") && name.ends_with(".so"), "{name}");
        }
    }
}

#[test]
fn tinysearch_registry_matches_bus_contract_and_published_release() {
    let search = find("tinysearch").expect("compiled search module");
    assert_eq!(search.bus_name, tinysearch_bus::names::INTERFACE);
    assert_eq!(search.object_path, tinysearch_bus::names::OBJECT_PATH);
    assert_eq!(search.version, "0.5.1");
    assert_eq!(tinysearch_bus::CONTRACT_VERSION, (2, 2));
    assert_eq!(
        search.asset_for("ubuntu-24.04-x86_64").unwrap().sha256,
        "f078d63ada0c5051a830d803079051d9dee4c7c08c9b6e61100692ed2c63c887"
    );
}
