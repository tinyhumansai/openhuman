use super::registry_test_support::{verify_family_contract, BoundFamilyWireContract};
use super::*;

#[test]
fn family_verifier_rejects_group_visibility_and_inventory_mutations() {
    use crate::core::runtime::DispatchCtx;

    fn parse(params: Map<String, Value>) -> Result<String, String> {
        params
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "missing id".into())
    }
    fn run(_ctx: DispatchCtx, _parsed: String) -> ControllerFuture {
        Box::pin(async { Ok(Value::Null) })
    }
    fn row(group: DomainGroup) -> GroupedController {
        GroupedController {
            group,
            controller: ControllerEntry::bound(BoundRegisteredController::typed(
                ControllerSchema {
                    namespace: "fixture",
                    function: "read",
                    description: "Read one fixture.",
                    inputs: vec![FieldSchema {
                        name: "id",
                        ty: TypeSchema::String,
                        comment: "Fixture id.",
                        required: true,
                    }],
                    outputs: vec![],
                },
                parse,
                run,
            )),
        }
    }

    let contract = BoundFamilyWireContract {
        namespaces: &["fixture"],
        group: DomainGroup::Platform,
        methods: &[("fixture.read", "openhuman.fixture_read")],
    };
    let fixtures = [ControllerParamContract {
        method: "fixture.read",
        required: &["id"],
        optional: &[],
        examples: vec![serde_json::json!({"id":"one"})],
    }];
    let good = row(DomainGroup::Platform);
    assert!(verify_family_contract(std::slice::from_ref(&good), &[], &contract, &fixtures).is_ok());
    assert!(
        verify_family_contract(&[row(DomainGroup::Security)], &[], &contract, &fixtures)
            .unwrap_err()
            .contains("wrong domain group")
    );
    assert!(verify_family_contract(
        std::slice::from_ref(&good),
        std::slice::from_ref(&good),
        &contract,
        &fixtures
    )
    .unwrap_err()
    .contains("internal registry"));

    for (methods, message) in [
        (
            &[("fixture.other", "openhuman.fixture_other")][..],
            "method inventory",
        ),
        (
            &[("fixture.read", "openhuman.fixture_other")][..],
            "RPC inventory",
        ),
    ] {
        let mutated = BoundFamilyWireContract {
            namespaces: &["fixture"],
            group: DomainGroup::Platform,
            methods,
        };
        assert!(
            verify_family_contract(std::slice::from_ref(&good), &[], &mutated, &fixtures)
                .unwrap_err()
                .contains(message)
        );
    }
    let wrong_fixture = [ControllerParamContract {
        method: "fixture.other",
        required: &["id"],
        optional: &[],
        examples: vec![serde_json::json!({"id":"one"})],
    }];
    assert!(
        verify_family_contract(std::slice::from_ref(&good), &[], &contract, &wrong_fixture)
            .unwrap_err()
            .contains("fixture inventory/order mismatch")
    );
}
