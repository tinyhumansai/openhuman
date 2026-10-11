use super::*;
use std::sync::Mutex;
use tinychannels_bus::{config::EmailConfig, EmailAttachmentUpload};

#[derive(Default)]
struct Fixture {
    chunks: Mutex<Vec<EmailUploadChunk>>,
    sent: Mutex<bool>,
    aborted: Mutex<bool>,
    bad_ack: bool,
    append_entered: Option<tokio::sync::Notify>,
    abort_finished: tokio::sync::Notify,
}
#[async_trait]
impl EmailUploadTransport for Fixture {
    async fn begin(&self, request: EmailUploadBegin) -> Result<EmailUploadHandle, ModuleCallError> {
        assert_eq!(request.recipient, "listener@example.test");
        assert_eq!(request.subject, "Podcast");
        assert_eq!(request.attachments[0].filename, "podcast.mp3");
        Ok(EmailUploadHandle("module-owned-upload".into()))
    }
    async fn append(&self, chunk: EmailUploadChunk) -> Result<u32, ModuleCallError> {
        if let Some(entered) = &self.append_entered {
            entered.notify_one();
            std::future::pending::<()>().await;
        }
        let ack = if self.bad_ack { 0 } else { chunk.sequence + 1 };
        self.chunks.lock().unwrap().push(chunk);
        Ok(ack)
    }
    async fn send(
        &self,
        handle: EmailUploadHandle,
    ) -> Result<ChannelSendMessageResult, ModuleCallError> {
        assert_eq!(handle.0, "module-owned-upload");
        *self.sent.lock().unwrap() = true;
        Ok(ChannelSendMessageResult::default())
    }
    async fn abort(&self, _: EmailUploadHandle) -> Result<(), ModuleCallError> {
        *self.aborted.lock().unwrap() = true;
        self.abort_finished.notify_one();
        Ok(())
    }
}
fn request(length: usize) -> EmailUploadBegin {
    EmailUploadBegin {
        config: EmailConfig::default(),
        recipient: "listener@example.test".into(),
        subject: "Podcast".into(),
        body: "Attached.".into(),
        attachments: vec![EmailAttachmentUpload {
            filename: "podcast.mp3".into(),
            content_type: "audio/mpeg".into(),
            byte_count: length,
        }],
    }
}
#[tokio::test]
async fn podcast_attachment_uses_bounded_ordered_chunks_before_module_send() {
    let fixture = Arc::new(Fixture::default());
    let bytes = vec![42; EMAIL_UPLOAD_CHUNK_BYTES + 3];
    upload_and_send(fixture.clone(), request(bytes.len()), bytes.clone())
        .await
        .unwrap();
    let chunks = fixture.chunks.lock().unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].sequence, 0);
    assert_eq!(chunks[1].sequence, 1);
    assert!(chunks
        .iter()
        .all(|chunk| chunk.bytes.len() <= EMAIL_UPLOAD_CHUNK_BYTES && chunk.attachment_index == 0));
    assert_eq!(
        chunks
            .iter()
            .flat_map(|chunk| chunk.bytes.clone())
            .collect::<Vec<_>>(),
        bytes
    );
    assert!(*fixture.sent.lock().unwrap());
    assert!(!*fixture.aborted.lock().unwrap());
}
#[tokio::test]
async fn incorrect_upload_receipt_aborts_without_smtp_delivery() {
    let fixture = Arc::new(Fixture {
        bad_ack: true,
        ..Fixture::default()
    });
    assert_eq!(
        upload_and_send(fixture.clone(), request(1), vec![1])
            .await
            .unwrap_err(),
        ModuleCallError::ModuleFault
    );
    assert!(*fixture.aborted.lock().unwrap());
    assert!(!*fixture.sent.lock().unwrap());
}
#[tokio::test]
async fn inconsistent_attachment_metadata_is_refused_before_module_upload() {
    let fixture = Arc::new(Fixture::default());
    assert_eq!(
        upload_and_send(fixture.clone(), request(2), vec![1])
            .await
            .unwrap_err(),
        ModuleCallError::ModuleFault
    );
    assert!(fixture.chunks.lock().unwrap().is_empty());
    assert!(!*fixture.sent.lock().unwrap());
}

#[tokio::test]
async fn canceled_attachment_upload_closes_its_module_handle_without_sending() {
    let fixture = Arc::new(Fixture {
        append_entered: Some(tokio::sync::Notify::new()),
        ..Fixture::default()
    });
    let task = tokio::spawn(upload_and_send(fixture.clone(), request(1), vec![42]));
    fixture.append_entered.as_ref().unwrap().notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fixture.abort_finished.notified(),
    )
    .await
    .unwrap();
    assert!(*fixture.aborted.lock().unwrap());
    assert!(!*fixture.sent.lock().unwrap());
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn unattested_bus_never_receives_email_credentials_or_retries_plaintext() {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};
    #[derive(Clone)]
    struct Unattested(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    // An in-process fixture deliberately has no module attestation.
    #[tinybus::interface(name = "test.ChannelEmail")]
    impl Unattested {
        async fn begin_email_upload(
            &self,
            _: EmailUploadBegin,
        ) -> tinybus::Result<EmailUploadHandle> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(EmailUploadHandle("unattested".into()))
        }
    }
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    service
        .serve_at(
            ObjectPath::new("/test/ChannelEmail").unwrap(),
            Unattested(calls.clone()),
        )
        .await
        .unwrap();
    service.request_name("test.ChannelEmail").await.unwrap();
    let proxy = caller
        .proxy(
            "test.ChannelEmail",
            "/test/ChannelEmail",
            "test.ChannelEmail",
        )
        .unwrap();
    assert_eq!(
        send_attachment(ModuleClient::fixture(proxy), request(1), vec![42])
            .await
            .unwrap_err(),
        ModuleCallError::TransportFailed
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}
