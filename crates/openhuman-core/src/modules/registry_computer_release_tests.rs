use super::*;

#[test]
fn tinycomputer_registry_matches_bus_contract_and_published_release() {
    let desktop = find("tinycomputer").expect("compiled computer module");
    assert_eq!(desktop.bus_name, tinycomputer_bus::names::INTERFACE);
    assert_eq!(desktop.object_path, tinycomputer_bus::names::OBJECT_PATH);
    assert_eq!(desktop.version, "0.16.0");
    assert_eq!(tinycomputer_bus::CONTRACT_VERSION, (2, 29));

    let mut actual = desktop
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let mut published = [
        (
            "archlinux-arm64",
            "tinycomputer-0.16.0-archlinux-arm64.tar.gz",
            "6dc3229b014f15560fd7dd793fc877b653b88571db9963dc919be8866e58a4f1",
        ),
        (
            "archlinux-x86_64",
            "tinycomputer-0.16.0-archlinux-x86_64.tar.gz",
            "3f19184a00aefeb218aa1dac6ecb534be0ef9973becceb10506ac075e9911b9d",
        ),
        (
            "fedora-43-arm64",
            "tinycomputer-0.16.0-fedora-43-arm64.tar.gz",
            "7c0cc88be5169950dbf5f5677df8431c8c76c747cdc4a78d64c7b666e6ad121c",
        ),
        (
            "fedora-43-x86_64",
            "tinycomputer-0.16.0-fedora-43-x86_64.tar.gz",
            "a203aef368219cf4483cd023d92b712b7ad38e1517074639c7ea38d9ccccbac3",
        ),
        (
            "fedora-44-arm64",
            "tinycomputer-0.16.0-fedora-44-arm64.tar.gz",
            "05c4dc081306af96846f9196741cfadceaee51a9c221c210fc7486649f690bc3",
        ),
        (
            "fedora-44-x86_64",
            "tinycomputer-0.16.0-fedora-44-x86_64.tar.gz",
            "dd249cf9cfd0854e0493b4100a40b767e22e20b0cc316e4529625c5b3d500279",
        ),
        (
            "macos-15-arm64",
            "tinycomputer-0.16.0-macos-15-arm64.tar.gz",
            "97ef1ef6134d36bc25c770efcbedd239374d43e615ec57cf91f5eb44ac314265",
        ),
        (
            "macos-15-x86_64",
            "tinycomputer-0.16.0-macos-15-x86_64.tar.gz",
            "5bafc90beff673f66b1442cb0ffb41acb33512431ed167833d0f0de64f8b6f94",
        ),
        (
            "macos-26-arm64",
            "tinycomputer-0.16.0-macos-26-arm64.tar.gz",
            "a028876b857e62a8a55b850811a0d20f85dd0a2bb8a6533c2c4008a7062b4a35",
        ),
        (
            "macos-26-x86_64",
            "tinycomputer-0.16.0-macos-26-x86_64.tar.gz",
            "1604838f9793dd1e467ff299b2beda47aedeb94810fb8ac857629fdd061c0812",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinycomputer-0.16.0-ubuntu-22.04-arm64.tar.gz",
            "5cce23246a32c23cc6a6d0adf530126e65eaaeaaa864e5cc7b20b63c92ac29e0",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinycomputer-0.16.0-ubuntu-22.04-x86_64.tar.gz",
            "dbe66666a9ace5e4c1751ba191a12aa2f53e608afbd5f7383e6277d82238407b",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinycomputer-0.16.0-ubuntu-24.04-arm64.tar.gz",
            "4a5039effb89b49cc6650c66c5f32efa898c8cd0d18e2d50b3275d4c1cec372d",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinycomputer-0.16.0-ubuntu-24.04-x86_64.tar.gz",
            "3144d4dd90f7c4886e4eff899210f2fc8b6f8235e4d14c8c03454f356d00d062",
        ),
        (
            "windows-11-arm64",
            "tinycomputer-0.16.0-windows-11-arm64.zip",
            "857f8723b4624f5f29707797310de7e95439ac09ef0c7e46b7a3aab019805121",
        ),
        (
            "windows-2022-x86_64",
            "tinycomputer-0.16.0-windows-2022-x86_64.zip",
            "ec8ce6ef45c8b399b5b3ff6eb93ff5d74f714ad93e7cfc228e52e59fcbf00f1b",
        ),
        (
            "windows-2025-x86_64",
            "tinycomputer-0.16.0-windows-2025-x86_64.zip",
            "68133c2d864e8f8cbfb5773bd7afd7663e3d2ea0fd731f82570bbb961b37e694",
        ),
    ];
    assert_eq!(desktop.assets.len(), published.len());
    actual.sort_unstable();
    published.sort_unstable();
    assert_eq!(actual, published);
}
