use super::{find, ALL};
use tinybus::module::platform::candidates_for;

#[cfg(feature = "security-module")]
#[test]
fn tinysecurity_is_eager_and_matches_the_typed_policy_contract() {
    let record = find("tinysecurity").expect("compiled native security module");
    assert_eq!(record.bus_name, tinysecurity_bus::names::INTERFACE);
    assert_eq!(record.object_path, tinysecurity_bus::names::OBJECT_PATH);
    assert_eq!(record.load, crate::modules::LoadPolicy::Eager);
}

#[test]
fn tinycomputer_registry_matches_bus_contract_and_published_release() {
    let desktop = find("tinycomputer").expect("compiled computer module");
    assert_eq!(desktop.bus_name, tinycomputer_bus::names::INTERFACE);
    assert_eq!(desktop.object_path, tinycomputer_bus::names::OBJECT_PATH);
    assert_eq!(desktop.version, "0.10.1");
    assert_eq!(desktop.assets.len(), 7);
    assert_eq!(
        desktop.asset_for("macos-26-arm64").unwrap().sha256,
        "39d1ca3db737c26b4d39d9a22c16da7845735c6349dd012c59533499498af874"
    );
}

#[test]
fn tinybox_registry_matches_the_published_v0116_release_manifest() {
    let record = find("tinybox").expect("compiled TinyBox module");
    assert_eq!(record.version, "0.1.16");
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.16"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "ubuntu-24.04-x86_64",
            "tinybox-0.1.16-ubuntu-24.04-x86_64.tar.gz",
            "3163f7cf621beaf71d99b978555085e6bb933a0810153970dd16b2c531e1a030",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinybox-0.1.16-ubuntu-24.04-arm64.tar.gz",
            "fad323bf74ce075f758a31c7cb93f393d84ac4c9a6a22f31fb7fc7e2ec4743c4",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinybox-0.1.16-ubuntu-22.04-x86_64.tar.gz",
            "9abddc0e8ad14ac9f34e479714baa7f1720ed029d199272214450f356b7d51da",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinybox-0.1.16-ubuntu-22.04-arm64.tar.gz",
            "fb164eeec76789035de8c621176630b6ea6aea0a5021778fbd97fafe163b6dac",
        ),
        (
            "macos-26-arm64",
            "tinybox-0.1.16-macos-26-arm64.tar.gz",
            "34cb87457a3b21ec5ee8b8b6817f6a78a854b12e4d40fa43221aeed508d7c40a",
        ),
        (
            "macos-26-x86_64",
            "tinybox-0.1.16-macos-26-x86_64.tar.gz",
            "5f7a4886fb191a58dc33bb5f43a7034c0f6b31c1e85161e13228aa0deb6ae5b9",
        ),
        (
            "macos-15-arm64",
            "tinybox-0.1.16-macos-15-arm64.tar.gz",
            "ae427b7962b0607051595c9c12f9cb630c87672bc3c92c7becdac87283aaefd4",
        ),
        (
            "macos-15-x86_64",
            "tinybox-0.1.16-macos-15-x86_64.tar.gz",
            "417d4c182c3882cd90b5ff323dc81c62c2d98b3e0bb8ac0a237cac51c34828fe",
        ),
        (
            "windows-2025-x86_64",
            "tinybox-0.1.16-windows-2025-x86_64.zip",
            "ada7ddb1edf16887ef20d84241f0453b2ce8ee29aa670c1505758c1f7913b4fc",
        ),
        (
            "windows-2022-x86_64",
            "tinybox-0.1.16-windows-2022-x86_64.zip",
            "044666f53b10c8db6626262de45806d1a2a34147ce2de196e8549987f32c1cd8",
        ),
        (
            "windows-11-arm64",
            "tinybox-0.1.16-windows-11-arm64.zip",
            "bc3da524fc3e702e77cc1e85c064dccaece633f896503737c71ce7127ba26440",
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

fn supported_host_keys(record: &super::ModuleRecord) -> Vec<String> {
    every_host_key()
        .into_iter()
        .filter(|key| record.id != "tinycomputer" || !key.starts_with("ubuntu-"))
        .collect()
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
    assert!(find("tinydocs").is_some());
    assert!(find("not-a-module").is_none());
}
