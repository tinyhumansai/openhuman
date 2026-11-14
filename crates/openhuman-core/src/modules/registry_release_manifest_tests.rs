use super::*;

#[cfg(feature = "security-module")]
#[test]
fn tinysecurity_registry_matches_the_published_release_manifest() {
    let security = find("tinysecurity").expect("compiled TinySecurity module");
    assert_eq!(security.version, "0.21.1");
    assert_eq!(tinysecurity_bus::CONTRACT_VERSION, (2, 2));
    assert_eq!(
        security.release_url,
        "https://github.com/tinyhumansai/tinysecurity/releases/tag/v0.21.1"
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
            "tinysecurity-module-0.21.1-macos-15-arm64.tar.gz",
            "1de5e5e83a30a400fcbeff3cc5cd8090e9bc243e4bc35a2f61b0a6e3193728d8",
        ),
        (
            "macos-15-x86_64",
            "tinysecurity-module-0.21.1-macos-15-x86_64.tar.gz",
            "6f4329bff1efcdcf82ed3d590e4536c89e814b43f3eccea4d4a5c902157babb1",
        ),
        (
            "macos-26-arm64",
            "tinysecurity-module-0.21.1-macos-26-arm64.tar.gz",
            "d66db53772d3843c9d1790b1b218bc5388c2b9b5f01a7ff6f4d5391498010213",
        ),
        (
            "macos-26-x86_64",
            "tinysecurity-module-0.21.1-macos-26-x86_64.tar.gz",
            "a7e95e03289917a69253f6d28c8fdfcf076ffa48d748e522c4c82d75f81e607b",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinysecurity-module-0.21.1-ubuntu-22.04-arm64.tar.gz",
            "c1c2b68d8e23bc80056dacb9bb343f705d7ead11b9aa139d87a82069318f743a",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinysecurity-module-0.21.1-ubuntu-22.04-x86_64.tar.gz",
            "bcd8512c79e4b85d84b03fb791ac273ade830775ac86ff7ed16452fbf829d8e0",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinysecurity-module-0.21.1-ubuntu-24.04-arm64.tar.gz",
            "caad79e9772384a0879c85700566c1efd58075bf7f8ac57f9067ca510459d440",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinysecurity-module-0.21.1-ubuntu-24.04-x86_64.tar.gz",
            "9f94fa86fe883e6aa7d96b0835ec8feef0c589c9f029a22b1c1c859103a088f2",
        ),
        (
            "windows-11-arm64",
            "tinysecurity-module-0.21.1-windows-11-arm64.zip",
            "cd77275f4849678c71e8dcaa6bad8b2ca4c473b093a641660a7b8195265e00f5",
        ),
        (
            "windows-2022-x86_64",
            "tinysecurity-module-0.21.1-windows-2022-x86_64.zip",
            "504c9937191a1c84f8e03964c3536e0e83417072769b1dba2ee31c94a2870e91",
        ),
        (
            "windows-2025-x86_64",
            "tinysecurity-module-0.21.1-windows-2025-x86_64.zip",
            "612d04ef0616589f61c852789768f06e0c4aa5fe02449e266a2bbc3df242d590",
        ),
    ];
    published.sort_unstable();
    assert_eq!(actual, published);
}

#[test]
fn tinychannels_registry_matches_scoped_runtime_release() {
    let record = find("tinychannels").expect("compiled TinyChannels module");
    assert_eq!(record.version, "0.2.1");
    assert_eq!(tinychannels_bus::CONTRACT_VERSION, 33);
    assert_eq!(
        tinychannels_bus::module_relay::MODULE_OPERATION_VERSION,
        (1, 25)
    );
    let expected = [
        (
            "archlinux-arm64",
            "tinychannels-module-0.2.1-archlinux-arm64.tar.gz",
            "2680ef5f51775adcf4d92c4cb228ed7518f8d0ec47fc93830c239569926cf98b",
        ),
        (
            "archlinux-x86_64",
            "tinychannels-module-0.2.1-archlinux-x86_64.tar.gz",
            "e8c5d4b98f1177a589f52b88bb0087c77370db37eab8387a177a86a3dab7a577",
        ),
        (
            "fedora-43-arm64",
            "tinychannels-module-0.2.1-fedora-43-arm64.tar.gz",
            "31f1e49213391a5ef3c30d195dc18230305cd7d1096afc1a26d50b1cb1094655",
        ),
        (
            "fedora-43-x86_64",
            "tinychannels-module-0.2.1-fedora-43-x86_64.tar.gz",
            "d5c8093c19347f80f371dd192b634d6d1c381098e269a8469a1fb429ef8ed112",
        ),
        (
            "fedora-44-arm64",
            "tinychannels-module-0.2.1-fedora-44-arm64.tar.gz",
            "c09fc5658b3db0d61a41592a0af985e01256ad1e57f4698956cd1aafbc50dbb8",
        ),
        (
            "fedora-44-x86_64",
            "tinychannels-module-0.2.1-fedora-44-x86_64.tar.gz",
            "224047a34161dea3918918623a907214ba37243ecd86470a9edd89974f335481",
        ),
        (
            "macos-15-arm64",
            "tinychannels-module-0.2.1-macos-15-arm64.tar.gz",
            "a5978525dec30a0179bb25c03dbddb7b188bc383ec609879ef4be2327ce37bfa",
        ),
        (
            "macos-15-x86_64",
            "tinychannels-module-0.2.1-macos-15-x86_64.tar.gz",
            "76080dbcd6c0ff4a0c2eb591f23087bb238520658a19372ff1b5d328e15138e4",
        ),
        (
            "macos-26-arm64",
            "tinychannels-module-0.2.1-macos-26-arm64.tar.gz",
            "06722cfb0686534cbdbcc7a07f5613d8e4f692b7df6a4087fffd34669f8735c8",
        ),
        (
            "macos-26-x86_64",
            "tinychannels-module-0.2.1-macos-26-x86_64.tar.gz",
            "450e65b3c2db1ad7a95bd2e9a97a90412d35808a55ca882ce2015e8683269704",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinychannels-module-0.2.1-ubuntu-22.04-arm64.tar.gz",
            "aa6fccae1e6762363c205c66be620fee096afabef0adb8652e0efe50b07d40ce",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinychannels-module-0.2.1-ubuntu-22.04-x86_64.tar.gz",
            "30ef63ef849ee8c7f959d1711a8f875629feb1643169ba4a1c5a6e75279ceebe",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinychannels-module-0.2.1-ubuntu-24.04-arm64.tar.gz",
            "d12dca9a9eee6bd487d45f845e9ee449f035049e8d47da1be26a2b2d5d282ad8",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinychannels-module-0.2.1-ubuntu-24.04-x86_64.tar.gz",
            "c22ab9d0342a7f24e37952665b8fd9ba7e51dbb11397c75f4e242bc228135a29",
        ),
        (
            "windows-11-arm64",
            "tinychannels-module-0.2.1-windows-11-arm64.zip",
            "ce07269fcda05f1ea4798fbf567cd26081d229c503a940981c48fbcd1d77771f",
        ),
        (
            "windows-2022-x86_64",
            "tinychannels-module-0.2.1-windows-2022-x86_64.zip",
            "03ce46d2e8e6728a531244d5e771ad26d082f490eab571079b051dc0aa323205",
        ),
        (
            "windows-2025-x86_64",
            "tinychannels-module-0.2.1-windows-2025-x86_64.zip",
            "fe8f9aaa89a3993df7c819a7fcdda4298c7f653116990a5355f0f726ba868d62",
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
