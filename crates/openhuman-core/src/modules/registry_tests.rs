use super::{find, ALL};
use tinybus::module::platform::candidates_for;

#[path = "registry_computer_release_tests.rs"]
mod computer_release_tests;

#[path = "registry_release_manifest_tests.rs"]
mod release_manifest_tests;

#[test]
#[cfg(feature = "web3")]
fn tinywallet_registry_matches_the_verified_v0290_release_manifest() {
    let wallet = find("tinywallet").expect("compiled TinyWallet module");
    assert_eq!(wallet.version, "0.29.0");
    #[cfg(feature = "web3")]
    assert_eq!(tinywallet_bus::CONTRACT_VERSION, (1, 15));
    assert_eq!(
        wallet.release_url,
        "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.29.0"
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
            "tinywallet-module-0.29.0-archlinux-arm64.tar.gz",
            "90d8cb352a9d6b6403549f053446139c0361d36354ca7c37e0bc0296eb544cae",
        ),
        (
            "archlinux-x86_64",
            "tinywallet-module-0.29.0-archlinux-x86_64.tar.gz",
            "8e9bc6e2d443807cbbb1759cb85896abbcc0d3294a0ec58007fe0c56aecb5a25",
        ),
        (
            "fedora-43-arm64",
            "tinywallet-module-0.29.0-fedora-43-arm64.tar.gz",
            "71e8ec37af5be258c111169e441fed5f4a46d9b3934c477569ec1185ec7ceba5",
        ),
        (
            "fedora-43-x86_64",
            "tinywallet-module-0.29.0-fedora-43-x86_64.tar.gz",
            "3e7438212776adbe82a54a5c48ff744cd26ee73d0dee66bce7de42b968608a16",
        ),
        (
            "fedora-44-arm64",
            "tinywallet-module-0.29.0-fedora-44-arm64.tar.gz",
            "7612aeeca130c741b38673279268494e0cf49891dd95574e14310e64634a6840",
        ),
        (
            "fedora-44-x86_64",
            "tinywallet-module-0.29.0-fedora-44-x86_64.tar.gz",
            "9807227a79989d6c17be18212462751e244fb1b5f2bbe1224e0054b3f37cc188",
        ),
        (
            "macos-15-arm64",
            "tinywallet-module-0.29.0-macos-15-arm64.tar.gz",
            "228b25d3ea7869436cf009bc228dc94f8f40fcf1d7ea81c622651c14b6bbc216",
        ),
        (
            "macos-15-x86_64",
            "tinywallet-module-0.29.0-macos-15-x86_64.tar.gz",
            "0b173d7ebed6ce23c7224a55a32c9b9716471a054119b02869abc2add15dc391",
        ),
        (
            "macos-26-arm64",
            "tinywallet-module-0.29.0-macos-26-arm64.tar.gz",
            "61ee2de3ca95cea7f8253de3e47f4da9ae42bd4ac406c65374aaf4cd22b742f3",
        ),
        (
            "macos-26-x86_64",
            "tinywallet-module-0.29.0-macos-26-x86_64.tar.gz",
            "5c7593c4bb15f806a83c25dee97e26bf86a73571046d6523164885804d0c9c7d",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinywallet-module-0.29.0-ubuntu-22.04-arm64.tar.gz",
            "54085d6e1391f98fb44b86735df0f147318502153c4d3e96c5586421835cb5aa",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinywallet-module-0.29.0-ubuntu-22.04-x86_64.tar.gz",
            "4d2016fac3c2001dae9ff152dad91057741aa6c031093b10fce4a4f3e1654045",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinywallet-module-0.29.0-ubuntu-24.04-arm64.tar.gz",
            "dedb29838c947d2da479c9998a38d772de875bd914eae0d0a8b81377bb7b894c",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinywallet-module-0.29.0-ubuntu-24.04-x86_64.tar.gz",
            "74036ca124938a556687ff760d83fb3ad2eebb9e91a2a9cb93873a537f8e85a3",
        ),
        (
            "windows-11-arm64",
            "tinywallet-module-0.29.0-windows-11-arm64.zip",
            "5fd451a1166e64af8f5936239f22ca88deadb6ff96058e7b9295887fa8315999",
        ),
        (
            "windows-2022-x86_64",
            "tinywallet-module-0.29.0-windows-2022-x86_64.zip",
            "b272c66ade706b7cb3c443fbd05c549abc2ef024f044b1cdfeba19f61367db78",
        ),
        (
            "windows-2025-x86_64",
            "tinywallet-module-0.29.0-windows-2025-x86_64.zip",
            "2a6b49b81af30bbe6975c7274a9c2f8cef2a3bde10a3cebae1f4c8b9c1bf5333",
        ),
    ];
    actual.sort_unstable();
    published.sort_unstable();
    assert_eq!(actual, published);
}

