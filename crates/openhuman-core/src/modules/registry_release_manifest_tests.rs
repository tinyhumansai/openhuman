use super::*;

#[cfg(feature = "security-module")]
#[test]
fn tinysecurity_registry_matches_the_verified_v0182_release_manifest() {
    let security = find("tinysecurity").expect("compiled TinySecurity module");
    assert_eq!(security.version, "0.18.2");
    assert_eq!(tinysecurity_bus::CONTRACT_VERSION, (1, 33));
    assert_eq!(
        security.release_url,
        "https://github.com/senamakel/tinysecurity/releases/tag/v0.18.2"
    );

    let mut actual = security
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    actual.sort_unstable();
    let mut published = [
        (
            "macos-15-arm64",
            "tinysecurity-module-0.18.2-macos-15-arm64.tar.gz",
            "7cd14bf7bbb11bceb4e09560a8f66c941e28541e94ac42320156ba9a7473952b",
        ),
        (
            "macos-15-x86_64",
            "tinysecurity-module-0.18.2-macos-15-x86_64.tar.gz",
            "7829b6e6b41a34d789973e8f94f828235c26ae201050de3645dfd6b53568941c",
        ),
        (
            "macos-26-arm64",
            "tinysecurity-module-0.18.2-macos-26-arm64.tar.gz",
            "bc765db3337e98c4f0c96ba6fd47576181c9783e833f9bdeccc745dcab276dc5",
        ),
        (
            "macos-26-x86_64",
            "tinysecurity-module-0.18.2-macos-26-x86_64.tar.gz",
            "10575824c14ea3503170420523ecdb6b3fa33cecb85f9ff3d36817666bb7833e",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinysecurity-module-0.18.2-ubuntu-22.04-arm64.tar.gz",
            "34c3ef7b6fde18d525ae27fe09716ad2775a91b3152663a1f72b88a0da15e325",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinysecurity-module-0.18.2-ubuntu-22.04-x86_64.tar.gz",
            "000e9a3a07a8388fee3a6b823615998959d5255e584526b978b653959fad8a30",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinysecurity-module-0.18.2-ubuntu-24.04-arm64.tar.gz",
            "a91db19a360840fd5f8a7c341ef4856fcdc25db98cc597b7acdd80ac075ab046",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinysecurity-module-0.18.2-ubuntu-24.04-x86_64.tar.gz",
            "4c8d90347c37293d5ea773245e0ce9dda4a18234117736757564beeffea5f5fa",
        ),
        (
            "windows-11-arm64",
            "tinysecurity-module-0.18.2-windows-11-arm64.zip",
            "dc6e7b53e7bf65c9adea251d0f1f5852a89f0588c782dbdb6ac0bab08a33c8ef",
        ),
        (
            "windows-2022-x86_64",
            "tinysecurity-module-0.18.2-windows-2022-x86_64.zip",
            "105ccd4d6fc2787cefaedba2dcc2f5e40129c0b4759e41095099322da5d6058b",
        ),
        (
            "windows-2025-x86_64",
            "tinysecurity-module-0.18.2-windows-2025-x86_64.zip",
            "2b41ef005c4d0756233451ca8667fdce492b73a85234bd7f4e2af06949130b70",
        ),
    ];
    published.sort_unstable();
    assert_eq!(actual, published);
}

