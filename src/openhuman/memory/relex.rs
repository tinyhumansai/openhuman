//! Relation Extraction (RelEx) using GLiNER on ONNX for the OpenHuman memory system.
//!
//! This module implements entity and relation extraction from unstructured text.
//! It uses a specialized GLiNER model (General Language Model for Information 
//! Extraction) exported to ONNX format. The extraction process identifies:
//! 1. **Entities**: Named people, organizations, projects, etc.
//! 2. **Relations**: Semantic links between entities (e.g., "Person A works on Project B").
//!
//! The module handles model asset management (downloading, checksumming),
//! ONNX Runtime initialization, and the complex tensor-based inference logic.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use futures_util::TryStreamExt;
#[cfg(target_os = "windows")]
use glob::glob;
use ndarray::{Array, Array2, Array3, Array4, Ix2, Ix3, Ix4};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use parking_lot::Mutex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;

use crate::openhuman::memory::DEFAULT_GLINER_RELEX_MODEL;

// Constants for model paths and asset management.
const DEFAULT_EXPORTED_RELEX_DIR: &str =
    "_tmp/gliner-export/artifacts/gliner-relex-large-v0.5-onnx";
const DEFAULT_MANAGED_RELEX_DIR: &str = ".openhuman/models/gliner-relex-large-v0.5-onnx";
const DEFAULT_RELEX_RELEASE_BASE_URL: &str =
    "https://github.com/sanil-23/GLiNER/releases/download/tinyhumans-gliner-relex-v0.5-onnx.1";
const MODEL_FILE_NAME: &str = "model_quantized.onnx";
const FALLBACK_MODEL_FILE_NAME: &str = "model.onnx";
const TOKENIZER_FILE_NAME: &str = "tokenizer.json";
const TOKENIZER_CONFIG_FILE_NAME: &str = "tokenizer_config.json";
const GLINER_CONFIG_FILE_NAME: &str = "gliner_config.json";

#[cfg(target_os = "windows")]
const ORT_DYLIB_FILE_NAME: &str = "onnxruntime.dll";
#[cfg(target_os = "macos")]
const ORT_DYLIB_FILE_NAME: &str = "libonnxruntime.dylib";
#[cfg(target_os = "linux")]
const ORT_DYLIB_FILE_NAME: &str = "libonnxruntime.so";
#[cfg(target_os = "linux")]
const ORT_SHARED_PROVIDER_FILE_NAME: &str = "libonnxruntime_providers_shared.so";

/// Metadata for a single model asset used during download and verification.
struct BundleAsset {
    remote_name: &'static str,
    local_name: &'static str,
    sha256: &'static str,
}

/// Core assets required by all platforms.
const CORE_BUNDLE_ASSETS: &[BundleAsset] = &[
    BundleAsset {
        remote_name: MODEL_FILE_NAME,
        local_name: MODEL_FILE_NAME,
        sha256: "7D4B8D35750D0AEC35DA0EB1EDFE33076C6958B8CD6EEC4560C59822536C9AEF",
    },
    BundleAsset {
        remote_name: TOKENIZER_FILE_NAME,
        local_name: TOKENIZER_FILE_NAME,
        sha256: "0FD23B86F1BACEE52F4485FCD4441B923132302BED55BC5E081172CA013E7654",
    },
    BundleAsset {
        remote_name: TOKENIZER_CONFIG_FILE_NAME,
        local_name: TOKENIZER_CONFIG_FILE_NAME,
        sha256: "3157274603C17459B0589DBB6818A47714D780718A6D0EB505C10347C466F2CD",
    },
    BundleAsset {
        remote_name: GLINER_CONFIG_FILE_NAME,
        local_name: GLINER_CONFIG_FILE_NAME,
        sha256: "FF6D7FEFD65F721515A3822BB074F2A36EC9B66AC75DAA400E2465FFE52F02BA",
    },
];

