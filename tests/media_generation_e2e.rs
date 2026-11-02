//! End-to-end regression for media generation (the "anime cartoon" incident).
//!
//! Boots the real TinyHumans backend transport in-process, builds the
//! production `media_generate_image` / `media_generate_video` tools through
//! the registered media tool source's own code path
//! (`openhuman_tinyhumans::integrations::media::managed_generators` +
//! `media_tools_from`), and drives them against a scripted fake of the
//! backend's `/agent-integrations/openrouter` proxy:
//!
//! - responses arrive in the backend's `{success, data}` envelope;
//! - the video job reports `completed` with **no** `unsigned_urls` before the
//!   output exists — the exact shape that previously ended in "reported
//!   success but returned no media" while the generation was billed;
//! - the clip is downloaded through the authenticated content proxy.
//!
//! It asserts the files land in `generated-media/`, that every request carried
//! the backend credential and the product-identity header, and that the job
//! was polled through the empty `completed` state rather than failing on it.

#![cfg(feature = "media")]

#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{json, Value};
use tinyagents_harness::tinyinference_video::WaitPolicy;
use tinytools::Tool;

use openhuman_core::config::Config;
use openhuman_core::media::generation::media_tools_from;
use openhuman_tinyhumans::integrations::media::managed_generators;

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];
const MP4: &[u8] = b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom";
const API_KEY: &str = "th_live_media_e2e_0123456789abcdef";

#[derive(Clone, Default)]
struct Backend {
    /// `(method path, authorization, x-sdk-name present)` per request.
    requests: Arc<Mutex<Vec<(String, String, bool)>>>,
    image_bodies: Arc<Mutex<Vec<Value>>>,
    video_bodies: Arc<Mutex<Vec<Value>>>,
    polls: Arc<AtomicUsize>,
}

impl Backend {
    fn record(&self, route: &str, headers: &HeaderMap) {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let sdk_name = headers.contains_key("x-sdk-name");
        self.requests
            .lock()
            .unwrap()
            .push((route.to_owned(), auth, sdk_name));
    }
}

async fn start_backend() -> (String, Backend) {
    let backend = Backend::default();
    let prefix = "/agent-integrations/openrouter";
    let router = Router::new()
        .route(
            &format!("{prefix}/images"),
            post(|State(b): State<Backend>, headers: HeaderMap, Json(body): Json<Value>| async move {
                b.record("POST images", &headers);
                b.image_bodies.lock().unwrap().push(body);
                Json(json!({ "success": true, "data": {
                    "created": 1,
                    "data": [{ "b64_json": base64::engine::general_purpose::STANDARD.encode(PNG), "media_type": "image/png" }],
                    "usage": { "cost": 0.035 }
                }}))
            }),
        )
        .route(
            &format!("{prefix}/images/models"),
            get(|State(b): State<Backend>, headers: HeaderMap| async move {
                b.record("GET images/models", &headers);
                Json(json!({ "success": true, "data": { "object": "list", "data": [
                    { "id": "bytedance-seed/seedream-5-0-lite", "display_name": "Seedream 5.0 Lite" }
                ]}}))
            }),
        )
        .route(
            &format!("{prefix}/videos"),
            post(|State(b): State<Backend>, headers: HeaderMap, Json(body): Json<Value>| async move {
                b.record("POST videos", &headers);
                b.video_bodies.lock().unwrap().push(body);
                Json(json!({ "success": true, "data": {
                    "id": "gen-vid-1790000000-abcdefghijklmnopqrst",
                    "polling_url": "/api/v1/videos/gen-vid-1790000000-abcdefghijklmnopqrst",
                    "status": "pending"
                }}))
            }),
        )
        .route(
            &format!("{prefix}/videos/models"),
            get(|State(b): State<Backend>, headers: HeaderMap| async move {
                b.record("GET videos/models", &headers);
                Json(json!({ "success": true, "data": { "object": "list", "data": [] } }))
            }),
        )
        .route(
            &format!("{prefix}/videos/{{job}}"),
            get(
                |State(b): State<Backend>, headers: HeaderMap, AxumPath(job): AxumPath<String>| async move {
                    b.record("GET videos/:job", &headers);
                    let n = b.polls.fetch_add(1, Ordering::SeqCst);
                    // in_progress → completed WITHOUT outputs (the incident shape)
                    // → completed with the output.
                    let (status, urls) = match n {
                        0 => ("in_progress", vec![]),
                        1 | 2 => ("completed", vec![]),
                        _ => ("completed", vec!["https://cdn.example/out.mp4"]),
                    };
                    Json(json!({ "success": true, "data": {
                        "id": job, "status": status, "unsigned_urls": urls, "usage": { "cost": 0.38 }
                    }}))
                },
            ),
        )
        .route(
            &format!("{prefix}/videos/{{job}}/content"),
            get(|State(b): State<Backend>, headers: HeaderMap| async move {
                b.record("GET videos/:job/content", &headers);
                ([("content-type", "video/mp4")], MP4).into_response()
            }),
        )
        .with_state(backend.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.ok();
    });
    (format!("http://{address}"), backend)
}

