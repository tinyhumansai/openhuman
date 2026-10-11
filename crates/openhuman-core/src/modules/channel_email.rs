//! Podcast attachment delivery through the loadable TinyChannels module.
use super::client::{ModuleCallError, ModuleClient};
use async_trait::async_trait;
use std::sync::Arc;
use tinychannels_bus::controllers::ChannelSendMessageResult;
use tinychannels_bus::module_email::{EMAIL_ATTACHMENT_BYTES, EMAIL_UPLOAD_CHUNK_BYTES};
use tinychannels_bus::{methods, EmailUploadBegin, EmailUploadChunk, EmailUploadHandle};

#[async_trait]
trait EmailUploadTransport: Send + Sync {
    async fn begin(&self, request: EmailUploadBegin) -> Result<EmailUploadHandle, ModuleCallError>;
    async fn append(&self, request: EmailUploadChunk) -> Result<u32, ModuleCallError>;
    async fn send(
        &self,
        handle: EmailUploadHandle,
    ) -> Result<ChannelSendMessageResult, ModuleCallError>;
    async fn abort(&self, handle: EmailUploadHandle) -> Result<(), ModuleCallError>;
}

#[async_trait]
impl EmailUploadTransport for ModuleClient {
    async fn begin(&self, request: EmailUploadBegin) -> Result<EmailUploadHandle, ModuleCallError> {
        self.call_confidential("tinychannels", methods::BEGIN_EMAIL_UPLOAD, (request,))
            .await
    }
    async fn append(&self, request: EmailUploadChunk) -> Result<u32, ModuleCallError> {
        self.call_confidential(
            "tinychannels",
            methods::APPEND_EMAIL_UPLOAD_CHUNK,
            (request,),
        )
        .await
    }
    async fn send(
        &self,
        handle: EmailUploadHandle,
    ) -> Result<ChannelSendMessageResult, ModuleCallError> {
        self.call_confidential("tinychannels", methods::SEND_EMAIL_UPLOAD, (handle,))
            .await
    }
    async fn abort(&self, handle: EmailUploadHandle) -> Result<(), ModuleCallError> {
        self.call_confidential("tinychannels", methods::ABORT_EMAIL_UPLOAD, (handle,))
            .await
    }
}

/// Transfer one validated attachment in bounded confidential frames and send it.
/// SMTP and MIME delivery execute inside the module; failures have no local fallback.
pub(crate) async fn send_attachment(
    client: ModuleClient,
    request: EmailUploadBegin,
    bytes: Vec<u8>,
) -> Result<ChannelSendMessageResult, ModuleCallError> {
    upload_and_send(Arc::new(client), request, bytes).await
}

async fn upload_and_send<T: EmailUploadTransport + 'static>(
    transport: Arc<T>,
    request: EmailUploadBegin,
    bytes: Vec<u8>,
) -> Result<ChannelSendMessageResult, ModuleCallError> {
    if bytes.len() > EMAIL_ATTACHMENT_BYTES
        || request.attachments.len() != 1
        || request.attachments[0].byte_count != bytes.len()
    {
        return Err(ModuleCallError::ModuleFault);
    }
    let handle = transport.begin(request).await?;
    let mut lease = UploadLease {
        transport: transport.clone(),
        handle: Some(handle.clone()),
    };
    let result = async {
        for (sequence, bytes) in bytes.chunks(EMAIL_UPLOAD_CHUNK_BYTES).enumerate() {
            let expected = sequence as u32 + 1;
            let acknowledged = transport
                .append(EmailUploadChunk {
                    handle: handle.clone(),
                    attachment_index: 0,
                    sequence: sequence as u32,
                    bytes: bytes.to_vec(),
                })
                .await?;
            if acknowledged != expected {
                ModuleClient::report_malformed_reply("tinychannels");
                return Err(ModuleCallError::ModuleFault);
            }
        }
        transport.send(handle.clone()).await
    }
    .await;
    if result.is_err() {
        // A send already admitted to SMTP remains module-owned; Abort refuses
        // to cancel it. Failed uploads are explicitly closed before returning.
        if transport.abort(handle).await.is_ok() {
            lease.handle.take();
        }
    } else {
        lease.handle.take();
    }
    result
}

struct UploadLease<T: EmailUploadTransport + 'static> {
    transport: Arc<T>,
    handle: Option<EmailUploadHandle>,
}
impl<T: EmailUploadTransport + 'static> Drop for UploadLease<T> {
    fn drop(&mut self) {
        if let (Some(handle), Ok(runtime)) =
            (self.handle.take(), tokio::runtime::Handle::try_current())
        {
            let transport = self.transport.clone();
            runtime.spawn(async move {
                let _ = transport.abort(handle).await;
            });
        }
    }
}

#[cfg(test)]
#[path = "channel_email_tests.rs"]
mod tests;