/// Platform-specific dynamic libraries for ONNX Runtime.
#[cfg(target_os = "windows")]
const PLATFORM_BUNDLE_ASSETS: &[BundleAsset] = &[BundleAsset {
    remote_name: ORT_DYLIB_FILE_NAME,
    local_name: ORT_DYLIB_FILE_NAME,
    sha256: "EF720FC44A4EA48626BFE1EBD29642DE20222D7F104A509EA305D9F3CB3B7850",
}];
#[cfg(target_os = "macos")]
const PLATFORM_BUNDLE_ASSETS: &[BundleAsset] = &[BundleAsset {
    remote_name: ORT_DYLIB_FILE_NAME,
    local_name: ORT_DYLIB_FILE_NAME,
    sha256: "285C8CD1E53856507B9B2E38EE9AFFC69AA6E90AC30F8670DC8195710CA14B77",
}];
#[cfg(target_os = "linux")]
const PLATFORM_BUNDLE_ASSETS: &[BundleAsset] = &[
    BundleAsset {
        remote_name: ORT_DYLIB_FILE_NAME,
        local_name: ORT_DYLIB_FILE_NAME,
        sha256: "13AB8084954FA4A47C777880180B90810D6020F021441395712B48A75B74C68B",
    },
    BundleAsset {
        remote_name: ORT_SHARED_PROVIDER_FILE_NAME,
        local_name: ORT_SHARED_PROVIDER_FILE_NAME,
        sha256: "086EC1D5388F64153D9C63470D126693DB9A182C8CE236D3A1119068471B8A0D",
    },
];
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
const PLATFORM_BUNDLE_ASSETS: &[BundleAsset] = &[];

/// Predefined entity types the model is trained to recognize.
const ENTITY_LABELS: &[&str] = &[
    "person",
    "organization",
    "project",
    "product",
    "tool",
    "topic",
    "work item",
    "mode",
    "place",
    "room",
    "date",
];

/// Predefined relation types the model is trained to recognize.
const RELATION_LABELS: &[&str] = &[
    "owns",
    "uses",
    "works on",
    "responsible for",
    "reviews",
    "works for",
    "depends on",
    "prefers",
    "has deadline",
    "communicates with",
    "investigates",
    "evaluates",
    "north of",
    "south of",
    "east of",
    "west of",
    "avoids",
];

/// An entity identified by the RelEx system.
#[derive(Debug, Clone)]
pub(crate) struct RelexEntity {
    pub name: String,
    pub entity_type: String,
    pub confidence: f32,
}

/// A relationship identified between two entities.
#[derive(Debug, Clone)]
pub(crate) struct RelexRelation {
    pub subject: String,
    pub subject_type: String,
    pub predicate: String,
    pub object: String,
    pub object_type: String,
    pub confidence: f32,
}

/// The result of a RelEx extraction pass.
#[derive(Debug, Clone, Default)]
pub(crate) struct RelexExtraction {
    pub entities: Vec<RelexEntity>,
    pub relations: Vec<RelexRelation>,
}

/// The initialized Relex engine, holding the tokenizer and the ONNX session.
#[derive(Debug)]
pub(crate) struct RelexRuntime {
    tokenizer: Tokenizer,
    session: Mutex<Session>,
    config: RelexBundleConfig,
}

/// Configuration parameters loaded from the model bundle.
#[derive(Debug, Clone, Deserialize)]
struct RelexBundleConfig {
    #[serde(default = "default_ent_token")]
    ent_token: String,
    #[serde(default = "default_rel_token")]
    rel_token: String,
    #[serde(default = "default_sep_token")]
    sep_token: String,
    #[serde(default = "default_max_width")]
    max_width: usize,
}

/// A batch of input data ready for ONNX inference.
#[derive(Debug, Clone)]
struct PromptBatch {
    input_ids: Array2<i64>,
    attention_mask: Array2<i64>,
    words_mask: Array2<i64>,
    text_lengths: Array2<i64>,
    num_words: usize,
}

/// A slice of a string representing a single word or token.
#[derive(Debug, Clone)]
struct TokenSlice {
    start: usize,
    end: usize,
    text: String,
}

/// A decoded entity span with its associated class and probability.
#[derive(Debug, Clone)]
struct DecodedSpan {
    start: usize,
    end: usize,
    text: String,
    class_name: String,
    probability: f32,
}

fn default_ent_token() -> String { "<<ENT>>".to_string() }
fn default_rel_token() -> String { "<<REL>>".to_string() }
fn default_sep_token() -> String { "<<SEP>>".to_string() }
fn default_max_width() -> usize { 12 }

