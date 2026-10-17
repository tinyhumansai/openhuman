use super::*;
use crate::security::egress::EgressLedger;

/// The per-turn egress ledger rides the factory chain as an explicit argument:
/// two managed constructions through one `EgressLedger::per_turn()` disclose the
/// destination once, while constructions with no ledger disclose each time.
#[tokio::test]
async fn managed_construction_ledger_collapses_repeat_disclosures() {
    use crate::core::events::DomainEvent;

    let _guard = crate::inference::inference_test_guard_async().await;
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let config = Config {
        default_model: Some("egress-ledger-factory-test-model".to_string()),
        ..Config::default()
    };
    let build = |ledger: &EgressLedger| {
        managed_backend::make_openhuman_backend_model_for_thread(
            "chat",
            &config,
            "egress-ledger-factory-test-model",
            true,
            Some("ledger-test-thread"),
            ledger,
        )
        .expect("managed construction")
    };

    let ledger = EgressLedger::per_turn();
    build(&ledger);
    build(&ledger);
    build(&EgressLedger::none());

    let sentinel = "egress-ledger-factory-sentinel";
    crate::security::egress::emit_external_transfer(
        None,
        crate::security::egress::EgressDescriptor::network_fetch(sentinel),
    );
    let mut disclosures = 0;
    loop {
        match rx.recv().await {
            Some(DomainEvent::ExternalTransferPending { descriptor, .. }) => {
                if descriptor.service == "egress-ledger-factory-test-model" {
                    disclosures += 1;
                }
                if descriptor.service == sentinel {
                    break;
                }
            }
            Some(_) => continue,
            None => panic!("the bus closed before the sentinel arrived"),
        }
    }
    assert_eq!(
        disclosures, 2,
        "two ledgered constructions disclose once; the ledger-less one discloses again"
    );
}
