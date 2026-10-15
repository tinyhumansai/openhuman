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
    assert_eq!(desktop.version, "0.14.0");
    assert_eq!(desktop.assets.len(), 17);
    assert_eq!(tinycomputer_bus::CONTRACT_VERSION, (2, 19));
    assert_eq!(
        desktop.asset_for("ubuntu-24.04-x86_64").unwrap().sha256,
        "fe79e93bf45d1ba3cd4212d04872530462a99d3d220da182070d54817e8291c6"
    );
    assert_eq!(
        desktop.asset_for("macos-26-arm64").unwrap().sha256,
        "05872f842caa298380a54ea9108aeca04152a70c78fe7177936f61d21b43bc38"
    );
}

#[test]
#[cfg(feature = "web3")]
fn tinywallet_registry_matches_the_verified_v0233_release_manifest() {
    let wallet = find("tinywallet").expect("compiled TinyWallet module");
    assert_eq!(wallet.version, "0.23.3");
    #[cfg(feature = "web3")]
    assert_eq!(tinywallet_bus::CONTRACT_VERSION, (1, 14));
    assert_eq!(
        wallet.release_url,
        "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.23.3"
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
            "tinywallet-module-0.23.3-archlinux-arm64.tar.gz",
            "e2adef42c2fcc9794cad1eefd0d5676cf2961a7772d0d3d5ca03890094f89924",
        ),
        (
            "archlinux-x86_64",
            "tinywallet-module-0.23.3-archlinux-x86_64.tar.gz",
            "2a35c9b79cfc15e1917e7e1957588511ffc0ec21b2ad167cbf9e53213ca4511c",
        ),
        (
            "fedora-43-arm64",
            "tinywallet-module-0.23.3-fedora-43-arm64.tar.gz",
            "4038195b53b79eda71a7716ab3e64b63904911915b7f44a967ceff274074a947",
        ),
        (
            "fedora-43-x86_64",
            "tinywallet-module-0.23.3-fedora-43-x86_64.tar.gz",
            "419a9a57d9ef245a852c49b32be4c07f61d58dd6315ae8c23d27ba1493615368",
        ),
        (
            "fedora-44-arm64",
            "tinywallet-module-0.23.3-fedora-44-arm64.tar.gz",
            "0a84fb19f6c1386b35b71677b52f30bf5524f0689a9afc9e79d17b1889d24ad7",
        ),
        (
            "fedora-44-x86_64",
            "tinywallet-module-0.23.3-fedora-44-x86_64.tar.gz",
            "4a4b8e13468ade4ef23303af53e9f5121cd3547faa9cf8accb086c0290ab98fb",
        ),
        (
            "macos-15-arm64",
            "tinywallet-module-0.23.3-macos-15-arm64.tar.gz",
            "aee3d767944171ed68cab981092d4ae5fa0678dc6d85f3e450bc618c42682af9",
        ),
        (
            "macos-15-x86_64",
            "tinywallet-module-0.23.3-macos-15-x86_64.tar.gz",
            "cc617f3443a15c69e1c3a2b3f94b1129262a4c2dbc76d3e76ae07b1153024bab",
        ),
        (
            "macos-26-arm64",
            "tinywallet-module-0.23.3-macos-26-arm64.tar.gz",
            "a3e16793b25abafbb2a26ed6b2223ce499f3f16da66bc01898307ca6fc049581",
        ),
        (
            "macos-26-x86_64",
            "tinywallet-module-0.23.3-macos-26-x86_64.tar.gz",
            "f2dc9871afcffdf7bac9f8fcbf9f4e13776e992ad94833855fd9d7944acc132d",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinywallet-module-0.23.3-ubuntu-22.04-arm64.tar.gz",
            "58e0105f0c151bbe3aa44aa0e29539d6762a71be73bff9875f8e1d389c13500b",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinywallet-module-0.23.3-ubuntu-22.04-x86_64.tar.gz",
            "4d9d732760e95e9b8bb614a3282746053b4b7e2f3c599a395215836505c15c91",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinywallet-module-0.23.3-ubuntu-24.04-arm64.tar.gz",
            "cec6a9eb6ca864140187463e29f65a7f374cb192d75edfcbead8a101b7bb7fff",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinywallet-module-0.23.3-ubuntu-24.04-x86_64.tar.gz",
            "f2ae10459327d55d837b2016fff448fb855cf57eeb139d7e9a173066c9b00921",
        ),
        (
            "windows-11-arm64",
            "tinywallet-module-0.23.3-windows-11-arm64.zip",
            "21d7c857b98359d27715c70ed9983ae4dd4138648b1aa55fa7a726c5896dc528",
        ),
        (
            "windows-2022-x86_64",
            "tinywallet-module-0.23.3-windows-2022-x86_64.zip",
            "ba0c2b7c3ba8e7dbe51c854643616d27c6a9433600265f7e7d0a6cf9a03a7a7a",
        ),
        (
            "windows-2025-x86_64",
            "tinywallet-module-0.23.3-windows-2025-x86_64.zip",
            "ce3748d7d885356040d2121357ad5b8945f28d05bde1ec65540c06524a9c328b",
        ),
    ];
    actual.sort_unstable();
    published.sort_unstable();
    assert_eq!(actual, published);
}