/// Obtains a singleton or cached instance of the RelexRuntime.
pub(crate) async fn runtime(model_name: &str) -> Option<Arc<RelexRuntime>> {
    if !uses_default_bundle(model_name) {
        return load_runtime_for_model(model_name).await.ok();
    }

    static DEFAULT_RUNTIME: OnceLock<Mutex<Option<Arc<RelexRuntime>>>> = OnceLock::new();
    static DEFAULT_RUNTIME_BOOTSTRAP: OnceLock<AsyncMutex<()>> = OnceLock::new();

    let runtime_cell = DEFAULT_RUNTIME.get_or_init(|| Mutex::new(None));
    if let Some(runtime) = runtime_cell.lock().clone() {
        return Some(runtime);
    }

    let _guard = DEFAULT_RUNTIME_BOOTSTRAP
        .get_or_init(|| AsyncMutex::new(()))
        .lock()
        .await;

    if let Some(runtime) = runtime_cell.lock().clone() {
        return Some(runtime);
    }

    let runtime = load_default_runtime().await.ok().map(Arc::new)?;
    *runtime_cell.lock() = Some(runtime.clone());
    Some(runtime)
}

/// Pre-warms the default Relex bundle to ensure it's downloaded and ready.
pub(crate) async fn warm_default_bundle() -> Option<Arc<RelexRuntime>> {
    runtime(DEFAULT_GLINER_RELEX_MODEL).await
}

impl RelexRuntime {
    /// Extracts entities and relations from the given text.
    ///
    /// This method performs the following steps:
    /// 1. Tokenizes the input text into whitespace-delimited words.
    /// 2. Encodes the text along with the entity and relation labels into a model-specific prompt.
    /// 3. Constructs span and relation tensors for the model.
    /// 4. Runs the ONNX inference session.
    /// 5. Decodes the output logits into entities (using greedy filtering for overlapping spans).
    /// 6. Decodes relation logits into semantic links between the identified entities.
    pub(crate) fn extract(
        &self,
        text: &str,
        entity_threshold: f32,
        relation_threshold: f32,
    ) -> Result<RelexExtraction> {
        let tokens = split_whitespace_tokens(text);
        if tokens.is_empty() {
            return Ok(RelexExtraction::default());
        }

        // Prepare the model input (PromptBatch).
        let prompt = encode_prompt(
            &self.tokenizer,
            &self.config,
            &tokens,
            ENTITY_LABELS,
            RELATION_LABELS,
        )?;
        
        // Create span tensors. GLiNER uses these to evaluate every possible word span (up to max_width).
        let (span_idx, span_mask) = make_spans_tensors(prompt.num_words, self.config.max_width);

        let inputs = ort::inputs! {
            "input_ids" => Tensor::from_array(prompt.input_ids.clone())?,
            "attention_mask" => Tensor::from_array(prompt.attention_mask.clone())?,
            "words_mask" => Tensor::from_array(prompt.words_mask.clone())?,
            "text_lengths" => Tensor::from_array(prompt.text_lengths.clone())?,
            "span_idx" => Tensor::from_array(span_idx.clone())?,
            "span_mask" => Tensor::from_array(span_mask.clone())?,
        };

        // Execute inference.
        let mut session = self.session.lock();
        let outputs = session.run(inputs)?;
        
        // Extract 4D logits for entity spans.
        let logits = extract_f32_4d(outputs.get("logits").context("missing logits output")?)?;

        // Decode entities.
        let spans = decode_entity_spans(
            &logits,
            text,
            &tokens,
            ENTITY_LABELS,
            self.config.max_width,
            entity_threshold,
        );
        let entities = spans
            .iter()
            .map(|span| RelexEntity {
                name: span.text.clone(),
                entity_type: normalize_entity_label(&span.class_name).to_string(),
                confidence: span.probability,
            })
            .collect::<Vec<_>>();

        // Decode relations.
        let mut relations = Vec::new();
        let rel_idx = outputs.get("rel_idx").map(extract_i64_3d).transpose()?;
        let rel_logits = outputs.get("rel_logits").map(extract_f32_3d).transpose()?;
        let rel_mask = outputs.get("rel_mask").map(extract_bool_2d).transpose()?;

        if let (Some(rel_idx), Some(rel_logits), Some(rel_mask)) = (rel_idx, rel_logits, rel_mask) {
            // Index into the first (and only) batch.
            let rel_pairs = rel_idx.index_axis(ndarray::Axis(0), 0);
            let rel_scores = rel_logits.index_axis(ndarray::Axis(0), 0);
            let rel_valid = rel_mask.index_axis(ndarray::Axis(0), 0);

            for pair_idx in 0..rel_valid.shape()[0] {
                if !rel_valid[[pair_idx]] {
                    continue;
                }
                let head_idx = rel_pairs[[pair_idx, 0]];
                let tail_idx = rel_pairs[[pair_idx, 1]];
                if head_idx < 0 || tail_idx < 0 {
                    continue;
                }

                let head_idx = head_idx as usize;
                let tail_idx = tail_idx as usize;
                if head_idx >= spans.len() || tail_idx >= spans.len() {
                    continue;
                }

                let head = &spans[head_idx];
                let tail = &spans[tail_idx];
                let class_count = rel_scores.shape()[1].min(RELATION_LABELS.len());
                for class_idx in 0..class_count {
                    // Apply sigmoid to convert logit to probability.
                    let probability = sigmoid(rel_scores[[pair_idx, class_idx]]);
                    if probability < relation_threshold {
                        continue;
                    }
                    relations.push(RelexRelation {
                        subject: head.text.clone(),
                        subject_type: normalize_entity_label(&head.class_name).to_string(),
                        predicate: normalize_relation_label(RELATION_LABELS[class_idx]),
                        object: tail.text.clone(),
                        object_type: normalize_entity_label(&tail.class_name).to_string(),
                        confidence: probability,
                    });
                }
            }
        }

        Ok(RelexExtraction {
            entities,
            relations,
        })
    }
}