async fn config(root: &Path, api_url: &str) -> Config {
    let mut config = Config {
        config_path: root.join("config.toml"),
        workspace_dir: root.join("workspace"),
        api_url: Some(api_url.to_owned()),
        ..Default::default()
    };
    config.secrets.encrypt = false;
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    openhuman_core::credentials::api_key::store_api_key(&config, API_KEY)
        .await
        .expect("store the TinyHumans API key");
    config
}

fn tool<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> &'a dyn Tool {
    tools
        .iter()
        .find(|t| t.name() == name)
        .map(AsRef::as_ref)
        .unwrap_or_else(|| panic!("missing {name}"))
}

#[tokio::test]
async fn media_tools_deliver_images_and_videos_through_the_backend_proxy() {
    tinyhumans_boot::boot();
    let tmp = tempfile::tempdir().unwrap();
    let (api_url, backend) = start_backend().await;
    let mut config = config(tmp.path(), &api_url).await;
    let action_dir = tmp.path().join("projects");
    // Deliverables land in the visible Files folder (#5505); metadata is read
    // through the artifact store because it may use the configured backend.
    let files_dir = tmp.path().join("Files");
    config.files_dir_override = Some(files_dir.clone());

    // The source-backed product catalog is assembled after TinyHumans installs
    // its media registration. Keep these hosted tools in the core catalog as
    // well as exercising their generators below.
    let security = Arc::new(openhuman_core::security::SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    let registered = openhuman_core::tools::ops::all_tools(
        Arc::new(config.clone()),
        &security,
        openhuman_core::security::AuditSink::disabled(),
        &config.browser,
        &config.http_request,
        &action_dir,
        &config,
    )
    .await;
    for expected in [
        "media_generate_image",
        "media_generate_video",
        "media_list_models",
    ] {
        assert!(
            registered
                .iter()
                .any(|candidate| candidate.name() == expected),
            "the host-registered media source must expose `{expected}`"
        );
    }

    let generators = managed_generators(&config).expect("backend transport is installed");
    let tools = media_tools_from(
        generators,
        &action_dir,
        &config.workspace_dir,
        &files_dir,
        WaitPolicy::new(Duration::from_millis(5), Duration::from_secs(20)),
    );

    // ── image ────────────────────────────────────────────────────────────
    let result = tool(&tools, "media_generate_image")
        .execute(json!({
            "prompt": "a four-panel anime comic explaining a delivery certificate",
            "aspect_ratio": "landscape",
            "seed": 42
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "image tool failed: {result:?}");
    let image_artifacts = result
        .content
        .iter()
        .find_map(|block| match block {
            tinytools::ToolContent::Json { data } => data["artifacts"].as_array(),
            _ => None,
        })
        .expect("image result artifacts");
    assert_eq!(image_artifacts.len(), 1, "one filed image: {result:?}");
    let image_id = image_artifacts[0]["artifact_id"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "image was not filed as an artifact: {:?}",
                image_artifacts[0]
            )
        })
        .to_owned();

    // ── video: must poll through `completed` with no outputs ─────────────
    let result = tool(&tools, "media_generate_video")
        .execute(json!({
            "prompt": "two engineers shake hands, anime style",
            "duration": 4,
            "resolution": "480",
            "first_frame": "https://example.com/first.png"
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "video tool failed: {result:?}");
    let video_artifacts = result
        .content
        .iter()
        .find_map(|block| match block {
            tinytools::ToolContent::Json { data } => data["artifacts"].as_array(),
            _ => None,
        })
        .expect("video result artifacts");
    assert_eq!(video_artifacts.len(), 1, "one filed video: {result:?}");
    let video_id = video_artifacts[0]["artifact_id"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "video was not filed as an artifact: {:?}",
                video_artifacts[0]
            )
        })
        .to_owned();
    assert!(
        backend.polls.load(Ordering::SeqCst) >= 4,
        "the job must be polled through the empty `completed` states"
    );

    // ── artifacts are persisted and their visible files resolve ──────────
    assert!(
        !action_dir.join("generated-media").exists(),
        "blob-backed generation must not leave a filesystem staging directory"
    );
    for (id, kind) in [(&image_id, "image"), (&video_id, "video")] {
        let record = openhuman_core::artifacts::ops::ai_get_artifact(&config, id)
            .await
            .expect("filed artifact record")
            .value;
        assert_eq!(record["kind"], kind);
        assert_eq!(record["status"], "ready");
        let path = std::path::Path::new(
            record["absolute_path"]
                .as_str()
                .expect("resolved artifact path"),
        );
        assert!(path.starts_with(&files_dir), "{path:?}");
        assert!(path.is_file(), "{path:?}");
    }
    let mut saved: Vec<(String, Vec<u8>)> = std::fs::read_dir(&files_dir)
        .expect("visible files folder")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .map(|path| {
            (
                path.extension().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&path).unwrap(),
            )
        })
        .collect();
    saved.sort();
    assert_eq!(saved.len(), 2, "one image and one video: {saved:?}");
    assert_eq!(saved[0].0, "mp4");
    assert_eq!(saved[0].1, MP4);
    assert_eq!(saved[1].0, "png");
    assert_eq!(saved[1].1, PNG);

    // ── wire contract ────────────────────────────────────────────────────
    let image_body = backend.image_bodies.lock().unwrap()[0].clone();
    assert_eq!(image_body["model"], "bytedance-seed/seedream-5-0-lite");
    assert_eq!(image_body["aspect_ratio"], "16:9");
    assert_eq!(image_body["seed"], 42);
    let video_body = backend.video_bodies.lock().unwrap()[0].clone();
    assert_eq!(video_body["model"], "bytedance/seedance-2.0-mini");
    assert_eq!(video_body["resolution"], "480p");
    assert_eq!(video_body["duration"], 4);
    assert_eq!(video_body["frame_images"][0]["frame_type"], "first_frame");

    // ── every request authenticated and attributed ───────────────────────
    let requests = backend.requests.lock().unwrap().clone();
    assert!(!requests.is_empty());
    for (route, auth, sdk_name) in &requests {
        assert_eq!(auth, &format!("Bearer {API_KEY}"), "{route} credential");
        assert!(*sdk_name, "{route} is missing x-sdk-name");
    }
    let submits = requests
        .iter()
        .filter(|(r, _, _)| r.starts_with("POST"))
        .count();
    assert_eq!(
        submits, 2,
        "exactly one billed submit per tool call: {requests:?}"
    );
}
