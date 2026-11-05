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
fn tinybox_registry_matches_the_published_v025_release_manifest() {
    let record = find("tinybox").expect("compiled TinyBox module");
    assert_eq!(record.version, "0.2.5");
    assert_eq!(tinybox_bus::CONTRACT_VERSION, (2, 3));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinybox/releases/tag/v0.2.5"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-arm64",
            "tinybox-0.2.5-archlinux-arm64.tar.gz",
            "05fcd49d54ed41cebca618f3f9cdf785ee1095275ce1127154cc84c528d51665",
        ),
        (
            "archlinux-x86_64",
            "tinybox-0.2.5-archlinux-x86_64.tar.gz",
            "5767fb8efccac06826487affd7085e761caebca665b076051da5b7d4effc7ea0",
        ),
        (
            "fedora-43-arm64",
            "tinybox-0.2.5-fedora-43-arm64.tar.gz",
            "4bbd52705e19fe55ab3784d5509ede2569de41a9ca0f64ffcc6fdb8741eab235",
        ),
        (
            "fedora-43-x86_64",
            "tinybox-0.2.5-fedora-43-x86_64.tar.gz",
            "645374ff97a0ebf645771fd0c7fd5c7ab11da80dac1b329a9417f14ec0d97a87",
        ),
        (
            "fedora-44-arm64",
            "tinybox-0.2.5-fedora-44-arm64.tar.gz",
            "61e277e5814cdbfb1e5f4aefcae4bffa011d8dd4d991ad102498b0fad1c440df",
        ),
        (
            "fedora-44-x86_64",
            "tinybox-0.2.5-fedora-44-x86_64.tar.gz",
            "c0120866b20b05fb31d11bcf8124786b38097c14fde80b5110914fb44c02fdda",
        ),
        (
            "macos-15-arm64",
            "tinybox-0.2.5-macos-15-arm64.tar.gz",
            "642a6f7776acdd8483da49fbd1157663e19ee681cd85e69c90d7eae5c0cafbd5",
        ),
        (
            "macos-15-x86_64",
            "tinybox-0.2.5-macos-15-x86_64.tar.gz",
            "0fc96965cbe8cd8dd21e59988cc638d04983eed98a92ef6ad48645eea962de4e",
        ),
        (
            "macos-26-arm64",
            "tinybox-0.2.5-macos-26-arm64.tar.gz",
            "e6832b67ab5ebdeb82bc77168556cd1da79d64d44a6979c2a0dcd6f594f6163c",
        ),
        (
            "macos-26-x86_64",
            "tinybox-0.2.5-macos-26-x86_64.tar.gz",
            "d3b6c7a5c1d08baf8fd1c2350eadd2410490f5b05ce4fd4df6050961b45266cd",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinybox-0.2.5-ubuntu-22.04-arm64.tar.gz",
            "595b8f9dbce5459429d2f3b5bb65f28114990e8a2c2ae36e77b1852772d0ff84",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinybox-0.2.5-ubuntu-22.04-x86_64.tar.gz",
            "2ed696818a50df69fb8e823ad07497e505727798d828335f52828332ef9b0f9b",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinybox-0.2.5-ubuntu-24.04-arm64.tar.gz",
            "26b31a46ca0a24d213de63afe21214f2f44d74b14837bf8e6c8b4c9e18f26271",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinybox-0.2.5-ubuntu-24.04-x86_64.tar.gz",
            "a369579d000ca74ef2967681634330c96221305ead5c1f433d072f367898a161",
        ),
        (
            "windows-11-arm64",
            "tinybox-0.2.5-windows-11-arm64.zip",
            "69c79ca7f7dc9215353e329b20748f0c72ad278aa6bc1d55a2340f9d37a9b9fe",
        ),
        (
            "windows-2022-x86_64",
            "tinybox-0.2.5-windows-2022-x86_64.zip",
            "c6b322e2caf40273cd0c4ad6dc766d832e32c80dd88c1ab795f9cac566f5295b",
        ),
        (
            "windows-2025-x86_64",
            "tinybox-0.2.5-windows-2025-x86_64.zip",
            "c49fe369511c4f0ed6dfc47f7cedff439a50b6a64b4b5fda7c40b722c9fdec55",
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
    assert_eq!(search.version, "0.5.6");
    assert_eq!(tinysearch_bus::CONTRACT_VERSION, (2, 2));
    assert_eq!(
        search.asset_for("ubuntu-24.04-x86_64").unwrap().sha256,
        "75e517367ddc3f4e76c7632566498e8abf5f317b7b84c115ce9f1287c65dca85"
    );
}
