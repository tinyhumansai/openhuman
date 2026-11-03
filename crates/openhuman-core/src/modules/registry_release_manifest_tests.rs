use super::*;

#[cfg(feature = "security-module")]
#[test]
fn tinysecurity_registry_matches_the_verified_v0184_release_manifest() {
    let security = find("tinysecurity").expect("compiled TinySecurity module");
    assert_eq!(security.version, "0.18.4");
    assert_eq!(tinysecurity_bus::CONTRACT_VERSION, (1, 35));
    assert_eq!(
        security.release_url,
        "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.18.4"
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
            "tinysecurity-module-0.18.4-macos-15-arm64.tar.gz",
            "17d143697610cb7ab391d50d7d08d55165c1074ea1b454de08c9486cda19cc28",
        ),
        (
            "macos-15-x86_64",
            "tinysecurity-module-0.18.4-macos-15-x86_64.tar.gz",
            "b11d3ffe0a7a99e0a0fd71fd6fdb87d209e9b8e09c1cb4d3af9656376f368e8b",
        ),
        (
            "macos-26-arm64",
            "tinysecurity-module-0.18.4-macos-26-arm64.tar.gz",
            "107f42a9cedaae3a95edf09c82818ad85195e7f0668cf1255b51ee4fbc8ac26e",
        ),
        (
            "macos-26-x86_64",
            "tinysecurity-module-0.18.4-macos-26-x86_64.tar.gz",
            "b408ab2832246c1c66630c21ad2a274d6d6592a0f3eaf59039aa517e9955d8a4",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinysecurity-module-0.18.4-ubuntu-22.04-arm64.tar.gz",
            "f406c66e69817160527b55cd037e5421ddaadbb586702014e9c78d04622de991",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinysecurity-module-0.18.4-ubuntu-22.04-x86_64.tar.gz",
            "d56513cfc8d3f0ad2552abf31918365e3bbbdb700010a03e3273c79d6ac53b06",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinysecurity-module-0.18.4-ubuntu-24.04-arm64.tar.gz",
            "61cb28f3ebc5bfb77503b20abf128a4dbf55f2c5ba9f826ba240edf8bd51ff2c",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinysecurity-module-0.18.4-ubuntu-24.04-x86_64.tar.gz",
            "2e51299127281ae7c524c606590bb41ae2f635536c63791abe327d7f6abed0ba",
        ),
        (
            "windows-11-arm64",
            "tinysecurity-module-0.18.4-windows-11-arm64.zip",
            "825d2fde75cea1ffb11ff88cc037d1ec9428d784b2d5041f5eec828179f92020",
        ),
        (
            "windows-2022-x86_64",
            "tinysecurity-module-0.18.4-windows-2022-x86_64.zip",
            "8aa5719147a32209f14f9ad9fd4300e9c114a56dfe31d8f59d019b7540b0f111",
        ),
        (
            "windows-2025-x86_64",
            "tinysecurity-module-0.18.4-windows-2025-x86_64.zip",
            "fe1f64a199a820d5b58347eea41e29109096b62d81d00e0ed5df083a6fd5005a",
        ),
    ];
    published.sort_unstable();
    assert_eq!(actual, published);
}

#[test]
fn tinychannels_registry_matches_scoped_runtime_release() {
    let record = find("tinychannels").expect("compiled TinyChannels module");
    assert_eq!(record.version, "0.2.0");
    assert_eq!(tinychannels_bus::CONTRACT_VERSION, 33);
    assert_eq!(
        tinychannels_bus::module_relay::MODULE_OPERATION_VERSION,
        (1, 25)
    );
    let expected = [
        (
            "archlinux-arm64",
            "tinychannels-module-0.2.0-archlinux-arm64.tar.gz",
            "07d2285c42dbeca2d70e0eb26890df533b7daab2139a8e71c71d9ec88ce819cf",
        ),
        (
            "archlinux-x86_64",
            "tinychannels-module-0.2.0-archlinux-x86_64.tar.gz",
            "8d67e15a9c11f75cc53fab27c838308f5d622895df403f4d8ff5a37b4000411d",
        ),
        (
            "fedora-43-arm64",
            "tinychannels-module-0.2.0-fedora-43-arm64.tar.gz",
            "82ea9f45978fa82a1ab10846b922f7a31dfd1e0d6d7e13436aec3624dce707d6",
        ),
        (
            "fedora-43-x86_64",
            "tinychannels-module-0.2.0-fedora-43-x86_64.tar.gz",
            "a0ab5045055328e55e39568b77c1fd5dc63ea63a015fb86d0e0fbe905375b855",
        ),
        (
            "fedora-44-arm64",
            "tinychannels-module-0.2.0-fedora-44-arm64.tar.gz",
            "0eb80bc0ba0c36694e92a2ea628f73c010b9409952d232eca4261fc9a20cd837",
        ),
        (
            "fedora-44-x86_64",
            "tinychannels-module-0.2.0-fedora-44-x86_64.tar.gz",
            "a0ee9aa9d8f462ccbe60e6122fb32beb519f6e080d7ca7c19c150bff1a0e83a6",
        ),
        (
            "macos-15-arm64",
            "tinychannels-module-0.2.0-macos-15-arm64.tar.gz",
            "846b9391cdb7e5488c6b62c5c25fbd495e1899e3db92517cb0b0be0e32de8567",
        ),
        (
            "macos-15-x86_64",
            "tinychannels-module-0.2.0-macos-15-x86_64.tar.gz",
            "88a249b5ea3d5e80eb5bc753ee8d503bf8a2b2293d2c1127ca6a5fd3015dd421",
        ),
        (
            "macos-26-arm64",
            "tinychannels-module-0.2.0-macos-26-arm64.tar.gz",
            "3a666957b9d4cce0ffc23e1ea2a21b6fce90d823276cca018cba04ce957bfd54",
        ),
        (
            "macos-26-x86_64",
            "tinychannels-module-0.2.0-macos-26-x86_64.tar.gz",
            "ef0edd5d7990398a4b97ef114e8858d53f7418dbc818c1fad949472a712a2b61",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinychannels-module-0.2.0-ubuntu-22.04-arm64.tar.gz",
            "8d947a1b283bcb538ac59f68e1fdcce07fe47d9bb59d8ea0406316c213c04b13",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinychannels-module-0.2.0-ubuntu-22.04-x86_64.tar.gz",
            "92c90089eef81d7e90aeb8f2992dea706bcd91fd2cd9485004edea24eac18c0c",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinychannels-module-0.2.0-ubuntu-24.04-arm64.tar.gz",
            "cd7c1522e32d0cb125fe643feea5f3602663c99a8ae96f201fc5077a2496d588",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinychannels-module-0.2.0-ubuntu-24.04-x86_64.tar.gz",
            "b4556e866c309b1792a73af48bf7297c4a2714bae25c9ef1adc22c403f25dc04",
        ),
        (
            "windows-11-arm64",
            "tinychannels-module-0.2.0-windows-11-arm64.zip",
            "3e29456cd6b161a605c9224ebaa51dbdaa3f0a4b944529cfc3a8e471bc0d5725",
        ),
        (
            "windows-2022-x86_64",
            "tinychannels-module-0.2.0-windows-2022-x86_64.zip",
            "456d7c6cd16494aff56adfa2b4b96db47abd284203e11de10fc0e9efb6f76eda",
        ),
        (
            "windows-2025-x86_64",
            "tinychannels-module-0.2.0-windows-2025-x86_64.zip",
            "427bf8719b9cd2ac874f6cc9f74a5dc49fb08c8758b62cf8a09616448cb25134",
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
