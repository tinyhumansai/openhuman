use super::{find, ALL};
use tinybus::module::platform::candidates_for;

#[test]
fn tinycomputer_registry_matches_bus_contract_and_published_release() {
    let desktop = find("tinycomputer").expect("compiled computer module");
    assert_eq!(desktop.bus_name, tinycomputer_bus::names::INTERFACE);
    assert_eq!(desktop.object_path, tinycomputer_bus::names::OBJECT_PATH);
    assert_eq!(desktop.version, "0.11.0");
    assert_eq!(desktop.assets.len(), 16);
    assert_eq!(tinycomputer_bus::CONTRACT_VERSION, (2, 12));
    assert_eq!(
        desktop.asset_for("ubuntu-24.04-x86_64").unwrap().sha256,
        "430426bdbc6f8ad8cc5ed17d6f3bd05f73b4d4fe747a8fc9b77f0b3f4fd3506b"
    );
    assert_eq!(
        desktop.asset_for("macos-26-arm64").unwrap().sha256,
        "3a8e20a809262be2154c1b7339fed2a627c13c32b2a7b9de52e4d54ced94a994"
    );
}

#[test]
fn tinywallet_registry_matches_the_verified_v013_release_manifest() {
    let wallet = find("tinywallet").expect("compiled TinyWallet module");
    assert_eq!(wallet.version, "0.13.0");
    assert_eq!(
        wallet.release_url,
        "https://github.com/tinyhumansai/tinywallet/releases/tag/v0.13.0"
    );

    let actual = wallet
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "ubuntu-24.04-x86_64",
            "tinywallet-module-0.13.0-ubuntu-24.04-x86_64.tar.gz",
            "284983d01c548109b3e9d5bbca1951787912f713b4a98920b9b45cfd034b6760",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinywallet-module-0.13.0-ubuntu-24.04-arm64.tar.gz",
            "18ef8ffe518f731dc594d1606554ca12c840219e54f230382937061c395ab860",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinywallet-module-0.13.0-ubuntu-22.04-x86_64.tar.gz",
            "92d0d0fe9ab20fdf4c44aa58d762b7e28b6a08876899a5b2d368df3feabcd2d5",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinywallet-module-0.13.0-ubuntu-22.04-arm64.tar.gz",
            "4d0e17d778c0c84df17e4c5b7fa60aa9296efecc5726902008946409b3a3c9e1",
        ),
        (
            "macos-26-arm64",
            "tinywallet-module-0.13.0-macos-26-arm64.tar.gz",
            "e01a70efbcece8e134abfa84c70181425802415525303d7ba2a7798afbab1041",
        ),
        (
            "macos-26-x86_64",
            "tinywallet-module-0.13.0-macos-26-x86_64.tar.gz",
            "c48f1f7807d0b25c2b54dca97237f6e64d02ec3a9396370cd8e3b1857419296a",
        ),
        (
            "macos-15-arm64",
            "tinywallet-module-0.13.0-macos-15-arm64.tar.gz",
            "95f69f2e17ff1785d032f6e1017e12de5c6dc09af9633e67fc9c225dfcef9ce6",
        ),
        (
            "macos-15-x86_64",
            "tinywallet-module-0.13.0-macos-15-x86_64.tar.gz",
            "f3114694e2d666ea1b030db952c589f08d79c60ab79aadb844327e273c5faaf5",
        ),
        (
            "windows-2025-x86_64",
            "tinywallet-module-0.13.0-windows-2025-x86_64.zip",
            "7b79a25a1fccef17a4c6e0d400f49646c3a5f321821bb03ddc23e76d6030717a",
        ),
        (
            "windows-2022-x86_64",
            "tinywallet-module-0.13.0-windows-2022-x86_64.zip",
            "0b7126be04b23dd5fad883d7f1dd33dd65c1042af68f93593590ea07c5daa95e",
        ),
        (
            "windows-11-arm64",
            "tinywallet-module-0.13.0-windows-11-arm64.zip",
            "483d3fda2c46de71c8efd5dfa4ec828c0b6c86bb164c8362c1356f85cb8ec231",
        ),
        (
            "archlinux-arm64",
            "tinywallet-module-0.13.0-archlinux-arm64.tar.gz",
            "ad2222bd1d2dd680568f486e40389c0b963cf5ba2d170f9d658985f31cabb511",
        ),
        (
            "archlinux-x86_64",
            "tinywallet-module-0.13.0-archlinux-x86_64.tar.gz",
            "d67f541330f7712106238b90d993550d2e2e55f33d3c8c9f50669a1a51c1e981",
        ),
        (
            "fedora-43-arm64",
            "tinywallet-module-0.13.0-fedora-43-arm64.tar.gz",
            "a2d62b03f2e99008ee3fe7ea84290fd050e12016ce36e89fda24d5449e795261",
        ),
        (
            "fedora-43-x86_64",
            "tinywallet-module-0.13.0-fedora-43-x86_64.tar.gz",
            "0c8c5dbf2aa7b7caa371aa6366c7cfdd232fda41e2dbd894d2ef344c7c2cfa0a",
        ),
        (
            "fedora-44-arm64",
            "tinywallet-module-0.13.0-fedora-44-arm64.tar.gz",
            "272b535910fc4bc84099887b7ccfbdd9d41a6d187616086c174f86cb77f21757",
        ),
        (
            "fedora-44-x86_64",
            "tinywallet-module-0.13.0-fedora-44-x86_64.tar.gz",
            "3dcad60a65d4f0bbb1af838468a2f52d648d22b2fbc849ae0548a30ed9806646",
        ),
    ];
    assert_eq!(actual, published);
}

