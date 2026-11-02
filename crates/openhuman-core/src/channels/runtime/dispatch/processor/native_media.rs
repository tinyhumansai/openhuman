//! Resolve typed native-channel media through TinyChannels' scoped media port.

use tinybus::module::{ModuleCallError, ModuleObjectClient};
use tinychannels_bus::{
    module_media::{MediaReadBegin, MediaReadChunk, MediaReadFinish},
    ChannelInboundEnvelope, ContentPart, HostVoiceScope, MediaKind, MediaReference,
};
use tinyinference_llm::message::Message;

use crate::{channels::context::ChannelRuntimeContext, core::runtime::DispatchCtx};

const MAX_MEDIA_BYTES: usize = tinychannels_bus::module_media::MEDIA_UPLOAD_MAX_BYTES;

pub(super) fn requires_typed_preparation(envelope: &ChannelInboundEnvelope) -> bool {
    envelope.parts.iter().any(|part| {
        matches!(
            part,
            ContentPart::Image { .. }
                | ContentPart::File { .. }
                | ContentPart::Audio { .. }
                | ContentPart::Video { .. }
                | ContentPart::VoiceNote { .. }
        )
    })
}

fn log_rejection(stage: &'static str, error: String) -> String {
    // Error text can contain storage details or provider-controlled values.
    // Keep diagnostics useful without putting those values into channel logs.
    tracing::warn!(stage, "[dispatch] typed native media preparation rejected");
    error
}

fn log_module_call_rejection(
    method: &'static str,
    error: ModuleCallError,
    fallback: &'static str,
) -> String {
    let class = match error {
        ModuleCallError::Unavailable => "unavailable",
        ModuleCallError::IncompatibleContract => "incompatible_contract",
        ModuleCallError::TransportFailed => "transport_failed",
        ModuleCallError::ModuleFault => "module_fault",
    };
    tracing::warn!(
        method,
        module_error = class,
        "[dispatch] native media module call rejected"
    );
    fallback.to_owned()
}

