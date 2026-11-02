//! Digest gate for tests that load the published TinyFlows v0.9.4 module.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const V0_9_4_MODULE_HASHES: &[&str] = &[
    // tinyflows-module-0.9.4-macos-15-arm64.tar.gz
    "abdd0f29f1b510aceb0760595b613cf43bf9c14fc93fae925c693e0350acf614",
    // tinyflows-module-0.9.4-macos-15-x86_64.tar.gz
    "108192455fa955a68059f153f61547e142a9fd4a00867b4680390435f47c76fe",
    // tinyflows-module-0.9.4-macos-26-arm64.tar.gz
    "45b2c691b28bed3873cacc0b9b7af36e740de061ecaac9b66ac3430e31687f54",
    // tinyflows-module-0.9.4-macos-26-x86_64.tar.gz
    "a9c7caf7fb70e785a2ba0c5de56a4f27dce2517c87a28d9c5c754434111a78a6",
    // tinyflows-module-0.9.4-ubuntu-22.04-arm64.tar.gz
    "08136db614c2f5da2d7823d96307d07fde99b4a7c571c307f25d1ec05fb900d2",
    // tinyflows-module-0.9.4-ubuntu-22.04-x86_64.tar.gz
    "1b4cfb6003bdf47656d75ba8247a05ad33c124d6f475143da5a8553f777b9c46",
    // tinyflows-module-0.9.4-ubuntu-24.04-arm64.tar.gz
    "d599df4f8720f11e64c29390d0bac8ed7b5a322e4c9abb213b78a76898cfaee2",
    // tinyflows-module-0.9.4-ubuntu-24.04-x86_64.tar.gz
    "bc770310b6ae7d0fe7015b3e0971edf0f3056b54362f715c76b2e40b25ae1696",
    // tinyflows-module-0.9.4-windows-11-arm64.zip
    "af190bbbe0c1756080410b9d4322e35eeceaa6e9a13a5c77d878450bd8a2a085",
    // tinyflows-module-0.9.4-windows-2022-x86_64.zip
    "8f283555f36e3169035776c1f977473e27216ee755429f363eca3bcacb8285ce",
    // tinyflows-module-0.9.4-windows-2025-x86_64.zip
    "c414c94a761b85f1c4739a7ab7b48eda29961b0a9715836ed248e490e737468f",
];

/// Finds the staged platform artifact and confirms it matches a published v0.9.4 bundle.
pub fn staged_v0_9_4_module() -> (PathBuf, String) {
    let artifact = openhuman_core::modules::local_artifact("tinyflows")
        .expect("stage the published TinyFlows v0.9.4 module on OPENHUMAN_MODULE_PATH");
    let digest = verify_v0_9_4_module(&artifact);
    (artifact, digest)
}

/// Confirms `artifact` is one of the public platform bundles for v0.9.4.
pub fn verify_v0_9_4_module(artifact: &Path) -> String {
    let digest = hex::encode(Sha256::digest(
        std::fs::read(artifact).expect("read staged TinyFlows native artifact"),
    ));
    assert!(
        V0_9_4_MODULE_HASHES.contains(&digest.as_str()),
        "staged TinyFlows library is not a published v0.9.4 module: {} sha256={digest}",
        artifact.display()
    );
    digest
}