/// Checks if a model name refers to the default GLiNER bundle.
fn uses_default_bundle(model_name: &str) -> bool {
    model_name.trim().is_empty()
        || model_name == DEFAULT_GLINER_RELEX_MODEL
        || model_name == default_bundle_dir().to_string_lossy()
        || model_name == default_managed_bundle_dir().to_string_lossy()
}

/// Loads the default RelEx runtime, resolving and potentially downloading the bundle.
async fn load_default_runtime() -> Result<RelexRuntime> {
    let bundle_dir = resolve_bundle_dir(DEFAULT_GLINER_RELEX_MODEL)
        .await
        .ok_or_else(|| anyhow!("relex bundle directory not found"))?;
    load_runtime_from_bundle_dir(&bundle_dir)
}

/// Loads a runtime for a specifically named model/bundle.
async fn load_runtime_for_model(model_name: &str) -> Result<Arc<RelexRuntime>> {
    let bundle_dir = resolve_bundle_dir(model_name)
        .await
        .ok_or_else(|| anyhow!("relex bundle directory not found"))?;
    load_runtime_from_bundle_dir(&bundle_dir).map(Arc::new)
}

/// Initializes the ONNX session and tokenizer from a local bundle directory.
fn load_runtime_from_bundle_dir(bundle_dir: &Path) -> Result<RelexRuntime> {
    ensure_ort_dylib_path(bundle_dir);

    let tokenizer_path = bundle_dir.join(TOKENIZER_FILE_NAME);
    let model_path = model_file_path(bundle_dir)
        .ok_or_else(|| anyhow!("model file not found in {}", bundle_dir.display()))?;
    let config_path = bundle_dir.join(GLINER_CONFIG_FILE_NAME);

    let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|err| {
        anyhow!(
            "failed to load tokenizer from {}: {err}",
            tokenizer_path.display()
        )
    })?;
    let config = serde_json::from_slice::<RelexBundleConfig>(
        &std::fs::read(&config_path)
            .with_context(|| format!("failed to read {}", config_path.display()))?,
    )
    .with_context(|| format!("failed to parse {}", config_path.display()))?;

    let session = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .commit_from_file(&model_path)
        .with_context(|| format!("failed to load model {}", model_path.display()))?;

    Ok(RelexRuntime {
        tokenizer,
        session: Mutex::new(session),
        config,
    })
}

