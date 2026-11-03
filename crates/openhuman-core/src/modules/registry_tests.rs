use super::{find, ALL};
use tinybus::module::platform::candidates_for;

#[path = "registry_computer_release_tests.rs"]
mod computer_release_tests;

#[path = "registry_release_manifest_tests.rs"]
mod release_manifest_tests;

#[test]
#[cfg(feature = "web3")]
fn tinywallet_registry_matches_the_verified_v0280_release_manifest() {
    let wallet = find("tinywallet").expect("compiled TinyWallet module");
    assert_eq!(wallet.version, "0.28.0");
    #[cfg(feature = "web3")]
    assert_eq!(tinywallet_bus::CONTRACT_VERSION, (1, 14));
    assert_eq!(
        wallet.release_url,
        "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.28.0"
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
            "tinywallet-module-0.28.0-archlinux-arm64.tar.gz",
            "da90ff7b53c2a229896d5a4ab0e3121727488f37a78551b3207a9c78bc4a09a6",
        ),
        (
            "archlinux-x86_64",
            "tinywallet-module-0.28.0-archlinux-x86_64.tar.gz",
            "1e80c400e50643d938e25b45aa0327de170f66f39fe65645a9d5ccb31b5f4a88",
        ),
        (
            "fedora-43-arm64",
            "tinywallet-module-0.28.0-fedora-43-arm64.tar.gz",
            "fb2b630ae47e64bd10a975861e647a939bfaa55b664c8f0706cd096779d62c3b",
        ),
        (
            "fedora-43-x86_64",
            "tinywallet-module-0.28.0-fedora-43-x86_64.tar.gz",
            "84cc92b8775d26def86ad5073ed7315a50aae4b5aa7134a1cf485b923a32c38b",
        ),
        (
            "fedora-44-arm64",
            "tinywallet-module-0.28.0-fedora-44-arm64.tar.gz",
            "14a1f4a044a7e5b246aa2df7209c31c165bf1305b38d888a540c2e438c11e202",
        ),
        (
            "fedora-44-x86_64",
            "tinywallet-module-0.28.0-fedora-44-x86_64.tar.gz",
            "d550966866d4b3446abef6f3b19fcf6968d914579a445c8b08fff276465b9029",
        ),
        (
            "macos-15-arm64",
            "tinywallet-module-0.28.0-macos-15-arm64.tar.gz",
            "c8c791b7464f1db2c6395f286cf63727b24bacb7606e318bb7dad5fa120387e1",
        ),
        (
            "macos-15-x86_64",
            "tinywallet-module-0.28.0-macos-15-x86_64.tar.gz",
            "b72c5f5b979302599b3a4f85b6783bd753c119e23ade819c0b1edf9ffd01d729",
        ),
        (
            "macos-26-arm64",
            "tinywallet-module-0.28.0-macos-26-arm64.tar.gz",
            "b8d189befa3358f854346f3903187ef23b990b71d314a4a58539531c9212ca82",
        ),
        (
            "macos-26-x86_64",
            "tinywallet-module-0.28.0-macos-26-x86_64.tar.gz",
            "9eb8990e3401b8c6d30ca08a434c64f56ff82e6bc1b6e160d27aae4d30a8c24f",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinywallet-module-0.28.0-ubuntu-22.04-arm64.tar.gz",
            "c753826827230d2ebcb044eeae91f96d1f08d945f1ef37b26030ae0e0843d60c",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinywallet-module-0.28.0-ubuntu-22.04-x86_64.tar.gz",
            "02cec7d7e33112266b988be1bab6213b71193f7a183db8783dba941e62ef7b46",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinywallet-module-0.28.0-ubuntu-24.04-arm64.tar.gz",
            "5b8dc519829098029609e224b95b2149a71cdaad240148e33c0abac71ec5dc07",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinywallet-module-0.28.0-ubuntu-24.04-x86_64.tar.gz",
            "c2bb2acd7c15b061b66d564db738aa3eac6802544fba753a6715ade541faac86",
        ),
        (
            "windows-11-arm64",
            "tinywallet-module-0.28.0-windows-11-arm64.zip",
            "02598b083ce4d7e66c6d295785b5b42535861614741b2371462847760bbd2823",
        ),
        (
            "windows-2022-x86_64",
            "tinywallet-module-0.28.0-windows-2022-x86_64.zip",
            "fdf15c140e0de0492fb0fde449e38e8eef0653cc7af6ebbe71c56218f5d33c17",
        ),
        (
            "windows-2025-x86_64",
            "tinywallet-module-0.28.0-windows-2025-x86_64.zip",
            "a89031486e2aeb8f2f96fc314a5a291ebc4364e3b34dcdcd6a010b1a939b365e",
        ),
    ];
    actual.sort_unstable();
    published.sort_unstable();
    assert_eq!(actual, published);
}

