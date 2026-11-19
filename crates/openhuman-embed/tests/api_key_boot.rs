//! A library host that supplies its own API key boots with nobody to answer an
//! approval: storing the key needs TinySecurity's secret master grant before the
//! core (and its approval gate) exists, and the host's own key is what
//! authorises that grant. One test: the consent environment is process-wide.

mod common;

use common::{offline_config, runtime_without_master_key, stub_backend};
use openhuman_embed::{Runtime, Workspace};

#[test]
fn a_runtime_built_with_its_own_api_key_boots_without_a_consent_answer() {
    // Nothing may answer the grant on the host's behalf: no test responder.
    std::env::remove_var("OPENHUMAN_TEST_KEYRING_CONSENT");
    std::thread::Builder::new()
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(|| {
            runtime_without_master_key().block_on(async {
                let backend = stub_backend().await;
                let runtime = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    Runtime::builder()
                        .config(offline_config())
                        .workspace(Workspace::Ephemeral)
                        .backend_url(backend.uri())
                        .api_key("sk-fixture")
                        .build(),
                )
                .await
                .expect("the build is not left waiting on an approval")
                .expect("a runtime built with its own API key boots");
                drop(runtime);
            });
        })
        .unwrap()
        .join()
        .unwrap();
}