#[test]
fn tinybox_registry_matches_the_published_v0119_release_manifest() {
    let record = find("tinybox").expect("compiled TinyBox module");
    assert_eq!(record.version, "0.1.19");
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinybox/releases/tag/v0.1.19"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "ubuntu-24.04-x86_64",
            "tinybox-0.1.19-ubuntu-24.04-x86_64.tar.gz",
            "a1dae0e4af4a1ba84fc37b048ad1d1ef0e226ec2c2b312d7607a8ec08d09d134",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinybox-0.1.19-ubuntu-24.04-arm64.tar.gz",
            "245eaf3ab5465e9e01da207ef63bf930c46069b3858fe9ce9da8e71559c0faf8",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinybox-0.1.19-ubuntu-22.04-x86_64.tar.gz",
            "c23102c295b8e5ceef428d1ed178ac910031a23640eaebceb7ebfe3c19a2e05c",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinybox-0.1.19-ubuntu-22.04-arm64.tar.gz",
            "b81e147875a5f21b9b681dffa633c480cfa0bf0121e3ea309feb859d7206d866",
        ),
        (
            "macos-26-arm64",
            "tinybox-0.1.19-macos-26-arm64.tar.gz",
            "437ae2b09337b30ad1af85f3fe95be5cc0d8134986f401a231448a37c340f54b",
        ),
        (
            "macos-26-x86_64",
            "tinybox-0.1.19-macos-26-x86_64.tar.gz",
            "24c3397596e20e3661e6fc6d0ba28f2f36a8c3d7f4e5106030f674d5d3703f6d",
        ),
        (
            "macos-15-arm64",
            "tinybox-0.1.19-macos-15-arm64.tar.gz",
            "61145d8dbb56d866d85610702938973b7c8f523093154ec3a27790dc060565bd",
        ),
        (
            "macos-15-x86_64",
            "tinybox-0.1.19-macos-15-x86_64.tar.gz",
            "8e68322fe3ff762ad6b31a55413cfd7772174b2bdadc0b5391d1816d36a6b373",
        ),
        (
            "windows-2025-x86_64",
            "tinybox-0.1.19-windows-2025-x86_64.zip",
            "8ac5b73f23deee35d2f61209b801a5fcb6465277e35afc45dac4e91cfe528ce0",
        ),
        (
            "windows-2022-x86_64",
            "tinybox-0.1.19-windows-2022-x86_64.zip",
            "f2682b28908c9736382fab57c0f579db4a17ae3e037d9422f185a0f949e69143",
        ),
        (
            "windows-11-arm64",
            "tinybox-0.1.19-windows-11-arm64.zip",
            "50873a041c05d009f8f65812e6ac95fef001b1d71a897383da1f34fac1f2a7d0",
        ),
    ];

    assert_eq!(actual, published);
}

