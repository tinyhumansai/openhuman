//! Durable user uploads in the acting workspace. Provider resolution never
//! changes the original file or writes inline media into session records.

use crate::core::runtime::DispatchCtx;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tinyagents_harness::multimodal::{
    config::FileLimits, markers, resolve::resolve_attachment, UnknownMimePolicy,
};

use crate::config::Config;
use crate::security::SecurityPolicy;

pub(crate) mod host;
pub use host::HostAttachment;
pub(crate) mod legacy;
pub(crate) mod parts;
pub(crate) mod provider;
pub(crate) mod secure_open;

/// Per-model-call attachment authority copied from the live agent context.
/// The provider wrapper removes its request carrier before forwarding it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AttachmentAccessScope {
    pub(crate) external_channel: bool,
    pub(crate) workspace: Option<PathBuf>,
    pub(crate) native_staged_parts: Vec<tinychannels_bus::ContentPart>,
    pub(crate) thread_id: Option<String>,
}

const REQUEST_SCOPE_KEY: &str = "__openhuman_attachment_access_scope";
pub(super) const SOURCE_MISSING_MARKER: &str = "[attachment-source-missing]";

pub(crate) fn attach_request_scope(
    request: &mut tinyinference_llm::model::ModelRequest,
    context: &crate::agent::tinyagents::host::OpenHumanRunContext,
) {
    let external_channel = matches!(
        context.origin,
        Some(crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
    );
    let workspace = context
        .workspace
        .as_ref()
        .map(|workspace| workspace.root.clone());
    request.metadata[REQUEST_SCOPE_KEY] = serde_json::json!({
        "external_channel": external_channel,
        "workspace": workspace,
        "native_staged_parts": context.native_staged_parts,
        "thread_id": context.thread_id,
    });
}

pub(crate) fn take_request_scope(metadata: &mut serde_json::Value) -> AttachmentAccessScope {
    let value = metadata
        .as_object_mut()
        .and_then(|metadata| metadata.remove(REQUEST_SCOPE_KEY));
    let Some(value) = value else {
        return AttachmentAccessScope::default();
    };
    AttachmentAccessScope {
        external_channel: value
            .get("external_channel")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        workspace: value
            .get("workspace")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from),
        native_staged_parts: value
            .get("native_staged_parts")
            .and_then(|parts| serde_json::from_value(parts.clone()).ok())
            .unwrap_or_default(),
        thread_id: value
            .get("thread_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    }
}

const PREFIX: &str = "[ATTACHMENT:";

/// Metadata carried by a compact transcript reference. Paths are relative to
/// the acting workspace and can also be used from its `/workspace` mount.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub path: String,
    pub name: String,
    pub mime: String,
    pub size_bytes: usize,
}

impl Attachment {
    pub fn marker(&self) -> String {
        let json = serde_json::to_string(self).expect("attachment metadata serializes");
        format!(
            "{PREFIX}{}]",
            url::form_urlencoded::byte_serialize(json.as_bytes()).collect::<String>()
        )
    }

    pub fn description(&self) -> String {
        format!(
            "[Uploaded file: {}; {}; {} bytes; workspace path: {}]",
            self.name, self.mime, self.size_bytes, self.path
        )
    }

    /// The attachment `text` is the [`description`](Self::description) of,
    /// when it is exactly one. The name may hold anything, so the fixed
    /// fields are read from the right and the result must describe itself
    /// back byte for byte.
    pub(crate) fn from_description(text: &str) -> Option<Self> {
        let body = text.strip_prefix("[Uploaded file: ")?.strip_suffix(']')?;
        let (head, path) = body.rsplit_once("; workspace path: ")?;
        let (head, size) = head.strip_suffix(" bytes")?.rsplit_once("; ")?;
        let (name, mime) = head.rsplit_once("; ")?;
        let attachment = Self {
            path: path.into(),
            name: name.into(),
            mime: mime.into(),
            size_bytes: size.parse().ok()?,
        };
        (attachment.description() == text).then_some(attachment)
    }
}

pub(crate) enum Segment {
    Text(String),
    Attachment(Attachment),
}

/// Preserve captions and attachments in their source order.
pub(crate) fn segments(text: &str) -> Vec<Segment> {
    let mut rest = text;
    let mut parts = Vec::new();
    while let Some(start) = rest.find(PREFIX) {
        if start > 0 {
            parts.push(Segment::Text(rest[..start].into()));
        }
        let tail = &rest[start + PREFIX.len()..];
        let Some(end) = tail.find(']') else {
            parts.push(Segment::Text(rest[start..].into()));
            return parts;
        };
        let encoded = format!("value={}", &tail[..end]);
        let decoded = url::form_urlencoded::parse(encoded.as_bytes())
            .next()
            .map(|(_, value)| value.into_owned());
        match decoded.and_then(|json| serde_json::from_str::<Attachment>(&json).ok()) {
            Some(file) if safe_relative_path(&file.path) => parts.push(Segment::Attachment(file)),
            _ => parts.push(Segment::Text(
                rest[start..start + PREFIX.len() + end + 1].into(),
            )),
        }
        rest = &tail[end + 1..];
    }
    if !rest.is_empty() {
        parts.push(Segment::Text(rest.into()));
    }
    parts
}

/// Split compact references without interpreting ordinary prose as paths.
pub(crate) fn parse(text: &str) -> (String, Vec<Attachment>) {
    let mut clean = String::new();
    let mut files = Vec::new();
    for segment in segments(text) {
        match segment {
            Segment::Text(text) => clean.push_str(&text),
            Segment::Attachment(file) => files.push(file),
        }
    }
    (clean, files)
}

fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\0', '\\'])
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

pub(crate) fn action_root(config: &Config, scope: &AttachmentAccessScope) -> PathBuf {
    scope
        .workspace
        .clone()
        .unwrap_or_else(|| config.action_dir.clone())
}

/// The folder a thread acts in: its bound working folder, else `action_dir`.
/// Uploads must be written, validated and read under this one root; the turn
/// runs with `action_dir` replaced by the thread's folder (#7714).
pub(crate) async fn thread_action_dir(
    ctx: &DispatchCtx,
    config: &Config,
    threads_dir: &Path,
    thread_id: Option<&str>,
) -> anyhow::Result<PathBuf> {
    let Some(thread_id) = thread_id.filter(|id| !id.is_empty()) else {
        return Ok(config.action_dir.clone());
    };
    let bound = crate::threads::ops::thread_working_dir(ctx, threads_dir.to_path_buf(), thread_id)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(bound.unwrap_or_else(|| config.action_dir.clone()))
}

fn policy(config: &Config, root: &Path) -> SecurityPolicy {
    let policy = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, root);
    *policy
        .media_files_roots
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some(crate::artifacts::FileRoots::from_config(config));
    policy
}