/// Resolve only opaque references carried by the trusted module envelope.
/// Provider URLs and paths are never used as a filesystem or network source.
pub(super) async fn resolve_typed_message(
    ctx: &ChannelRuntimeContext,
    host_dispatch: &DispatchCtx,
    config: &crate::config::Config,
    envelope: &ChannelInboundEnvelope,
    thread_id: &str,
    channel_context: &str,
) -> Result<Option<ResolvedNativeMessage>, String> {
    if !requires_typed_preparation(envelope) {
        return Ok(None);
    }
    validate_native_parts(&envelope.parts)
        .map_err(|error| log_rejection("part_validation", error))?;
    let scope = media_scope(envelope).map_err(|error| log_rejection("scope", error))?;
    let mut parts = envelope.parts.clone();
    let mut staged_parts = Vec::new();
    let mut image_count = 0usize;
    let mut file_count = 0usize;
    let (max_images, image_mb) = config.multimodal.effective_limits();
    let (configured_files, file_mb, _) = config.multimodal_files.effective_limits();
    let max_files = if config.multimodal_files.max_files == 0 {
        0
    } else {
        configured_files
    };
    let image_limit = image_mb.saturating_mul(1024 * 1024).min(MAX_MEDIA_BYTES);
    let file_limit = file_mb.saturating_mul(1024 * 1024).min(MAX_MEDIA_BYTES);
    for part in &parts {
        match part {
            ContentPart::Image { media, .. } => {
                image_count += 1;
                validate_reference(
                    media,
                    true,
                    image_limit,
                    &config.multimodal_files.allowed_mime_types,
                )
                .map_err(|error| log_rejection("image_reference", error))?;
            }
            ContentPart::File { media, .. } => {
                file_count += 1;
                validate_reference(
                    media,
                    false,
                    file_limit,
                    &config.multimodal_files.allowed_mime_types,
                )
                .map_err(|error| log_rejection("file_reference", error))?;
            }
            _ => {}
        }
    }
    if (config.multimodal.max_images == 0 && image_count > 0) || image_count > max_images {
        return Err(log_rejection(
            "image_count_limit",
            "native image attachments exceed their count limit".into(),
        ));
    }
    if file_count > max_files {
        return Err(log_rejection(
            "file_count_limit",
            "native file attachments are disabled or exceed their count limit".into(),
        ));
    }
    // Media reads must reach the same configured module object that owns the
    // provider activation and its opaque media store. A process-wide client
    // creates a callback-only runtime with no running provider state.
    let client = ctx
        .module_objects
        .get(&scope.configured_instance)
        .cloned()
        .ok_or_else(|| {
            log_rejection(
                "module_instance",
                "configured native media module is unavailable".into(),
            )
        })?;
    let module_dispatch = ctx.dispatch_ctx();
    let mut media_payloads = Vec::new();
    for (index, part) in parts.iter_mut().enumerate() {
        let (media, image, name) = match part {
            ContentPart::Image { media, .. } => (
                media.clone(),
                true,
                media.file_name.clone().unwrap_or_else(|| "image".into()),
            ),
            ContentPart::File {
                media, file_name, ..
            } => (
                media.clone(),
                false,
                file_name
                    .clone()
                    .or_else(|| media.file_name.clone())
                    .unwrap_or_else(|| "attachment".into()),
            ),
            ContentPart::Audio { .. } | ContentPart::Video { .. } => {
                return Err("native audio/video attachments are not supported".into());
            }
            ContentPart::VoiceNote { voice_note } => {
                let transcript = voice_note
                    .transcript
                    .as_ref()
                    .expect("validated transcript");
                let replacement = ContentPart::Text {
                    text: transcript.text.trim().to_owned(),
                    markdown_dialect: None,
                };
                *part = replacement;
                continue;
            }
            ContentPart::Text { .. } => continue,
            _ => return Err("native channel content kind is not supported".into()),
        };
        let byte_limit = if image { image_limit } else { file_limit };
        let mime = media
            .content_type
            .clone()
            .ok_or_else(|| "native media has no content type".to_owned())
            .map_err(|error| log_rejection("media_metadata", error))?;
        let bytes = read_scoped_media(
            &client,
            &module_dispatch,
            &scope.configured_instance,
            &scope,
            &media,
            byte_limit,
        )
        .await
        .map_err(|error| log_rejection("module_media_read", error))?;
        media_payloads.push((index, part.clone(), name, mime, bytes));
    }
    let staged_lease = crate::threads::attachments::NativeStagedLease::new(
        host_dispatch.clone(),
        thread_id.to_owned(),
        config.clone(),
        Vec::new(),
    );
    for (index, input, name, mime, bytes) in media_payloads {
        let staged = match crate::threads::attachments::stage_native_channel_part(
            host_dispatch,
            thread_id,
            config,
            &input,
            &name,
            &mime,
            &bytes,
            &staged_lease,
        )
        .await
        {
            Ok(staged) => staged,
            Err(error) => {
                let _ = staged_lease.discard().await;
                return Err(log_rejection("host_staging", error));
            }
        };
        let opaque_id = media_opaque_id(&staged)
            .ok_or_else(|| "staged native media has no host reference".to_owned())
            .map_err(|error| log_rejection("host_staging", error))?;
        let mut model_part = staged.clone();
        let model_media = match &mut model_part {
            ContentPart::Image { media, .. } | ContentPart::File { media, .. } => media,
            _ => {
                return Err(log_rejection(
                    "host_staging",
                    "native staged media kind changed unexpectedly".into(),
                ));
            }
        };
        model_media.path = Some(format!("openhuman-staged://{opaque_id}"));
        parts[index] = model_part;
        staged_parts.push(staged);
    }

    let message =
        match crate::agent::message_convert::user_message_from_parts(channel_context, &parts) {
            Ok(message) => message,
            Err(error) => {
                let _ = staged_lease.discard().await;
                return Err(log_rejection("message_conversion", error));
            }
        };
    Ok(Some(ResolvedNativeMessage {
        message,
        staged_parts,
        staged_lease,
    }))
}

fn media_opaque_id(part: &ContentPart) -> Option<&str> {
    match part {
        ContentPart::Image { media, .. } | ContentPart::File { media, .. } => {
            media.opaque_id.as_deref()
        }
        _ => None,
    }
}

pub(super) struct ResolvedNativeMessage {
    pub(super) message: Message,
    pub(super) staged_parts: Vec<ContentPart>,
    pub(super) staged_lease: crate::threads::attachments::NativeStagedLease,
}

