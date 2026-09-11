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
        for key in every_host_key() {
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
        let covered = every_host_key()
            .into_iter()
            .filter(|key| record.asset_for(key).is_some())
            .count();
        assert!(
            covered == 0 || covered == every_host_key().len(),
            "{} publishes assets for {covered} of {} host keys",
            record.id,
            every_host_key().len()
        );
    }
}

#[test]
fn find_resolves_known_ids_only() {
    assert!(find("tinydocs").is_some());
    assert!(find("not-a-module").is_none());
}