/// Resolve a local attachment under the same filesystem policy as acting
/// tools, including symlink checks and the always-forbidden credential floor.
pub(crate) async fn resolve_path(
    config: &Config,
    reference: &str,
    scope: &AttachmentAccessScope,
) -> anyhow::Result<PathBuf> {
    let root = action_root(config, scope);
    let policy = policy(config, &root);
    if !policy.is_path_string_allowed(reference) {
        anyhow::bail!("attachment path is not permitted");
    }
    let path = if Path::new(reference).is_absolute() {
        PathBuf::from(reference)
    } else {
        root.join(reference)
    };
    policy
        .validate_path(&path.to_string_lossy())
        .await
        .map_err(anyhow::Error::msg)
}

pub(super) async fn is_missing_local_reference(
    config: &Config,
    reference: &str,
    scope: &AttachmentAccessScope,
) -> bool {
    let root = action_root(config, scope);
    let path_policy = policy(config, &root);
    if !path_policy.is_path_string_allowed(reference) {
        return false;
    }
    let requested = Path::new(reference);
    let path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    matches!(
        tokio::fs::symlink_metadata(path).await,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

fn filename(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or("attachment");
    let mut sanitized = String::new();
    for c in leaf.chars() {
        let c = if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
            c
        } else {
            '_'
        };
        if sanitized.len() + c.len_utf8() > 180 {
            break;
        }
        sanitized.push(c);
    }
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "attachment".into()
    } else {
        sanitized
    }
}