/// Finds the local directory containing the model bundle, potentially downloading it.
async fn resolve_bundle_dir(model_name: &str) -> Option<PathBuf> {
    // 1. Check environment variable override.
    if let Ok(path) = env::var("OPENHUMAN_GLINER_RELEX_DIR") {
        let bundle_dir = PathBuf::from(path);
        if bundle_complete(&bundle_dir) {
            return Some(bundle_dir);
        }
    }

    // 2. Check if model_name is an absolute path or direct path.
    let requested = PathBuf::from(model_name);
    if requested.is_absolute() || model_name.contains('/') || model_name.contains('\\') {
        if requested.is_dir() && bundle_complete(&requested) {
            return Some(requested);
        }
        if requested.is_file()
            && requested
                .file_name()
                .is_some_and(|name| name == FALLBACK_MODEL_FILE_NAME || name == MODEL_FILE_NAME)
        {
            return requested.parent().map(Path::to_path_buf);
        }
    }

    // 3. Check managed bundle in user home directory.
    let managed_dir = default_managed_bundle_dir();
    if managed_bundle_complete(&managed_dir) {
        return Some(managed_dir);
    }

    // 4. Check bundled artifacts in the project directory.
    let bundle_dir = default_bundle_dir();
    if bundle_complete(&bundle_dir) {
        return Some(bundle_dir);
    }

    // 5. Download the default bundle if necessary.
    if uses_default_bundle(model_name)
        && ensure_managed_bundle(&managed_dir).await.is_ok()
        && bundle_complete(&managed_dir)
    {
        return Some(managed_dir);
    }

    None
}

/// Returns the path to the artifacts directory within the project source.
fn default_bundle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .join(DEFAULT_EXPORTED_RELEX_DIR)
}

/// Returns the path to the managed model directory in the user's home or cache dir.
fn default_managed_bundle_dir() -> PathBuf {
    if let Ok(path) = env::var("OPENHUMAN_GLINER_RELEX_CACHE_DIR") {
        return PathBuf::from(path);
    }

    directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().join(DEFAULT_MANAGED_RELEX_DIR))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MANAGED_RELEX_DIR))
}

/// Checks if a bundle directory contains all core required files.
fn bundle_complete(bundle_dir: &Path) -> bool {
    bundle_dir.join(TOKENIZER_FILE_NAME).exists()
        && bundle_dir.join(GLINER_CONFIG_FILE_NAME).exists()
        && model_file_path(bundle_dir).is_some()
}

/// Checks if a managed bundle contains core files AND platform-specific libraries.
fn managed_bundle_complete(bundle_dir: &Path) -> bool {
    bundle_complete(bundle_dir)
        && PLATFORM_BUNDLE_ASSETS
            .iter()
            .all(|asset| bundle_dir.join(asset.local_name).exists())
}

