use super::*;
use crate::core::events::DomainEvent;

fn guard(chat: Option<ApprovalChatContext>) -> Guard {
    Guard {
        config: Arc::new(Config::default()),
        chat,
    }
}

async fn disclosed_routing(
    rx: &mut tinybus::events::EventReceiver<DomainEvent>,
    service: &str,
) -> (Option<String>, Option<String>) {
    loop {
        match rx.recv().await {
            Some(DomainEvent::ExternalTransferPending {
                descriptor,
                thread_id,
                client_id,
                ..
            }) if descriptor.service == service => return (thread_id, client_id),
            Some(_) => continue,
            None => panic!("the bus closed before the disclosure arrived"),
        }
    }
}

/// A media request discloses its egress to the chat the call was built for, and
/// two guards built for different chats do not share a destination.
#[tokio::test]
async fn a_guard_discloses_to_the_chat_it_was_built_for() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

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
        disclosed_routing(&mut rx, &service("videos/probe-a")).await,
        (Some("thread-media-a".into()), Some("client-media-a".into()))
    );
    assert_eq!(
        disclosed_routing(&mut rx, &service("videos/probe-b")).await,
        (Some("thread-media-b".into()), Some("client-media-b".into()))
    );
    assert_eq!(
        disclosed_routing(&mut rx, &service("videos/probe-none")).await,
        (None, None)
    );
}

#[test]
fn generators_without_a_router_are_returned_as_they_are() {
    use tinyagents_harness::tinyinference_image::MockImageGenerator;
    use tinyagents_harness::tinyinference_video::{MockVideoGenerator, MockVideoScript};
    let generators = MediaGenerators::new(
        Arc::new(MockImageGenerator::new()),
        Arc::new(MockVideoGenerator::new(MockVideoScript::delivers())),
    );
    let routed = generators.routed(Some(&ApprovalChatContext {
        thread_id: "t".into(),
        client_id: "c".into(),
        request_id: None,
    }));
    assert!(Arc::ptr_eq(&generators.image, &routed.image));
    assert!(Arc::ptr_eq(&generators.video, &routed.video));
}
