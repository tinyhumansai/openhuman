use super::*;
use openhuman_embed::__host::core::events::DomainEvent;

fn guard(chat: Option<ApprovalChatContext>) -> Guard {
    Guard {
        config: Arc::new(Config::default()),
        chat,
    }
}

/// Waits for the external-transfer disclosure of `service` on `rx` and returns
/// the `(thread_id, client_id)` it was routed to.
macro_rules! disclosed_routing {
    ($rx:expr, $service:expr) => {{
        let service: &str = $service;
        loop {
            match $rx.recv().await {
                Some(DomainEvent::ExternalTransferPending {
                    descriptor,
                    thread_id,
                    client_id,
                    ..
                }) if descriptor.service == service => break (thread_id, client_id),
                Some(_) => continue,
                None => panic!("the bus closed before the disclosure arrived"),
            }
        }
    }};
}

/// A media request discloses its egress to the chat the call was built for, and
/// two guards built for different chats do not share a destination.
#[tokio::test]
async fn a_guard_discloses_to_the_chat_it_was_built_for() {
    openhuman_embed::__host::core::bus::init()
        .await
        .expect("bus init");
    let mut rx = openhuman_embed::__host::core::bus::BUS
        .get()
        .unwrap()
        .receiver();

    let chat_a = ApprovalChatContext {
        thread_id: "thread-media-a".into(),
        client_id: "client-media-a".into(),
        request_id: None,
    };
    let chat_b = ApprovalChatContext {
        thread_id: "thread-media-b".into(),
        client_id: "client-media-b".into(),
        request_id: None,
    };
    let (a, b, none) = (guard(Some(chat_a)), guard(Some(chat_b)), guard(None));
    a.admit("videos/probe-a").unwrap();
    tokio::task::yield_now().await;
    b.admit("videos/probe-b").unwrap();
    none.admit("videos/probe-none").unwrap();

    let service = |route: &str| format!("{OPENROUTER_PROXY_PATH}/{route}");
    assert_eq!(
        disclosed_routing!(rx, &service("videos/probe-a")),
        (Some("thread-media-a".into()), Some("client-media-a".into()))
    );
    assert_eq!(
        disclosed_routing!(rx, &service("videos/probe-b")),
        (Some("thread-media-b".into()), Some("client-media-b".into()))
    );
    assert_eq!(
        disclosed_routing!(rx, &service("videos/probe-none")),
        (None, None)
    );
}