/// Returns the path to the ONNX model file within a bundle, checking for quantized first.
fn model_file_path(bundle_dir: &Path) -> Option<PathBuf> {
    for file_name in [MODEL_FILE_NAME, FALLBACK_MODEL_FILE_NAME] {
        let candidate = bundle_dir.join(file_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Configures the ONNX runtime library path for the current process.
#[allow(unused_variables)]
fn ensure_ort_dylib_path(bundle_dir: &Path) {
    if env::var_os("ORT_DYLIB_PATH").is_some() {
        return;
    }

    // Check the bundle directory first.
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let bundled = bundle_dir.join(ORT_DYLIB_FILE_NAME);
        if bundled.exists() {
            env::set_var("ORT_DYLIB_PATH", bundled);
            return;
        }
    }

    // Check ORT_LIB_LOCATION environment variable.
    if let Some(lib_path) = env::var_os("ORT_LIB_LOCATION") {
        let candidate = PathBuf::from(lib_path);
        if candidate.is_file() {
            env::set_var("ORT_DYLIB_PATH", candidate);
            return;
        }
        let runtime_lib = candidate.join(ORT_DYLIB_FILE_NAME);
        if runtime_lib.exists() {
            env::set_var("ORT_DYLIB_PATH", runtime_lib);
        }
    }

    // Platform-specific fallback (Windows uv cache).
    #[cfg(target_os = "windows")]
    {
        let Some(user_profile) = env::var_os("USERPROFILE") else {
            return;
        };
        let pattern = PathBuf::from(user_profile)
            .join("AppData/Local/uv/cache/archive-v0/*/onnxruntime/capi/onnxruntime.dll")
            .to_string_lossy()
            .replace('\\', "/");
        if let Ok(paths) = glob(&pattern) {
            for candidate in paths.flatten() {
                if candidate.exists() {
                    env::set_var("ORT_DYLIB_PATH", &candidate);
                    break;
                }
            }
        }
    }

    // Platform-specific fallback (Linux system paths).
    #[cfg(target_os = "linux")]
    {
        for candidate in [
            "/usr/lib/x86_64-linux-gnu/libonnxruntime.so",
            "/usr/local/lib/libonnxruntime.so",
            "/usr/lib/libonnxruntime.so",
        ] {
            let candidate = PathBuf::from(candidate);
            if candidate.exists() {
                env::set_var("ORT_DYLIB_PATH", &candidate);
                return;
            }
        }
    }
}

/// Downloads the default RelEx bundle and verifies its contents.
async fn ensure_managed_bundle(bundle_dir: &Path) -> Result<()> {
    static MANAGED_BUNDLE_BOOTSTRAP: OnceLock<AsyncMutex<()>> = OnceLock::new();
    let _guard = MANAGED_BUNDLE_BOOTSTRAP
        .get_or_init(|| AsyncMutex::new(()))
        .lock()
        .await;

    if managed_bundle_complete(bundle_dir) {
        return Ok(());
    }

    tokio::fs::create_dir_all(bundle_dir)
        .await
        .with_context(|| format!("failed to create {}", bundle_dir.display()))?;

    let client = crate::openhuman::config::build_runtime_proxy_client("memory.relex");
    let base_url = env::var("OPENHUMAN_GLINER_RELEX_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_RELEX_RELEASE_BASE_URL.to_string());

    for asset in CORE_BUNDLE_ASSETS
        .iter()
        .chain(PLATFORM_BUNDLE_ASSETS.iter())
    {
        let target = bundle_dir.join(asset.local_name);
        download_asset_if_needed(&client, &base_url, asset, &target).await?;
    }

    Ok(())
}

/// Downloads a single asset if it's missing or if the checksum doesn't match.
async fn download_asset_if_needed(
    client: &reqwest::Client,
    base_url: &str,
    asset: &BundleAsset,
    target: &Path,
) -> Result<()> {
    if file_matches_sha256(target, asset.sha256).await? {
        return Ok(());
    }

    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        asset.remote_name.trim_start_matches('/')
    );
    let tmp = target.with_extension("download");
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to start relex asset download {url}"))?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "failed to download relex asset {}, status {}",
            asset.remote_name,
            response.status()
        ));
    }

    let mut file = tokio::fs::File::create(&tmp)
        .await
        .with_context(|| format!("failed to create {}", tmp.display()))?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream
        .try_next()
        .await
        .with_context(|| format!("download stream error for {}", asset.remote_name))?
    {
        file.write_all(&chunk)
            .await
            .with_context(|| format!("failed writing {}", tmp.display()))?;
    }
    file.flush()
        .await
        .with_context(|| format!("failed flushing {}", tmp.display()))?;

    if !asset.sha256.is_empty() && !file_matches_sha256(&tmp, asset.sha256).await? {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(anyhow!(
            "checksum mismatch for downloaded relex asset {}",
            asset.remote_name
        ));
    }

    tokio::fs::rename(&tmp, target)
        .await
        .with_context(|| format!("failed to finalize {}", target.display()))?;
    Ok(())
}

/// Verifies that a file's SHA256 hash matches the expected value.
async fn file_matches_sha256(path: &Path, expected: &str) -> Result<bool> {
    if expected.is_empty() {
        return Ok(path.exists());
    }
    if !path.exists() {
        return Ok(false);
    }
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    let actual = hex::encode(Sha256::digest(bytes));
    Ok(actual.eq_ignore_ascii_case(expected))
}

/// Simple whitespace-based tokenizer that preserves character offsets.
fn split_whitespace_tokens(text: &str) -> Vec<TokenSlice> {
    let mut tokens = Vec::new();
    let mut current_start: Option<usize> = None;

    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = current_start.take() {
                tokens.push(TokenSlice {
                    start,
                    end: idx,
                    text: text[start..idx].to_string(),
                });
            }
        } else if current_start.is_none() {
            current_start = Some(idx);
        }
    }

    if let Some(start) = current_start {
        tokens.push(TokenSlice {
            start,
            end: text.len(),
            text: text[start..].to_string(),
        });
    }

    tokens
}

