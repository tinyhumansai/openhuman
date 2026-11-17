use super::*;

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

#[test]
fn generators_with_a_router_are_rebuilt_for_the_chat() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tinyagents_harness::tinyinference_image::MockImageGenerator;
    use tinyagents_harness::tinyinference_video::{MockVideoGenerator, MockVideoScript};

    let built_for_chat = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&built_for_chat);
    let router: GeneratorRouter = Arc::new(move |chat| {
        if chat.is_some() {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        let image: Arc<dyn ImageGenerator> = Arc::new(MockImageGenerator::new());
        let video: Arc<dyn VideoGenerator> =
            Arc::new(MockVideoGenerator::new(MockVideoScript::delivers()));
        (image, video)
    });
    let (image, video) = router(None);
    let generators = MediaGenerators::with_router(image, video, router);
    let routed = generators.routed(Some(&ApprovalChatContext {
        thread_id: "t".into(),
        client_id: "c".into(),
        request_id: None,
    }));
    assert_eq!(built_for_chat.load(Ordering::SeqCst), 1);
    assert!(!Arc::ptr_eq(&generators.image, &routed.image));
}