fn media_scope(envelope: &ChannelInboundEnvelope) -> Result<HostVoiceScope, String> {
    let configured_instance = envelope
        .channel
        .account_id
        .as_deref()
        .filter(|account| !account.trim().is_empty())
        .ok_or_else(|| "native media has no configured account scope".to_owned())?;
    if envelope.channel.id.trim().is_empty()
        || envelope.conversation.id.trim().is_empty()
        || envelope.message_id.trim().is_empty()
    {
        return Err("native media has an incomplete conversation scope".into());
    }
    if tinychannels_bus::provider_id(configured_instance) != envelope.channel.id {
        return Err("native media account does not match its provider".into());
    }
    Ok(HostVoiceScope {
        configured_instance: configured_instance.to_owned(),
        channel: envelope.channel.clone(),
        conversation: envelope.conversation.clone(),
        turn_id: envelope.message_id.clone(),
    })
}

fn validate_native_parts(parts: &[ContentPart]) -> Result<(), String> {
    for part in parts {
        match part {
            ContentPart::Text { .. } | ContentPart::Image { .. } | ContentPart::File { .. } => {}
            ContentPart::VoiceNote { voice_note } => {
                let transcript = voice_note
                    .transcript
                    .as_ref()
                    .filter(|transcript| {
                        transcript.state == tinychannels_bus::VoiceTranscriptState::Complete
                    })
                    .map(|transcript| transcript.text.trim())
                    .filter(|text| !text.is_empty());
                if transcript.is_none() {
                    return Err("native voice transcript is unavailable".into());
                }
            }
            ContentPart::Audio { .. } | ContentPart::Video { .. } => {
                return Err("native audio/video attachments are not supported".into());
            }
            _ => return Err("native channel content kind is not supported".into()),
        }
    }
    Ok(())
}

fn validate_reference(
    media: &MediaReference,
    image: bool,
    byte_limit: usize,
    allowed_file_mimes: &[String],
) -> Result<(), String> {
    let opaque_id = media
        .opaque_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| "native media reference is missing its opaque id".to_owned())?;
    if opaque_id.len() > tinychannels_bus::module_media::MEDIA_UPLOAD_BEGIN_MAX_BYTES
        || media.path.is_some()
        || media.url.is_some()
        || media.local_cache_path.is_some()
    {
        return Err("native media reference is not an opaque module reference".into());
    }
    if media
        .size_bytes
        .is_some_and(|size| size == 0 || size > byte_limit as u64)
    {
        return Err("native media exceeds its declared byte limit".into());
    }
    let mime = media
        .content_type
        .as_deref()
        .filter(|mime| !mime.trim().is_empty())
        .ok_or_else(|| "native media has no content type".to_owned())?;
    if image {
        if media.kind != MediaKind::Image {
            return Err("native media kind does not match its image part".into());
        }
        if !matches!(
            mime,
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            return Err("native image MIME type is unsupported".into());
        }
    } else {
        if !matches!(media.kind, MediaKind::Document | MediaKind::Unknown) {
            return Err("native media kind does not match its file part".into());
        }
        if !allowed_file_mimes.iter().any(|allowed| allowed == mime) {
            return Err("native file MIME type is unsupported".into());
        }
    }
    Ok(())
}

fn validate_actual_media_size(
    actual: usize,
    declared: usize,
    maximum: usize,
) -> Result<(), String> {
    if actual == 0 || actual != declared || actual > maximum || actual > MAX_MEDIA_BYTES {
        return Err("native media bytes exceed or disagree with their declared limit".into());
    }
    Ok(())
}