#[test]
fn tinymcp_registry_matches_the_published_v090_release_manifest() {
    let record = find("tinymcp").expect("compiled TinyMCP module");
    assert_eq!(record.version, "0.9.0");
    assert_eq!(tinymcp_bus::CONTRACT_VERSION, (1, 13));
    assert_eq!(
        record.release_url,
        "https://github.com/tinyhumansai/tinymcp/releases/tag/v0.9.0"
    );

    let actual = record
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let published = [
        (
            "archlinux-rolling-x86_64",
            "tinymcp-0.9.0-archlinux-rolling-x86_64.tar.gz",
            "57a72dac5b7e69444ba10688d5e3516cb4d5c0760663a59774c94060952c744f",
        ),
        (
            "fedora-43-arm64",
            "tinymcp-0.9.0-fedora-43-arm64.tar.gz",
            "6e5b4623debc566d1c1ed7dacbc1d81330e9d63723fcfd67753cb8af73e073cd",
        ),
        (
            "fedora-43-x86_64",
            "tinymcp-0.9.0-fedora-43-x86_64.tar.gz",
            "b5e27c097d3f694d40343ea61c4acc4b3aa0fed13630a9d63dcd339c00823fd0",
        ),
        (
            "fedora-44-arm64",
            "tinymcp-0.9.0-fedora-44-arm64.tar.gz",
            "1908e9fa46bf3b0ecc3917c45e9b98064a0d162833c5331831b896a32526ff3f",
        ),
        (
            "fedora-44-x86_64",
            "tinymcp-0.9.0-fedora-44-x86_64.tar.gz",
            "7937d3a6bc549cffdfb70f89e21dc4a26057c0700ed2a02cb08cfa5252869655",
        ),
        (
            "macos-15-arm64",
            "tinymcp-0.9.0-macos-15-arm64.tar.gz",
            "bceb6a3b50f78285e47d319951e92d279cc99da153dc660e671b6df9190dd197",
        ),
        (
            "macos-15-x86_64",
            "tinymcp-0.9.0-macos-15-x86_64.tar.gz",
            "0310c91e11dc44c93609e2003057a18b20919b0aee739e9b99ab8911350af334",
        ),
        (
            "macos-26-arm64",
            "tinymcp-0.9.0-macos-26-arm64.tar.gz",
            "6083044b9e2b38733014570fa91068f74ac23120cc4ef790c934984bc1812640",
        ),
        (
            "macos-26-x86_64",
            "tinymcp-0.9.0-macos-26-x86_64.tar.gz",
            "6d8c90efb63743523fa901d74b0e8e6939d30bacd0a96d77650a1af61602ac9d",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinymcp-0.9.0-ubuntu-22.04-arm64.tar.gz",
            "53bbdc087a444e0deb1e770ad4ea9cc23f77ffc4db6072124da819543dfabfd4",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinymcp-0.9.0-ubuntu-22.04-x86_64.tar.gz",
            "c1c1255bedd611ba32f70beb82c02452cba7dbe9a08de5d0fd3a2dde0216003d",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinymcp-0.9.0-ubuntu-24.04-arm64.tar.gz",
            "b1fb9348917a4f828768fd41e92fa7b52eb010cb90022cb3be66800c06a95857",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinymcp-0.9.0-ubuntu-24.04-x86_64.tar.gz",
            "38681621198eb5bfaf170154c619ae8c3dcfae1d233ba82c2cad039d165fe557",
        ),
        (
            "windows-11-arm64",
            "tinymcp-0.9.0-windows-11-arm64.zip",
            "6e8c65385acb52ff116ba08e29a895434114e46d3379a019953d7013bc22e38b",
        ),
        (
            "windows-2022-x86_64",
            "tinymcp-0.9.0-windows-2022-x86_64.zip",
            "4a821cabaeba893ecb2358e3eef92e3392e18def07e9b59a4865ae1e8a232f57",
        ),
        (
            "windows-2025-x86_64",
            "tinymcp-0.9.0-windows-2025-x86_64.zip",
            "4a18d8a76f9be30bf64224c5a2e7165f70716bd584e3b8fbaa88f5e70854905b",
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
    assert!(find("tinydocs").is_some());
    assert!(find("not-a-module").is_none());
}

#[test]
fn tinychannels_registry_matches_health_check_release() {
    let record = find("tinychannels").expect("compiled TinyChannels module");
    assert_eq!(record.version, "0.1.18");
    assert_eq!(tinychannels_bus::CONTRACT_VERSION, 8);
    assert_eq!(
        tinychannels_bus::module_relay::MODULE_OPERATION_VERSION,
        (1, 9)
    );
    let expected = [
        (
            "macos-15-arm64",
            "tinychannels-module-0.1.18-macos-15-arm64.tar.gz",
            "f152d03721ddd9ab5b0074f55b29958a141e023a0657e0f158f7f16070055607",
        ),
        (
            "macos-15-x86_64",
            "tinychannels-module-0.1.18-macos-15-x86_64.tar.gz",
            "2492e497f45db161f588ff625ee18a59b0edaa8cc8e1e36e94a30e2604b48021",
        ),
        (
            "macos-26-arm64",
            "tinychannels-module-0.1.18-macos-26-arm64.tar.gz",
            "e5cdd8318e6c7d71a514e71c36ade1f0833a1ede250fb0026d3c4fce9a1c1f18",
        ),
        (
            "macos-26-x86_64",
            "tinychannels-module-0.1.18-macos-26-x86_64.tar.gz",
            "13fec100c1bcf92cb4713fc7e7c83dc5e0a148d78ad7b9c7f71f48f94984c927",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinychannels-module-0.1.18-ubuntu-22.04-arm64.tar.gz",
            "fd0ff1d518b24e692d00cd387d2f721808f0d682fe2c98a91e98293a150337ad",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinychannels-module-0.1.18-ubuntu-22.04-x86_64.tar.gz",
            "ecc1bff5be6adf6ac6deb210523429fabe8d3ad2e877d7885f374c9cd21d0ac4",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinychannels-module-0.1.18-ubuntu-24.04-arm64.tar.gz",
            "120816212af496e9617e478dad2a1fc0f007bd01e507c1613d8f9255b6006500",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinychannels-module-0.1.18-ubuntu-24.04-x86_64.tar.gz",
            "24efc6a352339afed5a0bec99304fd9567defdd73f13cc350471ce83c7b3e8b8",
        ),
        (
            "windows-11-arm64",
            "tinychannels-module-0.1.18-windows-11-arm64.zip",
            "806aa6b642dbc618f010d4eef3e7a7aef37d45bbfdfec311e1c2a908f5973b60",
        ),
        (
            "windows-2022-x86_64",
            "tinychannels-module-0.1.18-windows-2022-x86_64.zip",
            "9433165bf99c27b834a5f9366d78709d3eb15c7acede4a8064e62af84d1da8fe",
        ),
        (
            "windows-2025-x86_64",
            "tinychannels-module-0.1.18-windows-2025-x86_64.zip",
            "4f6d659cca8559093aa91f4a49d0b718f4087d242473358e67ae1e35714e0682",
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
    assert_eq!(record.version, "0.1.16");
    assert_eq!(tinyvoice_bus::CONTRACT_VERSION, (1, 5));
    let expected = [
        (
            "macos-15-arm64",
            "tinyvoice-module-0.1.16-macos-15-arm64.tar.gz",
            "72c75b371e72c22da9a149b0962395d74131b41910e2ed735d127667f9f38c1d",
        ),
        (
            "macos-15-x86_64",
            "tinyvoice-module-0.1.16-macos-15-x86_64.tar.gz",
            "a293f24b14570ac551be300d1aac28eddb0339066ab4ec0a231b6831fade4734",
        ),
        (
            "macos-26-arm64",
            "tinyvoice-module-0.1.16-macos-26-arm64.tar.gz",
            "8003a988fdc2eb4a6be724caf6d1ba16a97d3bbf7372921630bb180f02dfeb28",
        ),
        (
            "macos-26-x86_64",
            "tinyvoice-module-0.1.16-macos-26-x86_64.tar.gz",
            "20142bb6bb285b863b626d2d49c4879331de1f34cfc2847755d038adf5664abe",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinyvoice-module-0.1.16-ubuntu-22.04-arm64.tar.gz",
            "be42ffdf401d10bdaac083b05c42ebbeeca4f25649286cd0ada608fa02045c23",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinyvoice-module-0.1.16-ubuntu-22.04-x86_64.tar.gz",
            "9a43aedb59617d6322537c411f91710ffe31c9595066592e0abc6086aca13a1a",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinyvoice-module-0.1.16-ubuntu-24.04-arm64.tar.gz",
            "620167d61a331ad1260f3f432552cca57af4a0696b51b3e7d8d62ad1f0337a06",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinyvoice-module-0.1.16-ubuntu-24.04-x86_64.tar.gz",
            "90de2c9dce82e9d8e3864e9e0f827f1110599a3b5148d905a2841492145a1015",
        ),
        (
            "windows-11-arm64",
            "tinyvoice-module-0.1.16-windows-11-arm64.zip",
            "8272daea789109e6034ee3af5ad9a46e5559b511f9cf6c965c14225e1c59c69e",
        ),
        (
            "windows-2022-x86_64",
            "tinyvoice-module-0.1.16-windows-2022-x86_64.zip",
            "89fc25bdddbcb77393f8618aa4f7f00f9cd9abd27dd1e35ad184aafb4fe4fb9d",
        ),
        (
            "windows-2025-x86_64",
            "tinyvoice-module-0.1.16-windows-2025-x86_64.zip",
            "271ba8c8476c22535f9fba0086b0aa0a253f9d228cd6f6a73e91cbe24dfa8a2a",
        ),
    ];
    assert_eq!(record.assets.len(), expected.len());
    for (key, archive, digest) in expected {
        let asset = record.asset_for(key).unwrap();
        assert_eq!(asset.archive, archive);
        assert_eq!(asset.sha256, digest);
    }
}
