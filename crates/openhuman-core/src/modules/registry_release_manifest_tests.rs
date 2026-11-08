use super::*;

#[cfg(feature = "security-module")]
#[test]
fn tinysecurity_registry_matches_the_verified_v0190_release_manifest() {
    let security = find("tinysecurity").expect("compiled TinySecurity module");
    assert_eq!(security.version, "0.20.0");
    assert_eq!(tinysecurity_bus::CONTRACT_VERSION, (2, 1));
    assert_eq!(
        security.release_url,
        "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.20.0"
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
            "tinysecurity-module-0.20.0-macos-15-arm64.tar.gz",
            "d0c80a5fc21527d812504d64dc25aeadc55327f17ff938ed18d9b5da36800239",
        ),
        (
            "macos-15-x86_64",
            "tinysecurity-module-0.20.0-macos-15-x86_64.tar.gz",
            "6d351306f7d970715199c6e9b9e4b566e4ff29dfd94701d6ce19716bf03114c1",
        ),
        (
            "macos-26-arm64",
            "tinysecurity-module-0.20.0-macos-26-arm64.tar.gz",
            "9f51e6bfda60aa7d44520130cf4a3e1e88e5fee7b423ca663a979926e79e3d30",
        ),
        (
            "macos-26-x86_64",
            "tinysecurity-module-0.20.0-macos-26-x86_64.tar.gz",
            "4668ee65802f5aa55b8c9310f8dd11a3a290ff4e121895ba73f1e7951ad9fb21",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinysecurity-module-0.20.0-ubuntu-22.04-arm64.tar.gz",
            "3b89af12be97ccac2a2593ff9886cd8f9cb7fcba5405fe04c9d0986e4d6c110c",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinysecurity-module-0.20.0-ubuntu-22.04-x86_64.tar.gz",
            "d531a36c266cb467c1fc258543ed4713ddd2234930fc3beeba386b48e4bbe0ac",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinysecurity-module-0.20.0-ubuntu-24.04-arm64.tar.gz",
            "745b0583c73703644da49b9c741c8c8a487ea24116d51b797133f3cea0a5d680",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinysecurity-module-0.20.0-ubuntu-24.04-x86_64.tar.gz",
            "e694059c8f2e9f4da4265ae8523fa8d6643236a4eff745e5e4290ee6ffe06fa9",
        ),
        (
            "windows-11-arm64",
            "tinysecurity-module-0.20.0-windows-11-arm64.zip",
            "5c45a750b61315609a88fb487bab0a2ee87292c5f2eff6d4a19d46c93a53ca15",
        ),
        (
            "windows-2022-x86_64",
            "tinysecurity-module-0.20.0-windows-2022-x86_64.zip",
            "282f9e8cfff87da92b0f0c0aba2650edaf10def411a3a7423d46d890adfaae26",
        ),
        (
            "windows-2025-x86_64",
            "tinysecurity-module-0.20.0-windows-2025-x86_64.zip",
            "77359a4c2b3c1207e98e8c36e86bd014d7933cffbd4889bc4e7fc36c53adf3e7",
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
    assert_eq!(record.version, "0.4.3");
    assert_eq!(tinyvoice_bus::CONTRACT_VERSION, (1, 12));
    let expected = [
        (
            "archlinux-arm64",
            "tinyvoice-module-0.4.3-archlinux-arm64.tar.gz",
            "d17a2610a57e023f1492301fd49bbf9ccfe14382add281f4cc0f0ea11b82bcb3",
        ),
        (
            "archlinux-x86_64",
            "tinyvoice-module-0.4.3-archlinux-x86_64.tar.gz",
            "3cef1a9d35c9637a1a31513f1d172717d1293eb7958ee380f773475f33d8979d",
        ),
        (
            "fedora-43-arm64",
            "tinyvoice-module-0.4.3-fedora-43-arm64.tar.gz",
            "565c95f7a677f8d7c619c16ec0bc31c3abbc1b5863fa1f1c8616d25607c71788",
        ),
        (
            "fedora-43-x86_64",
            "tinyvoice-module-0.4.3-fedora-43-x86_64.tar.gz",
            "3d8ef34b8120edd13496a3b51178048ab489f3281fa9bb36890999ea10c01228",
        ),
        (
            "fedora-44-arm64",
            "tinyvoice-module-0.4.3-fedora-44-arm64.tar.gz",
            "21bdaf1e19c1ca858124a38eff712bc7d2b49c6a2806902e93525029de5fae2a",
        ),
        (
            "fedora-44-x86_64",
            "tinyvoice-module-0.4.3-fedora-44-x86_64.tar.gz",
            "ec84a82551cc61bd03a305d7004b763e03dda56b9cfe70d2ab9dc57395f3f1af",
        ),
        (
            "macos-15-arm64",
            "tinyvoice-module-0.4.3-macos-15-arm64.tar.gz",
            "181b588bef33710e2ae3e35bd17da3d3f1390ca6278548cdfbc9321e3a0f2470",
        ),
        (
            "macos-15-x86_64",
            "tinyvoice-module-0.4.3-macos-15-x86_64.tar.gz",
            "ee08802d3b39d2afcbc8cc445ae5c61584d3099df0d1efa2458cfa4648f0ad28",
        ),
        (
            "macos-26-arm64",
            "tinyvoice-module-0.4.3-macos-26-arm64.tar.gz",
            "af827579dded0d0c6b307f332d6907be258831b2b21dd8595a807052d5879acf",
        ),
        (
            "macos-26-x86_64",
            "tinyvoice-module-0.4.3-macos-26-x86_64.tar.gz",
            "b253819adf530d0a97c0299850f4aa72574a4dc4147afce8677240b3abf06cbc",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinyvoice-module-0.4.3-ubuntu-22.04-arm64.tar.gz",
            "dc3a472a525cfe91940bcb30696af3d5c5a2f1a2378d40d282b4834c28187750",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinyvoice-module-0.4.3-ubuntu-22.04-x86_64.tar.gz",
            "f69566e150bc27b6bbf93eaf5fbf49221e2264bc983ea9f67c5fa96f039b2a3b",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinyvoice-module-0.4.3-ubuntu-24.04-arm64.tar.gz",
            "9cfe3429144f54d12cc3150b3e88ee77748015099b746cabd659bc78dcae8e30",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinyvoice-module-0.4.3-ubuntu-24.04-x86_64.tar.gz",
            "06d1756f6b470034e83f40d61bdda7d353e9be9df9cdfbd5e6160bfcc300ef1f",
        ),
        (
            "windows-11-arm64",
            "tinyvoice-module-0.4.3-windows-11-arm64.zip",
            "789ba0cbe7f46888493786d76e31b4c5e8c1c20375563c9632ff332f6718f972",
        ),
        (
            "windows-2022-x86_64",
            "tinyvoice-module-0.4.3-windows-2022-x86_64.zip",
            "1c6259d330e5b134a829d4dbdfb8daa2d29f3754f51ab3507a3b28e51f63e9d2",
        ),
        (
            "windows-2025-x86_64",
            "tinyvoice-module-0.4.3-windows-2025-x86_64.zip",
            "b2311f4895ba74b5ec359202d06dc951c0430311f1e38d2dd659e2b030748059",
        ),
    ];
    assert_eq!(record.assets.len(), expected.len());
    for (key, archive, digest) in expected {
        let asset = record.asset_for(key).unwrap();
        assert_eq!(asset.archive, archive);
        assert_eq!(asset.sha256, digest);
    }
}