#[test]
fn tinybox_registry_matches_the_published_v026_release_manifest() {
    let record = find("tinybox").expect("compiled TinyBox module");
    assert_eq!(record.version, "0.2.6");
    assert_eq!(tinybox_bus::CONTRACT_VERSION, (2, 4));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinybox/releases/tag/v0.2.6"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-arm64",
            "tinybox-0.2.6-archlinux-arm64.tar.gz",
            "284cda47d58de498a02f9643243746deb858d8f71c8f54ccac7275f0965e3be1",
        ),
        (
            "archlinux-x86_64",
            "tinybox-0.2.6-archlinux-x86_64.tar.gz",
            "55f4ecda62225df11f6e546c13851c9a1a78ed3037f99394b955816f79ffce6b",
        ),
        (
            "fedora-43-arm64",
            "tinybox-0.2.6-fedora-43-arm64.tar.gz",
            "2e4d134f8dbdef9b41fba43036e5803babebb3d0f24bdfff341c8f29d25ae0d8",
        ),
        (
            "fedora-43-x86_64",
            "tinybox-0.2.6-fedora-43-x86_64.tar.gz",
            "57a259f7297f5a7e20e9f9b8cb47c57d70a4edbaf6f6ea0d4c33f5f6321a66dc",
        ),
        (
            "fedora-44-arm64",
            "tinybox-0.2.6-fedora-44-arm64.tar.gz",
            "3ab57d75c478091384e23019e8fe7807ffddaaeb436b52b8d9604ac0336d9ceb",
        ),
        (
            "fedora-44-x86_64",
            "tinybox-0.2.6-fedora-44-x86_64.tar.gz",
            "6b9f65d8501ff0a32ee60f13c22e6501696d9722fd2951f73fbe5a796bb21eb9",
        ),
        (
            "macos-15-arm64",
            "tinybox-0.2.6-macos-15-arm64.tar.gz",
            "8d62d2e86c41a2220ca6c4d989fb116fb5e09664b6ead2b0cdccb984db50b068",
        ),
        (
            "macos-15-x86_64",
            "tinybox-0.2.6-macos-15-x86_64.tar.gz",
            "8e4f2ebde00a6f8a06445b310df7417de5877c8bdb7d6fbe7ae5653d003f7afa",
        ),
        (
            "macos-26-arm64",
            "tinybox-0.2.6-macos-26-arm64.tar.gz",
            "a9c14971de977c02202197a57329de9c73275e09b13cdaa4b2726472b711f718",
        ),
        (
            "macos-26-x86_64",
            "tinybox-0.2.6-macos-26-x86_64.tar.gz",
            "c63f858405bdbe38f6635b77cb79cc45b11fbe186e69ca03b5d88cb2a7df7696",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinybox-0.2.6-ubuntu-22.04-arm64.tar.gz",
            "a8b79919554b603022b1acf028cc75594b999569b242f12630ec0f1c13ce7a8b",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinybox-0.2.6-ubuntu-22.04-x86_64.tar.gz",
            "9df93f7112ce5f05285446c75891d886dfe9228c7f2b42620f8d9ec68ec9e0a1",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinybox-0.2.6-ubuntu-24.04-arm64.tar.gz",
            "422bec85c4e45953fc830aab4d7f9774e91df46c533b53028db6eb51b7a982f8",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinybox-0.2.6-ubuntu-24.04-x86_64.tar.gz",
            "576c03733c70bba869647ab1fc137b6b941aee977c322494df0e72e4194705ee",
        ),
        (
            "windows-11-arm64",
            "tinybox-0.2.6-windows-11-arm64.zip",
            "7400205c377b2803ba89a8be5e78cb06307da113d452582be8cfe04afd7c877c",
        ),
        (
            "windows-2022-x86_64",
            "tinybox-0.2.6-windows-2022-x86_64.zip",
            "4cad9cad7f79502757583d46134ee25bf80d0d9745d889f91e3e68b98bc1cd0a",
        ),
        (
            "windows-2025-x86_64",
            "tinybox-0.2.6-windows-2025-x86_64.zip",
            "a21e2d12a1ad844abad51f584c9ffb6fa5a8bb9c5900f8713a4f89e5bd70dfaf",
        ),
    ];

    assert_eq!(actual.len(), 17);
    assert_eq!(actual, published);
}
#[test]
fn tinymcp_registry_matches_the_published_v0206_release_manifest() {
    let record = find("tinymcp").expect("compiled TinyMCP module");
    assert_eq!(record.version, "0.20.6");
    assert_eq!(tinymcp_bus::CONTRACT_VERSION, (1, 28));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.20.6"
    );
    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-rolling-arm64",
            "tinymcp-0.20.6-archlinux-arm64.tar.gz",
            "af521ed1dcc7bf781a5def1d99356b529bb5bfafc38295d4fa370884b4f4946c",
        ),
        (
            "archlinux-rolling-x86_64",
            "tinymcp-0.20.6-archlinux-x86_64.tar.gz",
            "8968cdeca99b4282c0ee17b04817552a51b8341589d437ba4c76782872ab6fbb",
        ),
        (
            "fedora-43-arm64",
            "tinymcp-0.20.6-fedora-43-arm64.tar.gz",
            "c5dc43b7f923ae23009f449c9ec62fbe3b474834b775e3c22b6266fd23cd891b",
        ),
        (
            "fedora-43-x86_64",
            "tinymcp-0.20.6-fedora-43-x86_64.tar.gz",
            "598ab75907d76a829e41e8d3efe78e9f65bfb74e0fd565b07d861ed9c7b9bed5",
        ),
        (
            "fedora-44-arm64",
            "tinymcp-0.20.6-fedora-44-arm64.tar.gz",
            "7cf36b64af0b22723f4cea25ba5e9d08219fbeb26885fddf002f5669f963da93",
        ),
        (
            "fedora-44-x86_64",
            "tinymcp-0.20.6-fedora-44-x86_64.tar.gz",
            "388e9ee9da761a608dcb4eeb433b56d5bace5c099e4fe0fa155889777903a74e",
        ),
        (
            "macos-15-arm64",
            "tinymcp-0.20.6-macos-15-arm64.tar.gz",
            "de7ea2933772e1addf207480850e7eeab4c18d8a410d1b62e4c02a3e42cfc26b",
        ),
        (
            "macos-15-x86_64",
            "tinymcp-0.20.6-macos-15-x86_64.tar.gz",
            "f2c56a2fe9b3762e4879ccb1d7bac4a101e2c758ed2726bf9b6aa5baf6dffeb7",
        ),
        (
            "macos-26-arm64",
            "tinymcp-0.20.6-macos-26-arm64.tar.gz",
            "b2117c4dc28c0846c395b0e7564354476b55fdff867c5698f38a967e29ea03d3",
        ),
        (
            "macos-26-x86_64",
            "tinymcp-0.20.6-macos-26-x86_64.tar.gz",
            "ef26b7c7fed734c0d31a6d40259e4c500c5ed6bfde367dcc8fcfee7d0a32bd71",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinymcp-0.20.6-ubuntu-22.04-arm64.tar.gz",
            "e0eaed91578641ba6548fae2a98a95aa6077eddfd0c6f82186df3ba10ce71f39",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinymcp-0.20.6-ubuntu-22.04-x86_64.tar.gz",
            "f8634b838e8329ffbd056a109abd9ad8b1bd8e89b6bb78bbb85c52d5e56f66a9",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinymcp-0.20.6-ubuntu-24.04-arm64.tar.gz",
            "06cbb7c1e1040d8fa5fd1a8f11b57405e389d08478df3c66b161ab471984d7f5",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinymcp-0.20.6-ubuntu-24.04-x86_64.tar.gz",
            "b295e155e819ac56dd2c1e1fd8c895012ea442d77a35b552f1096703a2c7cc37",
        ),
        (
            "windows-11-arm64",
            "tinymcp-0.20.6-windows-11-arm64.zip",
            "37fbec7c0d8c040ac6278cff59c75aeb3f4cefb3f4cfb05cab576e8f8d6712b5",
        ),
        (
            "windows-2022-x86_64",
            "tinymcp-0.20.6-windows-2022-x86_64.zip",
            "41572a71adde15fcadf32d64687da34cf31947123b4e132acd7189e432f50c07",
        ),
        (
            "windows-2025-x86_64",
            "tinymcp-0.20.6-windows-2025-x86_64.zip",
            "df9a8467b16782ccba2634910714d24f940e2366243537efa24edc88b0712434",
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
    assert_eq!(search.version, "0.5.7");
    assert_eq!(tinysearch_bus::CONTRACT_VERSION, (2, 2));
    assert_eq!(
        search.asset_for("ubuntu-24.04-x86_64").unwrap().sha256,
        "80b3eff3095160b126f44949c500818a1765ea41512c2ca1ea9c3b361875ecb8"
    );
}