async fn read_scoped_media(
    client: &ModuleObjectClient,
    dispatch: &DispatchCtx,
    instance: &str,
    scope: &HostVoiceScope,
    media: &MediaReference,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let begin = client
        .call_confidential::<tinychannels_bus::MediaReadInfo>(
            tinychannels_bus::methods::BEGIN_MEDIA_READ,
            (MediaReadBegin {
                instance: instance.to_owned(),
                scope: scope.clone(),
                media: media.clone(),
                max_bytes: max_bytes as u64,
            },),
        )
        .await
        .map_err(|error| {
            log_module_call_rejection(
                tinychannels_bus::methods::BEGIN_MEDIA_READ,
                error,
                "native media could not be resolved in this turn",
            )
        })?;
    let mut lease = MediaReadLease::new(client.clone(), dispatch.clone(), begin.handle.clone());
    let byte_count_valid = begin.byte_count > 0
        && begin.byte_count <= max_bytes
        && begin.byte_count <= MAX_MEDIA_BYTES;
    let opaque_reference_matches = begin.media.opaque_id == media.opaque_id;
    let kind_matches = begin.media.kind == media.kind;
    let provider_locations_absent = begin.media.path.is_none()
        && begin.media.url.is_none()
        && begin.media.local_cache_path.is_none();
    let content_type_matches =
        begin.content_type == media.content_type.as_deref().unwrap_or_default();
    let declared_size_matches = media
        .size_bytes
        .is_none_or(|size| size == begin.byte_count as u64);
    if !(byte_count_valid
        && opaque_reference_matches
        && kind_matches
        && provider_locations_absent
        && content_type_matches
        && declared_size_matches)
    {
        tracing::warn!(
            byte_count_valid,
            opaque_reference_matches,
            kind_matches,
            provider_locations_absent,
            content_type_matches,
            declared_size_matches,
            "[dispatch] native media module returned mismatched read metadata"
        );
        return Err("native media read metadata does not match its scoped reference".into());
    }
    let expected_chunks = begin
        .byte_count
        .div_ceil(tinychannels_bus::module_media::MEDIA_READ_CHUNK_BYTES);
    if begin.chunk_count as usize != expected_chunks {
        return Err(log_rejection(
            "module_read_chunk_count",
            "native media read chunk count is invalid".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(begin.byte_count);
    for sequence in 0..begin.chunk_count {
        let chunk = client
            .call_confidential::<tinychannels_bus::MediaReadChunkResult>(
                tinychannels_bus::methods::READ_MEDIA_CHUNK,
                (MediaReadChunk {
                    handle: begin.handle.clone(),
                    sequence,
                },),
            )
            .await
            .map_err(|error| {
                log_module_call_rejection(
                    tinychannels_bus::methods::READ_MEDIA_CHUNK,
                    error,
                    "native media read was interrupted",
                )
            })?;
        let expected_done = sequence + 1 == begin.chunk_count;
        if chunk.sequence != sequence || chunk.done != expected_done {
            return Err(log_rejection(
                "module_read_sequence",
                "native media read returned an invalid chunk sequence".into(),
            ));
        }
        if chunk.bytes.len() > tinychannels_bus::module_media::MEDIA_READ_CHUNK_BYTES {
            return Err(log_rejection(
                "module_read_chunk_size",
                "native media read returned an oversized chunk".into(),
            ));
        }
        if bytes.len().saturating_add(chunk.bytes.len()) > begin.byte_count
            || bytes.len().saturating_add(chunk.bytes.len()) > max_bytes
        {
            return Err(log_rejection(
                "module_read_total_size",
                "native media bytes exceed their declared limit".into(),
            ));
        }
        bytes.extend(chunk.bytes);
    }
    validate_actual_media_size(bytes.len(), begin.byte_count, max_bytes)
        .map_err(|error| log_rejection("module_read_final_size", error))?;
    lease.abort().await.map_err(|error| {
        log_module_call_rejection(
            tinychannels_bus::methods::ABORT_MEDIA_READ,
            error,
            "native media read cleanup failed",
        )
    })?;
    Ok(bytes)
}

struct MediaReadLease {
    client: ModuleObjectClient,
    dispatch: DispatchCtx,
    handle: Option<tinychannels_bus::MediaReadHandle>,
}

impl MediaReadLease {
    fn new(
        client: ModuleObjectClient,
        dispatch: DispatchCtx,
        handle: tinychannels_bus::MediaReadHandle,
    ) -> Self {
        Self {
            client,
            dispatch,
            handle: Some(handle),
        }
    }

    async fn abort(&mut self) -> Result<(), ModuleCallError> {
        let Some(handle) = self.handle.as_ref().cloned() else {
            return Ok(());
        };
        self.client
            .call_confidential::<()>(
                tinychannels_bus::methods::ABORT_MEDIA_READ,
                (MediaReadFinish(handle),),
            )
            .await?;
        self.handle = None;
        Ok(())
    }
}

impl Drop for MediaReadLease {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        let client = self.client.clone();
        self.dispatch.spawn(async move {
            let _ = client
                .call_confidential::<()>(
                    tinychannels_bus::methods::ABORT_MEDIA_READ,
                    (MediaReadFinish(handle),),
                )
                .await;
        });
    }
}

#[cfg(test)]
pub(crate) fn drop_media_read_lease_for_test(
    client: ModuleObjectClient,
    dispatch: DispatchCtx,
    handle: tinychannels_bus::MediaReadHandle,
) {
    drop(MediaReadLease::new(client, dispatch, handle));
}

#[cfg(test)]
#[path = "native_media_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_media_flow_tests.rs"]
pub(in crate::channels::runtime::dispatch::processor) mod flow_tests;