#[test]
fn tinychannels_registry_matches_scoped_runtime_release() {
    let record = find("tinychannels").expect("compiled TinyChannels module");
    assert_eq!(record.version, "0.1.45");
    assert_eq!(tinychannels_bus::CONTRACT_VERSION, 33);
    assert_eq!(
        tinychannels_bus::module_relay::MODULE_OPERATION_VERSION,
        (1, 25)
    );
    let expected = [
        (
            "archlinux-arm64",
            "tinychannels-module-0.1.45-archlinux-arm64.tar.gz",
            "2adc56f7ea983f208a46d77d4bfc9bf05b5f414e3be7c1a578c2079c57e2a983",
        ),
        (
            "archlinux-x86_64",
            "tinychannels-module-0.1.45-archlinux-x86_64.tar.gz",
            "c264de7479ea0f67f3c8fb3ed51480d0fc91e4c3ac8e96d27a9f2d65fc5c6c7a",
        ),
        (
            "fedora-43-arm64",
            "tinychannels-module-0.1.45-fedora-43-arm64.tar.gz",
            "02795d3c0643dec350d961c08c9ba3ed463463c70f9025c0faa71fb9fcfb9c8b",
        ),
        (
            "fedora-43-x86_64",
            "tinychannels-module-0.1.45-fedora-43-x86_64.tar.gz",
            "e6897ee47572d049cd2d8956f5a9dba226f013af2510c11290b02be1cd3518e4",
        ),
        (
            "fedora-44-arm64",
            "tinychannels-module-0.1.45-fedora-44-arm64.tar.gz",
            "cd9d6328464fa0ecb46ad51fe794ca0920101e1ac2fbce4acf19d5bb4bcacf14",
        ),
        (
            "fedora-44-x86_64",
            "tinychannels-module-0.1.45-fedora-44-x86_64.tar.gz",
            "f845f4b838c7b95ce0f4e7c6d8cd07e920d8e9874e5186cac6a607c52c952638",
        ),
        (
            "macos-15-arm64",
            "tinychannels-module-0.1.45-macos-15-arm64.tar.gz",
            "4b6a4cdf406c5ce95c6c8afe8ec1c3592e5aa3c7a3aee6f18359cd8ea994a55e",
        ),
        (
            "macos-15-x86_64",
            "tinychannels-module-0.1.45-macos-15-x86_64.tar.gz",
            "c6692f4517fbc0d0630df24d68507aec61546bb97a1399732fcfba7c66bdd603",
        ),
        (
            "macos-26-arm64",
            "tinychannels-module-0.1.45-macos-26-arm64.tar.gz",
            "07de20fa41ba08e4cd3fd727b9c305abca197bce0594d367de50e6c5021345d9",
        ),
        (
            "macos-26-x86_64",
            "tinychannels-module-0.1.45-macos-26-x86_64.tar.gz",
            "4e65f802b3aeb3aadc73af0a696e00de324ec4eef363e07a3d50f0cde2b1a84b",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinychannels-module-0.1.45-ubuntu-22.04-arm64.tar.gz",
            "d521277a7d963332c61233e6a083d920cabbaf38befb346def1fa864bf74adb4",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinychannels-module-0.1.45-ubuntu-22.04-x86_64.tar.gz",
            "add0a97cf1ab4b85cc5c651e4d4204ff700595c1b94d6aede4a8e2220fa73b3d",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinychannels-module-0.1.45-ubuntu-24.04-arm64.tar.gz",
            "a51a42c8ee18b26322ce8981a01f6c83e5a6d6a633c002dd3040fa12cb60e513",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinychannels-module-0.1.45-ubuntu-24.04-x86_64.tar.gz",
            "9cf0d1be60f55601c263fc29c1c91172db2bfcb30d4ea19a11bcb0e032ed7c19",
        ),
        (
            "windows-11-arm64",
            "tinychannels-module-0.1.45-windows-11-arm64.zip",
            "f5c36f904623a3c6e6b523df62618a6576cc60157f1457de11a7f2b11dea3cfb",
        ),
        (
            "windows-2022-x86_64",
            "tinychannels-module-0.1.45-windows-2022-x86_64.zip",
            "07545499e434b3e25d58a48f59874c49b7a9a5e609d904ce52011932e7640bc2",
        ),
        (
            "windows-2025-x86_64",
            "tinychannels-module-0.1.45-windows-2025-x86_64.zip",
            "e732561700bde58c35b896f02c395540a5bb45824e59e3c3fea0231786c217fe",
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
    assert_eq!(record.version, "0.4.2");
    assert_eq!(tinyvoice_bus::CONTRACT_VERSION, (1, 12));
    let expected = [
        (
            "archlinux-arm64",
            "tinyvoice-module-0.4.2-archlinux-arm64.tar.gz",
            "0cef54f6248019db21282fa76db4f9663fbe830c3b18c2804ab28833d60329e1",
        ),
        (
            "archlinux-x86_64",
            "tinyvoice-module-0.4.2-archlinux-x86_64.tar.gz",
            "707838c2fb8c3e4ad861d56e37418fb71c1b21bc56826fb159fd46057f1af9f0",
        ),
        (
            "fedora-43-arm64",
            "tinyvoice-module-0.4.2-fedora-43-arm64.tar.gz",
            "51c1d67747872016127c3633fda9ea7111eaa8142782e6b6da4ef15ed9c4a171",
        ),
        (
            "fedora-43-x86_64",
            "tinyvoice-module-0.4.2-fedora-43-x86_64.tar.gz",
            "01aac694078952c8dea1090bc99b584930c4a8a043ff8defb0512110f4aa9433",
        ),
        (
            "fedora-44-arm64",
            "tinyvoice-module-0.4.2-fedora-44-arm64.tar.gz",
            "21bb4a50fe65aaa6a5d09f94d15933c3dd7878b7cf919daa06f2f0939ea421be",
        ),
        (
            "fedora-44-x86_64",
            "tinyvoice-module-0.4.2-fedora-44-x86_64.tar.gz",
            "7113c298e9abbbfacc55a45e237b4962ce9b0256f9ffe12e901ee45cddb226d0",
        ),
        (
            "macos-15-arm64",
            "tinyvoice-module-0.4.2-macos-15-arm64.tar.gz",
            "1c0cf8391e8afb35a13763b6c05fe811c14a640fdf9a79e17bae7f32952009d0",
        ),
        (
            "macos-15-x86_64",
            "tinyvoice-module-0.4.2-macos-15-x86_64.tar.gz",
            "72c6d9fb1aad0b4ce3f7cceaad45111d4450e9a1285140945dcc585a69e16bde",
        ),
        (
            "macos-26-arm64",
            "tinyvoice-module-0.4.2-macos-26-arm64.tar.gz",
            "2b37df831a1196f5d379994a1b85e9f69015d7cde47564470afbb2ff31a85b77",
        ),
        (
            "macos-26-x86_64",
            "tinyvoice-module-0.4.2-macos-26-x86_64.tar.gz",
            "1f9ffd9307505693be31df2f6eaa0e6de32f104bd46640c06dc82bffa3e03fa7",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinyvoice-module-0.4.2-ubuntu-22.04-arm64.tar.gz",
            "ad63d27d2c40f8388d93a9d3556bfc8539c4b5ffe8839efaa0c1b7a1b7a61d43",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinyvoice-module-0.4.2-ubuntu-22.04-x86_64.tar.gz",
            "c2523bf5066757ce0c81e7f5c97a4b902312007ce079606d005e008ca8be66a5",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinyvoice-module-0.4.2-ubuntu-24.04-arm64.tar.gz",
            "78da28da3a483731bec8d53bc561219cc6dec793d66f27eb908755d88118461e",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinyvoice-module-0.4.2-ubuntu-24.04-x86_64.tar.gz",
            "1b542b8b39223525f40f02245ef21b7d995a039eef9cb8a2586bb46cce9668f9",
        ),
        (
            "windows-11-arm64",
            "tinyvoice-module-0.4.2-windows-11-arm64.zip",
            "734db41e5c5cd5580c5aa4fd7a9c3e5281aee62ea17c31d8c7037c7356360e82",
        ),
        (
            "windows-2022-x86_64",
            "tinyvoice-module-0.4.2-windows-2022-x86_64.zip",
            "47701888511b5b8ac71a5f92affcb04334b43e479b43e2b5a0937f2134f4cc5e",
        ),
        (
            "windows-2025-x86_64",
            "tinyvoice-module-0.4.2-windows-2025-x86_64.zip",
            "46b29e886d3ec82b996ca7138a7a60577fe6236383df59cb34aedfd3c670e9b0",
        ),
    ];
    assert_eq!(record.assets.len(), expected.len());
    for (key, archive, digest) in expected {
        let asset = record.asset_for(key).unwrap();
        assert_eq!(asset.archive, archive);
        assert_eq!(asset.sha256, digest);
    }
}
