//! Digest gate for tests that load the published TinyFlows v0.9.5 module.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const V0_9_5_MODULE_HASHES: &[&str] = &[
    // tinyflows-module-0.9.5-macos-15-arm64.tar.gz
    "11a25c86bc23cae06766f4fcdfa206886735a407c29d115ec37127752bd76d45",
    // tinyflows-module-0.9.5-macos-15-x86_64.tar.gz
    "e519baae32a05f66e4d394cc97a5d77dbd6a205d68ddc3a8bf87af0592bdd6f0",
    // tinyflows-module-0.9.5-macos-26-arm64.tar.gz
    "90fcee41304eb7390024df346e0a223b607dda6e61830f50cef3cbb87f67ac84",
    // tinyflows-module-0.9.5-macos-26-x86_64.tar.gz
    "a7e4604bec9436a189f83c94787475c5df91bf314721ab239927b695a01d8c05",
    // tinyflows-module-0.9.5-ubuntu-22.04-arm64.tar.gz
    "c5bfc95cdbc4087efa0c451c3c75237f39e6eb087f3e68f020a6c593912fba5e",
    // tinyflows-module-0.9.5-ubuntu-22.04-x86_64.tar.gz
    "38733f7c6618567851fe327184c1f2ed2443ed4458896973f8950c91bf318824",
    // tinyflows-module-0.9.5-ubuntu-24.04-arm64.tar.gz
    "fcb38ed6aa6f1317363b8d2b965cf21bb1047eeb09970b631bf7807b0ef4c4b4",
    // tinyflows-module-0.9.5-ubuntu-24.04-x86_64.tar.gz
    "9cd8a93c94595e92afafd9c9c4820f76afa8347da4c3dfb97c27b7f1ee1691f9",
    // tinyflows-module-0.9.5-windows-11-arm64.zip
    "9d7eff99566ff1e54cf66e188e0948335ca4a0ceb19c6697f389b7dd3a52a2be",
    // tinyflows-module-0.9.5-windows-2022-x86_64.zip
    "cdbbd6fb591cf52e995936a9fc6559c8d1de06b2342635993bf315973f7dfd8c",
    // tinyflows-module-0.9.5-windows-2025-x86_64.zip
    "89b981fd1b7b2441e6670438d5c75ebc3a20456a33211f5c5b57602930e79e67",
];

/// Finds the staged platform artifact and confirms it matches a published v0.9.5 bundle.
pub fn staged_v0_9_5_module() -> (PathBuf, String) {
    let artifact = openhuman_core::modules::local_artifact("tinyflows")
        .expect("stage the published TinyFlows v0.9.5 module on OPENHUMAN_MODULE_PATH");
    let digest = verify_v0_9_5_module(&artifact);
    (artifact, digest)
}

/// Confirms `artifact` is one of the public platform bundles for v0.9.5.
pub fn verify_v0_9_5_module(artifact: &Path) -> String {
    let digest = hex::encode(Sha256::digest(
        std::fs::read(artifact).expect("read staged TinyFlows native artifact"),
    ));
    assert!(
        V0_9_5_MODULE_HASHES.contains(&digest.as_str()),
        "staged TinyFlows library is not a published v0.9.5 module: {} sha256={digest}",
        artifact.display()
    );
    digest
}
