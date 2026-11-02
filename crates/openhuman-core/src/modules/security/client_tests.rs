use super::*;
use std::sync::{Arc, Mutex};
use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};

#[derive(Clone, Default)]
struct DeviceFixture {
    pairing: Arc<Mutex<Option<tinysecurity_bus::DevicePairingRequest2>>>,
    frame: Arc<Mutex<Option<tinysecurity_bus::DeviceFrameRequest2>>>,
}

#[tinybus::interface(name = "ai.tinyhumans.tinysecurity.Policy")]
impl DeviceFixture {
    #[tinybus(confidential)]
    async fn device_pairing2(
        &self,
        request: tinysecurity_bus::DevicePairingRequest2,
    ) -> tinybus::Result<tinysecurity_bus::DevicePairingResponse2> {
        *self.pairing.lock().unwrap() = Some(request);
        Ok(tinysecurity_bus::DevicePairingResponse2 {
            receipt: None,
            pairing_id: None,
            session_id: None,
            core_public_key: None,
            device_public_key: None,
            frame: vec![3],
            expires_at_ms: 0,
        })
    }

    #[tinybus(confidential)]
    async fn device_frame2(
        &self,
        request: tinysecurity_bus::DeviceFrameRequest2,
    ) -> tinybus::Result<tinysecurity_bus::DeviceFrameResponse2> {
        *self.frame.lock().unwrap() = Some(request);
        Ok(tinysecurity_bus::DeviceFrameResponse2 { bytes: vec![2] })
    }
}

#[tokio::test]
async fn device_pairing_and_frame_calls_reject_unattested_recipients() {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let fixture = DeviceFixture::default();
    service
        .serve_at(
            ObjectPath::new(tinysecurity_bus::names::OBJECT_PATH).unwrap(),
            fixture.clone(),
        )
        .await
        .unwrap();
    service.request_name("test.TinySecurity").await.unwrap();
    let proxy = caller
        .proxy(
            "test.TinySecurity",
            tinysecurity_bus::names::OBJECT_PATH,
            tinysecurity_bus::names::INTERFACE,
        )
        .unwrap();
    let object = crate::modules::ops::fixture_client(proxy)
        .object_client(
            super::super::MODULE_ID,
            tinysecurity_bus::names::OBJECT_PATH,
        )
        .await
        .unwrap();
    let client = Client::from_object(object);

    let pairing = tinysecurity_bus::DevicePairingRequest2 {
        call_id: "host-call".into(),
        operation: tinysecurity_bus::DevicePairingOperation2::Begin {
            host_event_id: "event".into(),
        },
    };
    let raw_proxy = caller
        .proxy(
            "test.TinySecurity",
            tinysecurity_bus::names::OBJECT_PATH,
            tinysecurity_bus::names::INTERFACE,
        )
        .unwrap();
    let raw: Result<tinysecurity_bus::DevicePairingResponse2, tinybus::Error> = raw_proxy
        .call_confidential(
            tinysecurity_bus::names::methods::DEVICE_PAIRING2,
            (pairing.clone(),),
        )
        .await;
    assert!(matches!(
        raw,
        Err(tinybus::Error::MethodFailed { name, .. }) if name == tinybus::Error::NOT_ATTESTED
    ));
    assert!(matches!(
        client.device_pairing2(pairing).await,
        Err(SecurityError::Unavailable(_))
    ));
    assert!(fixture.pairing.lock().unwrap().is_none());

    let frame = tinysecurity_bus::DeviceFrameRequest2 {
        scope_id: "scope".into(),
        handle: "opaque-session".into(),
        call_id: "frame-call".into(),
        operation: tinysecurity_bus::DeviceFrameOperation2::Open { frame: vec![7] },
    };
    assert!(matches!(
        client.device_frame2(frame).await,
        Err(SecurityError::Unavailable(_))
    ));
    assert!(fixture.frame.lock().unwrap().is_none());
}
