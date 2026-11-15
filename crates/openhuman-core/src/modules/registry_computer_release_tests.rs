use super::*;

#[test]
fn tinycomputer_registry_matches_bus_contract_and_published_release() {
    let desktop = find("tinycomputer").expect("compiled computer module");
    assert_eq!(desktop.bus_name, tinycomputer_bus::names::INTERFACE);
    assert_eq!(desktop.object_path, tinycomputer_bus::names::OBJECT_PATH);
    assert_eq!(desktop.version, "0.18.0");
    assert_eq!(tinycomputer_bus::CONTRACT_VERSION, (3, 3));

    let mut actual = desktop
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let mut published = [
        (
            "archlinux-arm64",
            "tinycomputer-0.18.0-archlinux-arm64.tar.gz",
            "32ed1e7d7eb3ed93496752d3342bb28b02d302e2425a3bfa3d3c65b8b306060f",
        ),
        (
            "archlinux-x86_64",
            "tinycomputer-0.18.0-archlinux-x86_64.tar.gz",
            "6ba9df62f52ab4f23552ed2e12ff0bbabc16d7d076a20589c6be646f175bf735",
        ),
        (
            "fedora-43-arm64",
            "tinycomputer-0.18.0-fedora-43-arm64.tar.gz",
            "9252d5f7dc27297f71a0187b57fe82fc17595b38937bda0a242700e54db52757",
        ),
        (
            "fedora-43-x86_64",
            "tinycomputer-0.18.0-fedora-43-x86_64.tar.gz",
            "c634e7380ae1dddd04cdd45b0ef8e84dea7e15c0be4dcddf5ad2157fcfb86252",
        ),
        (
            "fedora-44-arm64",
            "tinycomputer-0.18.0-fedora-44-arm64.tar.gz",
            "acf92a651670614ccefaadd104b7d0e1701eb82fb838abd4ab5a05fbc6684a51",
        ),
        (
            "fedora-44-x86_64",
            "tinycomputer-0.18.0-fedora-44-x86_64.tar.gz",
            "a1ce13fd3c6b9e99730c92e3bf2c4edf41bcc4dd7f4259ced9417f0f1107fc39",
        ),
        (
            "macos-15-arm64",
            "tinycomputer-0.18.0-macos-15-arm64.tar.gz",
            "99a078dd03d3d444e51fcb7c12d3554929b0f6a8c9acb6b3cfb9d7f49040cf35",
        ),
        (
            "macos-15-x86_64",
            "tinycomputer-0.18.0-macos-15-x86_64.tar.gz",
            "eda9edca053a42fb82e06ba5a2a8b3dd873bc3e8d3d1be86a032787350b8a6dd",
        ),
        (
            "macos-26-arm64",
            "tinycomputer-0.18.0-macos-26-arm64.tar.gz",
            "06b3fbdc70520ecf8bc131f67bc5c7104af06a54bcb989b379ce3cd39491073f",
        ),
        (
            "macos-26-x86_64",
            "tinycomputer-0.18.0-macos-26-x86_64.tar.gz",
            "69d20c98997cbed3a0198076b1a3ccea127959585b2245abcd130c9de19567b1",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinycomputer-0.18.0-ubuntu-22.04-arm64.tar.gz",
            "0e752ce55fe488e5372242a064857ed9e2733f6be11b223a179e40aa8003bc1e",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinycomputer-0.18.0-ubuntu-22.04-x86_64.tar.gz",
            "976fdcb8e3406886bad35a6743ab56783989db67d9ab22669b04033397cadeae",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinycomputer-0.18.0-ubuntu-24.04-arm64.tar.gz",
            "ec20f09440218efe9f29080f59fcf9dba94a25fe3532d53580204b57059937be",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinycomputer-0.18.0-ubuntu-24.04-x86_64.tar.gz",
            "6cb3141cb850f308fc63ecb441e18cd727b26591f59c379fe317c5a2d13e95ec",
        ),
        (
            "windows-11-arm64",
            "tinycomputer-0.18.0-windows-11-arm64.zip",
            "f3046dfea2ed3e048ec46acf2511c40262c210c564335e496aa7d418d27deb9b",
        ),
        (
            "windows-2022-x86_64",
            "tinycomputer-0.18.0-windows-2022-x86_64.zip",
            "5915fd327a657dfd47f1402bcb21006e3e3a72d41cab176aac7614d9cfb20d12",
        ),
        (
            "windows-2025-x86_64",
            "tinycomputer-0.18.0-windows-2025-x86_64.zip",
            "a170cd5f27eb0db06a0852b045a29f9e205c87e6c69df929174972f1f2e04d9c",
        ),
    ];
    assert_eq!(desktop.assets.len(), published.len());
    actual.sort_unstable();
    published.sort_unstable();
    assert_eq!(actual, published);
}
