use super::{find, ALL};
use tinybus::module::platform::candidates_for;

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
fn tinybox_registry_matches_the_published_v0135_release_manifest() {
    let record = find("tinybox").expect("compiled TinyBox module");
    assert_eq!(record.version, "0.1.35");
    assert_eq!(tinybox_bus::CONTRACT_VERSION, (1, 20));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.35"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-arm64",
            "tinybox-0.1.35-archlinux-arm64.tar.gz",
            "63f6c029317ab539a2c8f922d5cbb9bf9ee69fa185ff3ea94b1050c98f4537aa",
        ),
        (
            "archlinux-x86_64",
            "tinybox-0.1.35-archlinux-x86_64.tar.gz",
            "3f2b1fafd32819fecfe22c86423566ea47e416a5531d82e9c76ef2f3cb5cd0e1",
        ),
        (
            "fedora-43-arm64",
            "tinybox-0.1.35-fedora-43-arm64.tar.gz",
            "4064fff0f9d67acb8658f7d7669c9beb26f56c63f572fd406aac720b5817e5cf",
        ),
        (
            "fedora-43-x86_64",
            "tinybox-0.1.35-fedora-43-x86_64.tar.gz",
            "a62c55240465bef70d653fbc875b959dca49ef6cc37c472c1b798db047c7bf08",
        ),
        (
            "fedora-44-arm64",
            "tinybox-0.1.35-fedora-44-arm64.tar.gz",
            "c618d63c544890f370e21fb9d4278ab3ba4d65436c4665a0f96e5dfe709815d4",
        ),
        (
            "fedora-44-x86_64",
            "tinybox-0.1.35-fedora-44-x86_64.tar.gz",
            "e96d326d9c3b1572c1dabca60e5859d3ace02c0bfcc9b695937f2a1f083f5ecf",
        ),
        (
            "macos-15-arm64",
            "tinybox-0.1.35-macos-15-arm64.tar.gz",
            "93295b38243fb192191739dcca5ccec457561d27d9a9ead0ffc7633fe1f65771",
        ),
        (
            "macos-15-x86_64",
            "tinybox-0.1.35-macos-15-x86_64.tar.gz",
            "a4a63bbdc630dea58c03ec009dc9969ae9a07196e49a91cc5657345b9df89f54",
        ),
        (
            "macos-26-arm64",
            "tinybox-0.1.35-macos-26-arm64.tar.gz",
            "e416e25042a442ce01fceb18a5993eac7b702457a162d333981bb3eced16c50c",
        ),
        (
            "macos-26-x86_64",
            "tinybox-0.1.35-macos-26-x86_64.tar.gz",
            "7228eda7813faac69c25d52aaefd08ad96e7513288ac498dc5dcb8fc8831aa58",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinybox-0.1.35-ubuntu-22.04-arm64.tar.gz",
            "acf4b24d64ee7457311a651aa6649045a523322bb4d71fb614f0978777001deb",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinybox-0.1.35-ubuntu-22.04-x86_64.tar.gz",
            "d0d383df305f3d633063f01cf81c9ffcb3469f09532043b8be8a90f66f4b32d5",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinybox-0.1.35-ubuntu-24.04-arm64.tar.gz",
            "1a627a284532ba3cbd86689bc6961bbb38f8e5fa710eaade10d7f0d1d66fcd66",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinybox-0.1.35-ubuntu-24.04-x86_64.tar.gz",
            "6bcfb35c9c14777859f385575920e9d4a8a6af1a46a42370db464ab6de3c0067",
        ),
        (
            "windows-11-arm64",
            "tinybox-0.1.35-windows-11-arm64.zip",
            "def7bc85fcdf431d3b3a298f1f498b9674528be257725329df49c868b2893b9a",
        ),
        (
            "windows-2022-x86_64",
            "tinybox-0.1.35-windows-2022-x86_64.zip",
            "e2d2776969108a5f273c4f8ab309161ae495ff7cb7791dff869cc1c03ca40d5b",
        ),
        (
            "windows-2025-x86_64",
            "tinybox-0.1.35-windows-2025-x86_64.zip",
            "95801c1ee5a536e8dc6416d18426b82fc7c2be7d812b2acb1215f0b49c9dc14a",
        ),
    ];

    assert_eq!(actual.len(), 17);
    assert_eq!(actual, published);
}
#[test]
fn tinymcp_registry_matches_the_published_v0186_release_manifest() {
    let record = find("tinymcp").expect("compiled TinyMCP module");
    assert_eq!(record.version, "0.18.6");
    assert_eq!(tinymcp_bus::CONTRACT_VERSION, (1, 25));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.18.6"
    );
    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-rolling-arm64",
            "tinymcp-0.18.6-archlinux-arm64.tar.gz",
            "228601b94dab7fa92c386e51fe73e91efa9b5c196deb123a8812d9876b64187a",
        ),
        (
            "archlinux-rolling-x86_64",
            "tinymcp-0.18.6-archlinux-x86_64.tar.gz",
            "0d9b4ad6aeccda99e5b947a201fa3d378182ac388c83d488acc048fb8224abb4",
        ),
        (
            "fedora-43-arm64",
            "tinymcp-0.18.6-fedora-43-arm64.tar.gz",
            "30a90ca8abb15958e50eaaf5d7dfb5e68e2bc0baccf546d4cd62901e5dbd9d30",
        ),
        (
            "fedora-43-x86_64",
            "tinymcp-0.18.6-fedora-43-x86_64.tar.gz",
            "d348a868b44fe86bab42709ea66a41b728957f69a3f6f531bea1ee026ee04280",
        ),
        (
            "fedora-44-arm64",
            "tinymcp-0.18.6-fedora-44-arm64.tar.gz",
            "cbc0ec089cd23c8c1fceff15b872f186e1765f03a019096e90810193d8636cae",
        ),
        (
            "fedora-44-x86_64",
            "tinymcp-0.18.6-fedora-44-x86_64.tar.gz",
            "0a50f51af058ca4880466638ffba48552e2670f1107204a55810330995423b8a",
        ),
        (
            "macos-15-arm64",
            "tinymcp-0.18.6-macos-15-arm64.tar.gz",
            "9db24e582c1b36e094bea7d293be6d344de82450ffaeb00dbf6436e9ae8c9b51",
        ),
        (
            "macos-15-x86_64",
            "tinymcp-0.18.6-macos-15-x86_64.tar.gz",
            "95b17758fba9c93ab29b4f65bb25fe1ab6e0447f4991f65ca5720d56c6c50e30",
        ),
        (
            "macos-26-arm64",
            "tinymcp-0.18.6-macos-26-arm64.tar.gz",
            "070d048ca55e602024a8bf985c8d340b632af6676dda00638bcd26c58b5a636d",
        ),
        (
            "macos-26-x86_64",
            "tinymcp-0.18.6-macos-26-x86_64.tar.gz",
            "d4f813cfb21ed7d6288cf55b553173cd45100c6c0c55ca30df4fc91afe27c14a",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinymcp-0.18.6-ubuntu-22.04-arm64.tar.gz",
            "c317a9e99ad60c2e4a7fe3b42397d9dd14f24d0aa369a49ab479fb4c6423beff",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinymcp-0.18.6-ubuntu-22.04-x86_64.tar.gz",
            "c95e613930c97ab88fd32a5e83b8002373b79c7736faed6b95323ab82dd32a49",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinymcp-0.18.6-ubuntu-24.04-arm64.tar.gz",
            "8fbd069465a8246a1049eaf31e10140f354341c7e2cbefd2a3c6b2da110beacf",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinymcp-0.18.6-ubuntu-24.04-x86_64.tar.gz",
            "94a780a8268f61a056be03429c7a63a8911576157cce17133c9788b8b23276ba",
        ),
        (
            "windows-11-arm64",
            "tinymcp-0.18.6-windows-11-arm64.zip",
            "6fd81fbd5ebfafd0d3a32f552e45b4948aca0bf7364336c533aec94538ae4b51",
        ),
        (
            "windows-2022-x86_64",
            "tinymcp-0.18.6-windows-2022-x86_64.zip",
            "0319252c3b10184ee858597a6447f5b50f382ddd7e880e13be98f91bc50ea078",
        ),
        (
            "windows-2025-x86_64",
            "tinymcp-0.18.6-windows-2025-x86_64.zip",
            "c9ec10631ce1902d065f31e58ce2e4ecfbc2c91d2d8fe236ad1276da26c527a0",
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
