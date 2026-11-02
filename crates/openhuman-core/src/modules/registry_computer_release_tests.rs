use super::*;

#[test]
fn tinycomputer_registry_matches_bus_contract_and_published_release() {
    let desktop = find("tinycomputer").expect("compiled computer module");
    assert_eq!(desktop.bus_name, tinycomputer_bus::names::INTERFACE);
    assert_eq!(desktop.object_path, tinycomputer_bus::names::OBJECT_PATH);
    assert_eq!(desktop.version, "0.15.2");
    assert_eq!(tinycomputer_bus::CONTRACT_VERSION, (2, 28));

    let mut actual = desktop
        .assets
        .iter()
        .map(|asset| (asset.host_key, asset.archive, asset.sha256))
        .collect::<Vec<_>>();
    let mut published = [
        (
            "archlinux-arm64",
            "tinycomputer-0.15.2-archlinux-arm64.tar.gz",
            "c04621d08c6fdea41a933c0565af17284b00e2f59cfd6d4f98d0281570c7ad2b",
        ),
        (
            "archlinux-x86_64",
            "tinycomputer-0.15.2-archlinux-x86_64.tar.gz",
            "eb95063fe8e8694f121a008feb6f4db68eae5e50bce19ab2f85abe82bb1d0e6d",
        ),
        (
            "fedora-43-arm64",
            "tinycomputer-0.15.2-fedora-43-arm64.tar.gz",
            "227680999f0f970a87709e598df2bcd73b0330485f32b5422313cd72a56569d7",
        ),
        (
            "fedora-43-x86_64",
            "tinycomputer-0.15.2-fedora-43-x86_64.tar.gz",
            "aa729616e40aaeeb2246c2748e6e2b5429afa11690b15c571b11a3d17c21330d",
        ),
        (
            "fedora-44-arm64",
            "tinycomputer-0.15.2-fedora-44-arm64.tar.gz",
            "47b3ab7b479a5b9794940b2cb39b7b8c5fcf9dfcfa9797788af990e44bc8ebf3",
        ),
        (
            "fedora-44-x86_64",
            "tinycomputer-0.15.2-fedora-44-x86_64.tar.gz",
            "f8acff1ae49334b26649eb3be52ad2695d845fa480ef4c1fc2f663c07b16b0d5",
        ),
        (
            "macos-15-arm64",
            "tinycomputer-0.15.2-macos-15-arm64.tar.gz",
            "ef5c4158b93888bcea1ea0f9a6f88986d09867da0779875e0ecfef927f7e9e7c",
        ),
        (
            "macos-15-x86_64",
            "tinycomputer-0.15.2-macos-15-x86_64.tar.gz",
            "dc1329f968b885fc1e10919f5fe70d3d77a9601b5f9849c013eb6938a81ef39b",
        ),
        (
            "macos-26-arm64",
            "tinycomputer-0.15.2-macos-26-arm64.tar.gz",
            "d6ae6cfd96b63ed079aee6cb1b57d739a5a90be28f46b2bcbbfcff8e58c844f4",
        ),
        (
            "macos-26-x86_64",
            "tinycomputer-0.15.2-macos-26-x86_64.tar.gz",
            "f58e76987f70b4a1823763406a081d18cfd4b10b81ee54556c6ee242c68ac8e8",
        ),
        (
            "ubuntu-22.04-arm64",
            "tinycomputer-0.15.2-ubuntu-22.04-arm64.tar.gz",
            "fd5bab476db4b2f480bdd4200fd79cfffdba4d0151baf98eb1a9c7fe42345eba",
        ),
        (
            "ubuntu-22.04-x86_64",
            "tinycomputer-0.15.2-ubuntu-22.04-x86_64.tar.gz",
            "2ff37fc1b586648fb1e694f69634085a87527c66430bc15afaec24b34a7cf318",
        ),
        (
            "ubuntu-24.04-arm64",
            "tinycomputer-0.15.2-ubuntu-24.04-arm64.tar.gz",
            "b79143f18b74b293c6c4860d0a44dc06af3edf5b6ea2499f7d6e28135fd7ef35",
        ),
        (
            "ubuntu-24.04-x86_64",
            "tinycomputer-0.15.2-ubuntu-24.04-x86_64.tar.gz",
            "238b0f7c561b9f420b8672ab20294dc7953139c9f98f4d5624013a60260d19cf",
        ),
        (
            "windows-11-arm64",
            "tinycomputer-0.15.2-windows-11-arm64.zip",
            "f5cd876112e18670ebb076a2895c9e4a13b014d2966ae833cc7818b6e222d1c0",
        ),
        (
            "windows-2022-x86_64",
            "tinycomputer-0.15.2-windows-2022-x86_64.zip",
            "371b34e4a47afb9dbb1dd03e4975210c9931c5e5c2b2909e3098ba32f6ae7511",
        ),
        (
            "windows-2025-x86_64",
            "tinycomputer-0.15.2-windows-2025-x86_64.zip",
            "0bb7ca5f9b7501ee1662899e6a3b43557e9aadc2e136fd227fb4872b5f4b7e89",
        ),
    ];
    assert_eq!(desktop.assets.len(), published.len());
    actual.sort_unstable();
    published.sort_unstable();
    assert_eq!(actual, published);
}