async fn save(
    config: &Config,
    thread: &str,
    name: &str,
    mime: &str,
    bytes: &[u8],
    scope: &AttachmentAccessScope,
) -> anyhow::Result<Attachment> {
    let root = action_root(config, scope);
    // Validate the complete prospective destination before creating even the
    // workspace root. This checks existing symlink ancestors as well as the
    // credential/internal-state floor.
    policy(config, &root)
        .validate_parent_path(&root.join("uploads").join("validation").to_string_lossy())
        .await
        .map_err(anyhow::Error::msg)?;
    tokio::fs::create_dir_all(&root).await?;
    let root = tokio::fs::canonicalize(root).await?;
    let thread = if !thread.is_empty()
        && thread.len() <= 128
        && thread
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        thread.to_string()
    } else {
        tinyagents_harness::multimodal::payload::sha256_prefix(thread.as_bytes())
    };
    let id = uuid::Uuid::new_v4().to_string();
    let relative = PathBuf::from("uploads")
        .join(thread)
        .join(id)
        .join(filename(name));
    let path = policy(config, &root)
        .validate_parent_path(&root.join(&relative).to_string_lossy())
        .await
        .map_err(anyhow::Error::msg)?;
    let mut parent = root.clone();
    for component in relative.parent().expect("upload parent").components() {
        parent.push(component);
        match tokio::fs::create_dir(&parent).await {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        // Existing upload directories must be real directories, never links.
        let metadata = tokio::fs::symlink_metadata(&parent).await?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            anyhow::bail!("upload directory is not a regular directory");
        }
        if !tokio::fs::canonicalize(&parent).await?.starts_with(&root) {
            anyhow::bail!("upload directory escapes workspace");
        }
    }
    let path = policy(config, &root)
        .validate_parent_path(&path.to_string_lossy())
        .await
        .map_err(anyhow::Error::msg)?;
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await?;
    if let Err(error) = async {
        file.write_all(bytes).await?;
        file.sync_all().await
    }
    .await
    {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(error.into());
    }
    Ok(Attachment {
        path: relative.to_string_lossy().replace('\\', "/"),
        name: if name.is_empty() {
            "attachment".into()
        } else {
            name.into()
        },
        mime: mime.into(),
        size_bytes: bytes.len(),
    })
}