/// Encodes the input text and labels into the format expected by the GLiNER model.
fn encode_prompt(
    tokenizer: &Tokenizer,
    config: &RelexBundleConfig,
    tokens: &[TokenSlice],
    entity_labels: &[&str],
    relation_labels: &[&str],
) -> Result<PromptBatch> {
    // 1. Build the prompt prefix containing all possible entity and relation types.
    let mut prompt_words = Vec::new();
    for label in entity_labels {
        prompt_words.push(config.ent_token.clone());
        prompt_words.push((*label).to_string());
    }
    prompt_words.push(config.sep_token.clone());
    for label in relation_labels {
        prompt_words.push(config.rel_token.clone());
        prompt_words.push((*label).to_string());
    }
    prompt_words.push(config.sep_token.clone());

    // 2. Combine labels with the actual document tokens.
    let mut words = prompt_words.clone();
    words.extend(tokens.iter().map(|token| token.text.clone()));

    // 3. Tokenize each word into subtokens (Byte-Pair Encoding).
    let mut encoded_words = Vec::with_capacity(words.len());
    let mut total_tokens = 2usize; // [CLS] and [SEP]
    let mut prompt_subtokens = 0usize;

    for (index, word) in words.iter().enumerate() {
        let encoding = tokenizer
            .encode(word.as_str(), false)
            .map_err(|err| anyhow!("failed to tokenize prompt word `{word}`: {err}"))?;
        let ids = encoding.get_ids().to_vec();
        if index < prompt_words.len() {
            prompt_subtokens += ids.len();
        }
        total_tokens += ids.len();
        encoded_words.push(ids);
    }

    // 4. Construct the input tensors (input_ids, attention_mask, words_mask).
    let text_offset = prompt_subtokens + 1;
    let mut input_ids = vec![0_i64; total_tokens];
    let mut attention_mask = vec![0_i64; total_tokens];
    let mut words_mask = vec![0_i64; total_tokens];

    let mut cursor = 0usize;
    input_ids[cursor] = 1; // [CLS]
    attention_mask[cursor] = 1;
    cursor += 1;

    let mut word_id = 0_i64;
    for ids in encoded_words {
        for (token_index, token_id) in ids.iter().enumerate() {
            input_ids[cursor] = i64::from(*token_id);
            attention_mask[cursor] = 1;
            // words_mask maps subtokens back to their original word index.
            if cursor >= text_offset && token_index == 0 {
                words_mask[cursor] = word_id;
            }
            cursor += 1;
        }
        if cursor >= text_offset {
            word_id += 1;
        }
    }

    input_ids[cursor] = 2; // [SEP]
    attention_mask[cursor] = 1;

    Ok(PromptBatch {
        input_ids: Array2::from_shape_vec((1, total_tokens), input_ids)?,
        attention_mask: Array2::from_shape_vec((1, total_tokens), attention_mask)?,
        words_mask: Array2::from_shape_vec((1, total_tokens), words_mask)?,
        text_lengths: Array2::from_shape_vec((1, 1), vec![tokens.len() as i64])?,
        num_words: tokens.len(),
    })
}

/// Generates span index and mask tensors for the model.
fn make_spans_tensors(num_words: usize, max_width: usize) -> (Array3<i64>, Array2<bool>) {
    let num_spans = num_words * max_width;
    let mut span_idx = Array3::<i64>::zeros((1, num_spans, 2));
    let mut span_mask = Array2::<bool>::from_elem((1, num_spans), false);

    for start in 0..num_words {
        let actual_max_width = max_width.min(num_words.saturating_sub(start));
        for width in 0..actual_max_width {
            let dim = start * max_width + width;
            span_idx[[0, dim, 0]] = start as i64;
            span_idx[[0, dim, 1]] = (start + width) as i64;
            span_mask[[0, dim]] = true;
        }
    }

    (span_idx, span_mask)
}

/// Decodes the raw 4D logits from the model into human-readable entity spans.
fn decode_entity_spans(
    logits: &Array4<f32>,
    text: &str,
    tokens: &[TokenSlice],
    entity_labels: &[&str],
    max_width: usize,
    threshold: f32,
) -> Vec<DecodedSpan> {
    let mut spans = Vec::new();
    let num_words = tokens.len();
    let width_count = logits.shape().get(2).copied().unwrap_or_default();
    let class_count = logits.shape().get(3).copied().unwrap_or_default();

    for start in 0..num_words {
        let actual_max_width = max_width
            .min(width_count)
            .min(num_words.saturating_sub(start));
        for width in 0..actual_max_width {
            let end_word = start + width;
            if end_word >= num_words {
                continue;
            }
            for class_idx in 0..class_count.min(entity_labels.len()) {
                let probability = sigmoid(logits[[0, start, width, class_idx]]);
                if probability < threshold {
                    continue;
                }
                let start_offset = tokens[start].start;
                let end_offset = tokens[end_word].end;
                spans.push(DecodedSpan {
                    start: start_offset,
                    end: end_offset,
                    text: text[start_offset..end_offset].to_string(),
                    class_name: entity_labels[class_idx].to_string(),
                    probability,
                });
            }
        }
    }

    spans.sort_unstable_by_key(|span| (span.start, span.end));
    // Remove overlapping spans using a greedy approach, preferring higher confidence.
    greedy_filter(spans)
}

