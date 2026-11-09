use super::*;

fn identity(slot: u16) -> SecretCipherDeliveryIdentity2 {
    SecretCipherDeliveryIdentity2 {
        consumer_id: "consumer-id".into(),
        registration_id: "registration-id".into(),
        operation_id: "operation-id".into(),
        output_id: "output-id".into(),
        generation: 1,
        operation: tinysecurity_bus::SecretCipherKind2::Decrypt,
        slot,
        expires_at_ms: 10,
    }
}

#[tokio::test]
async fn only_the_matching_success_identity_can_release_staged_plaintext() {
    let consumer = Consumer::new("registration-id".into(), "consumer-id".into());
    let expected = identity(0);
    consumer
        .deliver(SecretCipherDeliveryRequest2 {
            identity: expected.clone(),
            plaintext: b"secret value".to_vec(),
            replacement: Some(b"replacement".to_vec()),
        })
        .await
        .unwrap();

    let mut mismatch = expected.clone();
    mismatch.output_id.push('x');
    assert_eq!(
        consumer.take(&mismatch).unwrap_err(),
        "cipher output unavailable"
    );

    let (plaintext, replacement) = consumer.take(&expected).unwrap();
    assert_eq!(&*plaintext, "secret value");
    assert_eq!(
        replacement.as_deref().map(String::as_str),
        Some("replacement")
    );
    assert_eq!(
        consumer.take(&expected).unwrap_err(),
        "cipher output unavailable"
    );
}

#[tokio::test]
async fn a_staged_slot_cannot_be_overwritten() {
    let consumer = Consumer::new("registration-id".into(), "consumer-id".into());
    let first = identity(0);
    consumer
        .deliver(SecretCipherDeliveryRequest2 {
            identity: first.clone(),
            plaintext: b"first".to_vec(),
            replacement: None,
        })
        .await
        .unwrap();

    let mut second = first;
    second.output_id.push('2');
    assert!(consumer
        .deliver(SecretCipherDeliveryRequest2 {
            identity: second,
            plaintext: b"second".to_vec(),
            replacement: None,
        })
        .await
        .is_err());
}

#[tokio::test]
async fn consumer_rejects_outputs_for_another_registration_or_consumer() {
    let consumer = Consumer::new("registration-id".into(), "consumer-id".into());
    for mut identity in [identity(0), identity(1)] {
        if identity.slot == 0 {
            identity.registration_id.push('x');
        } else {
            identity.consumer_id.push('x');
        }
        assert!(consumer
            .deliver(SecretCipherDeliveryRequest2 {
                identity,
                plaintext: b"secret".to_vec(),
                replacement: None,
            })
            .await
            .is_err());
    }
}