/// Persist bytes returned by a successful scoped channel-module media read.
/// This bypasses marker URL/path resolution: only this internal caller can
/// provide the bytes, and the existing workspace/security destination checks
/// still apply.
pub(crate) async fn store_scoped_channel_media(
    config: &Config,
    thread: &str,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> anyhow::Result<Attachment> {
    const MODULE_MEDIA_MAX_BYTES: usize = 20 * 1024 * 1024;
    if bytes.is_empty() || bytes.len() > MODULE_MEDIA_MAX_BYTES {
        anyhow::bail!("scoped channel media exceeds its byte limit");
    }
    let scope = AttachmentAccessScope {
        external_channel: false,
        workspace: Some(config.action_dir.clone()),
        ..Default::default()
    };
    save(config, thread, name, mime, bytes, &scope).await
}

/// Stage every original before scanning or persistence. Failure rejects the
/// turn rather than silently discarding an accepted upload.
pub(crate) async fn stage(
    message: &str,
    thread: &str,
    config: &Config,
    scope: &AttachmentAccessScope,
) -> anyhow::Result<String> {
    let (text, images) = markers::parse_image_markers(message);
    let (_, files) = markers::parse_file_markers(&text);
    if scope.external_channel && (!images.is_empty() || !files.is_empty()) {
        anyhow::bail!("local attachment reads are disabled for external channel input");
    }
    let (max_images, image_mb) = config.multimodal.effective_limits();
    let (max_files, file_mb, max_text) = config.multimodal_files.effective_limits();
    if images.len() > max_images
        || files.len() > max_files
        || (config.multimodal_files.max_files == 0 && (!files.is_empty() || !images.is_empty()))
    {
        anyhow::bail!("attachment count exceeds configured limit");
    }
    let client = crate::config::build_runtime_proxy_client_with_timeouts("provider.ollama", 30, 10);
    let mut out = String::new();
    let mut rest = message;
    loop {
        let next = [("[IMAGE:", true), ("[FILE:", false)]
            .into_iter()
            .filter_map(|(prefix, image)| rest.find(prefix).map(|start| (start, prefix, image)))
            .min_by_key(|(start, _, _)| *start);
        let Some((start, prefix, image)) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let tail = &rest[start + prefix.len()..];
        let end = tail
            .find(']')
            .ok_or_else(|| anyhow::anyhow!("unterminated attachment marker"))?;
        let source = tail[..end].trim();
        if source.is_empty() {
            anyhow::bail!("empty attachment reference");
        }
        let max_bytes = if image { image_mb } else { file_mb } * 1024 * 1024;
        let remote = if image {
            config.multimodal.allow_remote_fetch
        } else {
            config.multimodal_files.allow_remote_fetch
        };
        let limits = FileLimits {
            max_files: max_files.max(1),
            max_file_size_mb: max_bytes / 1024 / 1024,
            max_extracted_text_chars: max_text,
            allow_remote_fetch: remote,
            allowed_mime_types: Vec::new(),
        };
        let source = if !source.starts_with("data:")
            && !source.starts_with("http://")
            && !source.starts_with("https://")
        {
            let local = legacy::migrate_path(config, source, scope)
                .await?
                .unwrap_or_else(|| source.into());
            resolve_path(config, &local, scope)
                .await?
                .to_string_lossy()
                .into_owned()
        } else {
            source.to_string()
        };
        let resolved = resolve_attachment(
            &source,
            &limits,
            max_bytes,
            &client,
            UnknownMimePolicy::Accept,
        )
        .await?;
        if image && !resolved.mime.starts_with("image/") {
            anyhow::bail!("image attachment does not contain a supported image type");
        }
        let attachment = save(
            config,
            thread,
            &resolved.name,
            &resolved.mime,
            &resolved.bytes,
            scope,
        )
        .await?;
        out.push_str(&attachment.marker());
        rest = &tail[end + 1..];
    }

    Ok(out)
}

/// Explicit image forwarding for delegation. A filename mentioned in prose
/// grants no read; only the typed argument or a legacy image marker does.
pub(crate) async fn delegation_prompt(
    ctx: &crate::core::runtime::DispatchCtx,
    prompt: &str,
    args: &serde_json::Value,
    workspace: Option<&tinytools::WorkspaceDescriptor>,
    origin: Option<&crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> anyhow::Result<String> {
    let mut prompt = prompt.to_string();
    if let Some(value) = args.get("image_paths") {
        let paths = value
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("image_paths must be an array of paths"))?;
        for path in paths {
            let path = path
                .as_str()
                .filter(|p| !p.trim().is_empty() && !p.contains([']', '\0']))
                .ok_or_else(|| anyhow::anyhow!("image_paths must contain nonempty paths"))?;
            prompt.push_str(&format!("\n[IMAGE:{path}]"));
        }
    }
    if !prompt.contains("[IMAGE:") && !prompt.contains("[FILE:") {
        return Ok(prompt);
    }
    if matches!(
        origin,
        Some(crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
    ) {
        anyhow::bail!("local attachment reads are disabled for external channel input");
    }
    let mut config = ctx.load_config().await.map_err(anyhow::Error::msg)?;
    if let Some(workspace) = workspace {
        config.action_dir = workspace.root.clone();
    }
    let scope = AttachmentAccessScope {
        external_channel: false,
        workspace: workspace.map(|workspace| workspace.root.clone()),
        ..Default::default()
    };
    stage(&prompt, "delegation", &config, &scope).await
}

/// Validate a vision task's explicit references before constructing inference.
pub(crate) async fn has_resolvable_image(
    ctx: &crate::core::runtime::DispatchCtx,
    prompt: &str,
    workspace: Option<&tinytools::WorkspaceDescriptor>,
    origin: Option<&crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> anyhow::Result<bool> {
    let rehydrated = crate::agent::multimodal::rehydrate_image_placeholders(&[
        tinyagents_session::transcript::TranscriptMessage::user(prompt),
    ])
    .await;
    let mut paths = parse(prompt)
        .1
        .into_iter()
        .filter(|file| file.mime.starts_with("image/"))
        .map(|file| file.path)
        .collect::<Vec<_>>();
    paths.extend(markers::parse_image_markers(&rehydrated[0].content).1);
    if paths.is_empty() {
        return Ok(false);
    }
    let mut config = ctx.load_config().await.map_err(anyhow::Error::msg)?;
    if matches!(
        origin,
        Some(crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
    ) {
        anyhow::bail!("local attachment reads are disabled for external channel input");
    }
    if let Some(workspace) = workspace {
        config.action_dir = workspace.root.clone();
    }
    let scope = AttachmentAccessScope {
        external_channel: false,
        workspace: workspace.map(|workspace| workspace.root.clone()),
        ..Default::default()
    };
    for path in paths {
        let path = legacy::migrate_path(&config, &path, &scope)
            .await?
            .unwrap_or(path);
        let path = resolve_path(&config, &path, &scope).await?;
        let metadata = tokio::fs::metadata(path).await?;
        if metadata.is_file() && metadata.len() > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Prefix contextual text without flattening typed media. Keep a leading text
/// block joined so plain turns preserve their live/replayed representation.
pub(crate) fn enrich_input(
    input: tinyinference_llm::message::Message,
    original_text: &str,
    enriched: &str,
) -> tinyinference_llm::message::Message {
    use tinyinference_llm::message::{ContentBlock, Message};
    let Message::User(mut user) = input else {
        return input;
    };
    let prefix = enriched.strip_suffix(original_text).unwrap_or(enriched);
    if !prefix.is_empty() {
        if let Some(ContentBlock::Text(text)) = user.content.first_mut() {
            text.insert_str(0, prefix);
        } else {
            user.content.insert(0, ContentBlock::Text(prefix.into()));
        }
    }
    Message::User(user)
}

/// Extract explicit typed images into the delegation carrier without flattening
/// the input. Old sidecar tokens remain supported.
pub(crate) fn image_references(
    input: &tinyinference_llm::message::Message,
    legacy: &[String],
) -> Vec<String> {
    use tinyinference_llm::message::{ContentBlock, Message};
    let mut images = legacy.to_vec();
    if let Message::User(user) = input {
        for block in &user.content {
            if let ContentBlock::Image(image) = block {
                let marker = format!("[IMAGE:{}]", image.url);
                if !images.contains(&marker) {
                    images.push(marker);
                }
            }
        }
    }
    images
}

/// An explicit image selection replaces automatic parent forwarding.
pub(crate) fn should_forward_parent_images(prompt: &str) -> bool {
    parse(prompt)
        .1
        .iter()
        .all(|file| !file.mime.starts_with("image/"))
        && markers::parse_image_markers(prompt).1.is_empty()
        && markers::extract_image_placeholders_in_text(prompt).is_empty()
}

/// Normalize a direct session input under its bound runtime and workspace.
pub(crate) async fn stage_turn(
    message: &str,
    config: Option<&Config>,
    workspace: Option<&tinytools::WorkspaceDescriptor>,
    thread: Option<&str>,
    origin: Option<&crate::core::runtime::turn_origin::AgentTurnOrigin>,
) -> anyhow::Result<String> {
    if !message.contains("[IMAGE:") && !message.contains("[FILE:") {
        return Ok(message.into());
    }
    let mut config = config
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("attachment intake requires runtime configuration"))?;
    if matches!(
        origin,
        Some(crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
    ) {
        config.multimodal_files =
            crate::config::MultimodalFileConfig::for_untrusted_channel_input();
    }
    if let Some(workspace) = workspace {
        config.action_dir = workspace.root.clone();
    }
    let scope = AttachmentAccessScope {
        external_channel: matches!(
            origin,
            Some(crate::core::runtime::turn_origin::AgentTurnOrigin::ExternalChannel { .. })
        ),
        workspace: workspace.map(|workspace| workspace.root.clone()),
        ..Default::default()
    };
    stage(message, thread.unwrap_or("direct"), &config, &scope).await
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "mod_access_tests.rs"]
mod access_tests;