// Helper functions for extracting tensors from ONNX DynValue into ndarray.
fn extract_f32_4d(value: &ort::value::DynValue) -> Result<Array4<f32>> {
    let (shape, data) = value.try_extract_tensor::<f32>()?;
    Ok(Array::from_shape_vec(shape.to_ixdyn(), data.to_vec())?.into_dimensionality::<Ix4>()?)
}

fn extract_f32_3d(value: &ort::value::DynValue) -> Result<Array3<f32>> {
    let (shape, data) = value.try_extract_tensor::<f32>()?;
    Ok(Array::from_shape_vec(shape.to_ixdyn(), data.to_vec())?.into_dimensionality::<Ix3>()?)
}

fn extract_i64_3d(value: &ort::value::DynValue) -> Result<Array3<i64>> {
    let (shape, data) = value.try_extract_tensor::<i64>()?;
    Ok(Array::from_shape_vec(shape.to_ixdyn(), data.to_vec())?.into_dimensionality::<Ix3>()?)
}

fn extract_bool_2d(value: &ort::value::DynValue) -> Result<Array2<bool>> {
    let (shape, data) = value.try_extract_tensor::<bool>()?;
    Ok(Array::from_shape_vec(shape.to_ixdyn(), data.to_vec())?.into_dimensionality::<Ix2>()?)
}

/// Filters a list of spans to remove overlaps, keeping the highest probability spans.
fn greedy_filter(spans: Vec<DecodedSpan>) -> Vec<DecodedSpan> {
    if spans.is_empty() {
        return spans;
    }

    let mut selected = Vec::with_capacity(spans.len());
    let mut previous = 0usize;
    let mut next = 1usize;

    while next < spans.len() {
        let left = &spans[previous];
        let right = &spans[next];
        if disjoint(left, right) {
            selected.push(left.clone());
            previous = next;
        } else if left.probability < right.probability {
            previous = next;
        }
        next += 1;
    }

    selected.push(spans[previous].clone());
    selected
}

/// Checks if two spans are logically disjoint (non-overlapping).
fn disjoint(left: &DecodedSpan, right: &DecodedSpan) -> bool {
    right.start >= left.end || right.end <= left.start
}

/// Maps internal model entity labels to standardized uppercase names.
fn normalize_entity_label(label: &str) -> &'static str {
    match label {
        "person" => "PERSON",
        "organization" => "ORGANIZATION",
        "project" => "PROJECT",
        "product" => "PRODUCT",
        "tool" => "TOOL",
        "topic" => "TOPIC",
        "work item" => "WORK_ITEM",
        "mode" => "MODE",
        "place" => "PLACE",
        "room" => "ROOM",
        "date" => "DATE",
        _ => "TOPIC",
    }
}

/// Maps internal model relation labels to standardized snake_case names.
fn normalize_relation_label(label: &str) -> String {
    match label {
        "owns" => "owns".to_string(),
        "uses" => "uses".to_string(),
        "works on" => "works_on".to_string(),
        "responsible for" => "responsible_for".to_string(),
        "reviews" => "reviews".to_string(),
        "works for" => "works_for".to_string(),
        "depends on" => "depends_on".to_string(),
        "prefers" => "prefers".to_string(),
        "has deadline" => "has_deadline".to_string(),
        "communicates with" => "communicates_with".to_string(),
        "investigates" => "investigates".to_string(),
        "evaluates" => "evaluates".to_string(),
        "north of" => "north_of".to_string(),
        "south of" => "south_of".to_string(),
        "east of" => "east_of".to_string(),
        "west of" => "west_of".to_string(),
        "avoids" => "avoids".to_string(),
        _ => label.to_string(),
    }
}

/// Standard sigmoid activation function.
fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}