#[test]
fn tinybox_registry_matches_the_published_v0134_release_manifest() {
    let record = find("tinybox").expect("compiled TinyBox module");
    assert_eq!(record.version, "0.1.34");
    assert_eq!(tinybox_bus::CONTRACT_VERSION, (1, 19));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.34"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-arm64",
            "tinybox-0.1.34-archlinux-arm64.tar.gz",
            "d10557f46d0be57de5f337977189f5b9e793da404ab9c0ebdf0691a695f68c73",
        ),
        (
            "archlinux-x86_64",
            "tinybox-0.1.34-archlinux-x86_64.tar.gz",
            "96dda3c3b6b22bdb36f2bc0a6d01dda8798517d66b11225eb51af7c3598846d2",
        ),
        (
            "fedora-43-arm64",
            "tinybox-0.1.34-fedora-43-arm64.tar.gz",
            "38296b71d7c578b2a8e3f3555b2be46f69bea3659bdefbc83d9624f9433e8dc6",
        ),
        (
            "fedora-43-x86_64",
            "tinybox-0.1.34-fedora-43-x86_64.tar.gz",
            "c59a74303a973c046ceb205093fa02332d70f8c3c6c8442a1f73bd19655bf77c",
        ),
        (
            "fedora-44-arm64",
            "tinybox-0.1.34-fedora-44-arm64.tar.gz",
            "b6f50465f1fc28f4edc1d4d7f182c20cbf943da270debaf2ac22661d19969bb9",
        ),
        (
            "fedora-44-x86_64",
            "tinybox-0.1.34-fedora-44-x86_64.tar.gz",
            "a4ca0e455387927223a673b51e6c7203387e31d96a9f69fdbf35a9441bd81a24",
        ),
        (
            "macos-15-arm64",
            "tinybox-0.1.34-macos-15-arm64.tar.gz",
            "01d5685b15548732699db975f18143f050bbdc551d3b8d1580e457d8bf9b4b94",
        ),
        (
            "macos-15-x86_64",
            "tinybox-0.1.34-macos-15-x86_64.tar.gz",
            "dd57ecb79e342394d9847767e89143d59720f9a934803805e2968363d526bcb7",
        ),
        (
            "macos-26-arm64",
            "tinybox-0.1.34-macos-26-arm64.tar.gz",
            "d3a92cd86bd31d38a5f47bdefde9a643de33324d2d336949d10ab61b9349fd94",
        ),
        (
            "macos-26-x86_64",
            "tinybox-0.1.34-macos-26-x86_64.tar.gz",
            "9bc4875edc26b824014579b4723dacd6f29730bc7be961a25e8ae9de48c54970",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinybox-0.1.34-ubuntu-22.04-arm64.tar.gz",
            "99b925dcfd8cfffbd3d0239a3d0ef63bab6e4d88f28e29d3a7666031eb459ad9",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinybox-0.1.34-ubuntu-22.04-x86_64.tar.gz",
            "7db09dbbd8dd1b3a11dc40f1ea9d0c24e7c6e2c0feb14c4e859fa508a3fbab4f",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinybox-0.1.34-ubuntu-24.04-arm64.tar.gz",
            "8066e12a0ad477c66b28e2218a8118039bb1b2976184409d9e7b5f52e620edfc",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinybox-0.1.34-ubuntu-24.04-x86_64.tar.gz",
            "9f8e5765bd6e18450c9b47c0c92e8320d152e5170814b037f82b8afc30be6fa4",
        ),
        (
            "windows-11-arm64",
            "tinybox-0.1.34-windows-11-arm64.zip",
            "e53c68f35ebf63c6afebeabde7dbdad4a9b7b912b8e40600183f7b01695f05e6",
        ),
        (
            "windows-2022-x86_64",
            "tinybox-0.1.34-windows-2022-x86_64.zip",
            "820d751b12e9fe659319665da0b1c9ee5c07a207544761a20e0b0afdb7b065a4",
        ),
        (
            "windows-2025-x86_64",
            "tinybox-0.1.34-windows-2025-x86_64.zip",
            "84cec6888f732d6a202ea5a9a33f945323f55cf064557c5e1e4c49f2f5a936ff",
        ),
    ];

    assert_eq!(actual.len(), 17);
    assert_eq!(actual, published);
}
#[test]
fn tinymcp_registry_matches_the_published_v0185_release_manifest() {
    let record = find("tinymcp").expect("compiled TinyMCP module");
    assert_eq!(record.version, "0.18.5");
    assert_eq!(tinymcp_bus::CONTRACT_VERSION, (1, 24));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.18.5"
    );
    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-rolling-arm64",
            "tinymcp-0.18.5-archlinux-arm64.tar.gz",
            "0a3760807e225282677caba6a157e5c6cc16cfe509d22a8acc372a9a4c77f9d1",
        ),
        (
            "archlinux-rolling-x86_64",
            "tinymcp-0.18.5-archlinux-x86_64.tar.gz",
            "84f3bb050943bc040204b316c3540e31b25457b4f36112f0248f3a2bbd3f85c5",
        ),
        (
            "fedora-43-arm64",
            "tinymcp-0.18.5-fedora-43-arm64.tar.gz",
            "fae7790e4df4418eb8fff18818f5df04d69e2c307d3613b0fc177ad0d1810804",
        ),
        (
            "fedora-43-x86_64",
            "tinymcp-0.18.5-fedora-43-x86_64.tar.gz",
            "b8fe422717db38f63619bd108b2b63e9a3f3d40e8cb2ab653076d76d55f8bcf8",
        ),
        (
            "fedora-44-arm64",
            "tinymcp-0.18.5-fedora-44-arm64.tar.gz",
            "cc3f6cf33631e28921dd4483dd390988ab9a86806b0ce26274fc92fe0c56c53c",
        ),
        (
            "fedora-44-x86_64",
            "tinymcp-0.18.5-fedora-44-x86_64.tar.gz",
            "f92f83766f9f277ad8a8c4c9844cda5b1bcabc6404d8a133153cfeb941454f87",
        ),
        (
            "macos-15-arm64",
            "tinymcp-0.18.5-macos-15-arm64.tar.gz",
            "c706d86b21edcd0bfd6a9b01ac055f475904ba82a003be4dbdd8c715237901b9",
        ),
        (
            "macos-15-x86_64",
            "tinymcp-0.18.5-macos-15-x86_64.tar.gz",
            "b675c9777403c7215ce850ec79cdf3607d244631a0dd20eca7be4967455e548e",
        ),
        (
            "macos-26-arm64",
            "tinymcp-0.18.5-macos-26-arm64.tar.gz",
            "1296c0c405553eb30e4682dc304e657ebab2b7856a49f684e5a7aac6a287658b",
        ),
        (
            "macos-26-x86_64",
            "tinymcp-0.18.5-macos-26-x86_64.tar.gz",
            "4c34fb291190b4317ecf895869c38baa14689693e89d9af344642a222d683157",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinymcp-0.18.5-ubuntu-22.04-arm64.tar.gz",
            "404ec212c3b3143308ac7f892f558fbd1440e863e1f5a7cfeeb19d54c2bfcfad",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinymcp-0.18.5-ubuntu-22.04-x86_64.tar.gz",
            "bf569fb43def4f743c420701a4e9e39fc64679e1dceadac9e87894135d1b5013",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinymcp-0.18.5-ubuntu-24.04-arm64.tar.gz",
            "b1525e225a1481600624bd317c01cb2c13ccc4034860103fe775ff6ecda40803",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinymcp-0.18.5-ubuntu-24.04-x86_64.tar.gz",
            "f8c0b9dfc9a3a4b98314f5acf298c61b5ef96b3ce8605bad923a6bd642aefde6",
        ),
        (
            "windows-11-arm64",
            "tinymcp-0.18.5-windows-11-arm64.zip",
            "305bb0401e92752672c4db1eecedadf2c6bb720a868ab2316f97287048081b49",
        ),
        (
            "windows-2022-x86_64",
            "tinymcp-0.18.5-windows-2022-x86_64.zip",
            "6457801f2919cfe5e47dfe3577f3f163ad3f7567a814f99e51a37dee254f9c86",
        ),
        (
            "windows-2025-x86_64",
            "tinymcp-0.18.5-windows-2025-x86_64.zip",
            "2c2e2487c6b3fff2975df7b5928e688c46e42b9260afef02f72a0a77e1bc118a",
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
    assert_eq!(record.version, "0.1.34");
    assert_eq!(tinychannels_bus::CONTRACT_VERSION, 21);
    assert_eq!(
        tinychannels_bus::module_relay::MODULE_OPERATION_VERSION,
        (1, 21)
    );
    let expected = [
        (
            "macos-15-arm64",
            "tinychannels-module-0.1.34-macos-15-arm64.tar.gz",
            "b58f68ac45af467fa9eea0443afa59ed17a2f4e7ab9c3f4223ded6d83af88b22",
        ),
        (
            "macos-15-x86_64",
            "tinychannels-module-0.1.34-macos-15-x86_64.tar.gz",
            "ec04b49b184388b514ffc4aad9e130141987bd28d9cc5f4a76daf87c25e28a98",
        ),
        (
            "macos-26-arm64",
            "tinychannels-module-0.1.34-macos-26-arm64.tar.gz",
            "7b4f3be24e83a056e68d072cc1a5e00420d18f30eef0cd767cb6be40d0b2ad95",
        ),
        (
            "macos-26-x86_64",
            "tinychannels-module-0.1.34-macos-26-x86_64.tar.gz",
            "08e3d4ad69b30c46b06f4846133bb8066396e95f8f161f98c7d20d375b50453c",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinychannels-module-0.1.34-ubuntu-22.04-arm64.tar.gz",
            "035d782536952a7e60d3ba9011ee96cca71e63d515ef78e7397d4800ecf3aece",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinychannels-module-0.1.34-ubuntu-22.04-x86_64.tar.gz",
            "98292be5f921c027a79e41b463ffc9981fca139e9c269bb8daf882392c97817c",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinychannels-module-0.1.34-ubuntu-24.04-arm64.tar.gz",
            "57af460080ecdd14970e47dc02d8f27334cf165ab70ba3fb82e34dc88e445329",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinychannels-module-0.1.34-ubuntu-24.04-x86_64.tar.gz",
            "e153a65569f3e7aba90f78c3fe6ada4684280ea02776d5263a9df19071335fc7",
        ),
        (
            "windows-11-arm64",
            "tinychannels-module-0.1.34-windows-11-arm64.zip",
            "ef163bcd82fcdeafbb364f1f287eda0e9778255e04fb4062534ccf365f87bdeb",
        ),
        (
            "windows-2022-x86_64",
            "tinychannels-module-0.1.34-windows-2022-x86_64.zip",
            "a570821ef804e71ad98c747e51a22b8631a7d3db760e99b240cf666fafe67ced",
        ),
        (
            "windows-2025-x86_64",
            "tinychannels-module-0.1.34-windows-2025-x86_64.zip",
            "95bbf5aca663af06a6e6b912fa519e249fd1f454c65c0bb2993d1e1a79973316",
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
    assert_eq!(record.version, "0.3.2");
    assert_eq!(tinyvoice_bus::CONTRACT_VERSION, (1, 10));
    let expected = [
        (
            "macos-15-arm64",
            "tinyvoice-module-0.3.2-macos-15-arm64.tar.gz",
            "316cb2b9b9a1630f2c815744121e3bbc1d0df58364bb012dd6f11f78978ba947",
        ),
        (
            "macos-15-x86_64",
            "tinyvoice-module-0.3.2-macos-15-x86_64.tar.gz",
            "51045754d67386db5b09a19714ec8b1f319ad9896bb3f936325d7935ce5dce0e",
        ),
        (
            "macos-26-arm64",
            "tinyvoice-module-0.3.2-macos-26-arm64.tar.gz",
            "2a4ee8f87229422019adfd28f07ac26d5e2e78165550b04f0e563764a66a429e",
        ),
        (
            "macos-26-x86_64",
            "tinyvoice-module-0.3.2-macos-26-x86_64.tar.gz",
            "4199913e36c12987a82606f98c93e2a85e086aaa17380469a9aa68b719c853d9",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinyvoice-module-0.3.2-ubuntu-22.04-arm64.tar.gz",
            "533a3715f5efad9911e7768e8280bbe724c60785e0ff67939867ae21a20f690e",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinyvoice-module-0.3.2-ubuntu-22.04-x86_64.tar.gz",
            "d173a63a6b20e0441d18d639906ec9c74f96178c50e274ebd32bf94abbf5260b",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinyvoice-module-0.3.2-ubuntu-24.04-arm64.tar.gz",
            "f2145cd4bf729b5b6ad563687b061557d502c786ba935a65433e4338adf75a7a",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinyvoice-module-0.3.2-ubuntu-24.04-x86_64.tar.gz",
            "6ee278898d33f3b3ca7a9bf8a313dc4ab4e207eabf970608d37f56d825587519",
        ),
        (
            "windows-11-arm64",
            "tinyvoice-module-0.3.2-windows-11-arm64.zip",
            "b3cbff9b477495027839b0266201299b0db5cb585a9e41b3110cf46f16c35c99",
        ),
        (
            "windows-2022-x86_64",
            "tinyvoice-module-0.3.2-windows-2022-x86_64.zip",
            "7b7a8a8e04a57adc306b2b840bff43ea6a27090e2b089b74cfcf451f18172456",
        ),
        (
            "windows-2025-x86_64",
            "tinyvoice-module-0.3.2-windows-2025-x86_64.zip",
            "04cf6b21ed7d73375181c6cf5a18fa52a50fb09049031f0926cd46c3bcf33431",
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