#[test]
fn tinybox_registry_matches_the_published_v024_release_manifest() {
    let record = find("tinybox").expect("compiled TinyBox module");
    assert_eq!(record.version, "0.2.4");
    assert_eq!(tinybox_bus::CONTRACT_VERSION, (2, 3));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinybox/releases/tag/v0.2.4"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-arm64",
            "tinybox-0.2.4-archlinux-arm64.tar.gz",
            "aaf2c37f5ebe92c8897fe8b6b0300798e7afecbb00f3f438ccd19aa566f3a6ec",
        ),
        (
            "archlinux-x86_64",
            "tinybox-0.2.4-archlinux-x86_64.tar.gz",
            "07937b998542884ed38502c534c4d857d7e9fb55da0b5ae0a5ec6d9d7a369c29",
        ),
        (
            "fedora-43-arm64",
            "tinybox-0.2.4-fedora-43-arm64.tar.gz",
            "feec38ccea69e54962387321ac6377d207ca99c0de51781e1646b88ae6b3f899",
        ),
        (
            "fedora-43-x86_64",
            "tinybox-0.2.4-fedora-43-x86_64.tar.gz",
            "ff9aabc80750b956fb1d7824f650b51bf54b2027838641432027c03b14bef1e9",
        ),
        (
            "fedora-44-arm64",
            "tinybox-0.2.4-fedora-44-arm64.tar.gz",
            "ea56348b0b33d71e6fde07bba1b32be2c659a8be94c45feacb92b6d0e9e87d5e",
        ),
        (
            "fedora-44-x86_64",
            "tinybox-0.2.4-fedora-44-x86_64.tar.gz",
            "f10fc89683e7cf0c9bc5b9baaf5d3dfce9ec75f4b7c7aed928e4668392c50d5f",
        ),
        (
            "macos-15-arm64",
            "tinybox-0.2.4-macos-15-arm64.tar.gz",
            "0983c74b6ac9a5458518ebcb557bc2d5d3d91c2ab3ecee01f4a6de0a60dc7271",
        ),
        (
            "macos-15-x86_64",
            "tinybox-0.2.4-macos-15-x86_64.tar.gz",
            "c068a76205f1bf435a490b5248a8408ac8aa2537b1e9d446583b8dfe194be34e",
        ),
        (
            "macos-26-arm64",
            "tinybox-0.2.4-macos-26-arm64.tar.gz",
            "13aa4c590177eea88f0da67000494eb72fe40c238c477a2755eb6a439e8b66e3",
        ),
        (
            "macos-26-x86_64",
            "tinybox-0.2.4-macos-26-x86_64.tar.gz",
            "99e5159922eb99eaec894e49a7dcd947c0a369e5c51192052febf38c1a15a72a",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinybox-0.2.4-ubuntu-22.04-arm64.tar.gz",
            "abcb145cc78d48a1cea4a7e3a94414d7d11e1ff70adef660cab94c11832fbaf3",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinybox-0.2.4-ubuntu-22.04-x86_64.tar.gz",
            "39ea65a08bc1aa2be74121ebca9e815be379577dc461380c4c3eb926b1b7d735",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinybox-0.2.4-ubuntu-24.04-arm64.tar.gz",
            "0d9fa4e342afd59c2c3c1b6edf24a1d0383430c9ef238cb214b1c849ecc06f0f",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinybox-0.2.4-ubuntu-24.04-x86_64.tar.gz",
            "8424a0a42894f610eb81fcfdbc6eb87f642df953a5d886127d03cdf3e51a52d1",
        ),
        (
            "windows-11-arm64",
            "tinybox-0.2.4-windows-11-arm64.zip",
            "75163660dd3eec73b1bfc0256d5453bb81e10484a9584295c0021c1ffb3c750e",
        ),
        (
            "windows-2022-x86_64",
            "tinybox-0.2.4-windows-2022-x86_64.zip",
            "8be3c207bacd75a47706e61af5367e7046b6f9d1d1bc486438f7245915d6a970",
        ),
        (
            "windows-2025-x86_64",
            "tinybox-0.2.4-windows-2025-x86_64.zip",
            "2dee76ec3fdefe95c59492f8bc95e324aae46c53147461ad1339beb1e9084186",
        ),
    ];

    assert_eq!(actual.len(), 17);
    assert_eq!(actual, published);
}
#[test]
fn tinymcp_registry_matches_the_published_v0205_release_manifest() {
    let record = find("tinymcp").expect("compiled TinyMCP module");
    assert_eq!(record.version, "0.20.5");
    assert_eq!(tinymcp_bus::CONTRACT_VERSION, (1, 28));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.20.5"
    );
    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-rolling-arm64",
            "tinymcp-0.20.5-archlinux-arm64.tar.gz",
            "2bbc95f522647321d33106d1527e4c3195e920c31ba2416f435d43676939bb1a",
        ),
        (
            "archlinux-rolling-x86_64",
            "tinymcp-0.20.5-archlinux-x86_64.tar.gz",
            "1aaa41c55dda694886b6c045f6795c26786c4936888f39a8fdef225125e09d6c",
        ),
        (
            "fedora-43-arm64",
            "tinymcp-0.20.5-fedora-43-arm64.tar.gz",
            "c7028fe80ff726b97589beeefe3aeea003e1490fcdccd707caa6fdc84785013e",
        ),
        (
            "fedora-43-x86_64",
            "tinymcp-0.20.5-fedora-43-x86_64.tar.gz",
            "d9cee4abc57e6321ca664cee5ced27b9a310658d48cc423ffece55ac857e0c61",
        ),
        (
            "fedora-44-arm64",
            "tinymcp-0.20.5-fedora-44-arm64.tar.gz",
            "6a44d66ad55ffec2d53e0cd706849cd7474bead7a9aee7d649404a4da8c52692",
        ),
        (
            "fedora-44-x86_64",
            "tinymcp-0.20.5-fedora-44-x86_64.tar.gz",
            "a6c2853bc344377619290c64ffa32a1a9f9b20ed74356db8ee492e0aa944fd4e",
        ),
        (
            "macos-15-arm64",
            "tinymcp-0.20.5-macos-15-arm64.tar.gz",
            "a037577bb462b98862dd3d76b52d5fdac05e1baaa46644db98070c2b213c3310",
        ),
        (
            "macos-15-x86_64",
            "tinymcp-0.20.5-macos-15-x86_64.tar.gz",
            "a8611f7b9858f1a97eac413a38a1d4d3dd9c27bffaf48f93d744365f66be3642",
        ),
        (
            "macos-26-arm64",
            "tinymcp-0.20.5-macos-26-arm64.tar.gz",
            "9e079804323e8421dc2924c6a88d551e4baf867b5d865e0122f478238a47025d",
        ),
        (
            "macos-26-x86_64",
            "tinymcp-0.20.5-macos-26-x86_64.tar.gz",
            "a501ff4f7e70012bff07e8ab0a1c4ff72f98b6e08330e900a8bd9fe64fb949b0",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinymcp-0.20.5-ubuntu-22.04-arm64.tar.gz",
            "a134d7401747314cf3a0a1cef5d749e6a74397d115976b7dcc6c1aabc27bef6d",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinymcp-0.20.5-ubuntu-22.04-x86_64.tar.gz",
            "a59292015abdad2f9511b8ba4befccfb50a48e09a694fb4835fe82899f96889f",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinymcp-0.20.5-ubuntu-24.04-arm64.tar.gz",
            "6b37bcc8a3c597981cbb021b0a484629bb757e0bdd14cc056aeb111506f40d51",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinymcp-0.20.5-ubuntu-24.04-x86_64.tar.gz",
            "3a240f35b6adf00107db204e96b3ba9286c9bc1bf7125a47059321dc2d9ef26b",
        ),
        (
            "windows-11-arm64",
            "tinymcp-0.20.5-windows-11-arm64.zip",
            "2d72ebb7199b0a85e6b7de3386e3ad42e9ee89a6f15eac372fba534f2379517b",
        ),
        (
            "windows-2022-x86_64",
            "tinymcp-0.20.5-windows-2022-x86_64.zip",
            "f184c044b726394461b0ef0a53c18bf6bf35b7ebb50302101b4086dce3941a47",
        ),
        (
            "windows-2025-x86_64",
            "tinymcp-0.20.5-windows-2025-x86_64.zip",
            "8e6412b981599e8676fb51444f23898d9017972a6aed42137232aaff4f7c9754",
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
            record.release_url.starts_with("https://github.com/"),
            "{} release url is not a GitHub URL",
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
    assert_eq!(search.version, "0.5.4");
    assert_eq!(tinysearch_bus::CONTRACT_VERSION, (2, 2));
    assert_eq!(
        search.asset_for("ubuntu-24.04-x86_64").unwrap().sha256,
        "44fc5fe514416063542253f3f3dd3ff5c68042eaba29de5777a2bf0a4b33f100"
    );
}
