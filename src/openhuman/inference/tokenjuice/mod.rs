//! OpenHuman host adapter for the separately released TinyJuice module.

pub mod config_patch;
pub mod ml;
pub mod savings;
pub mod schemas;
pub mod tools;
pub mod types;

use tinyjuice_bus::names::methods;

pub use tools::TokenjuiceRetrieveTool;
pub use types::{AgentTokenjuiceCompression, CompressorKind, ContentKind};

use types::InstallRequest;

pub const RETRIEVE_TOOL_NAME: &str = "tinyjuice_retrieve";
pub const LEGACY_RETRIEVE_TOOL_NAME: &str = "retrieve_tool_output";
pub const RECOVERY_TOOL_NAMES: &[&str] = &[
    RETRIEVE_TOOL_NAME,
    "tokenjuice_retrieve",
    LEGACY_RETRIEVE_TOOL_NAME,
];

pub fn is_recovery_tool(name: &str) -> bool {
    RECOVERY_TOOL_NAMES.contains(&name)
}

pub async fn install_from_config(config: &crate::openhuman::config::Config) -> Result<(), String> {
    let tj = &config.tokenjuice;
    ml::configure(config.clone());
    savings::configure(
        config
            .default_model
            .clone()
            .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string()),
        &config.workspace_dir,
    );
    let request = InstallRequest {
        options: types::CompressOptions {
            router_enabled: tj.router_enabled,
            ccr_enabled: tj.ccr_enabled,
            search_enabled: tj.search_enabled,
            code_enabled: tj.code_enabled,
            html_enabled: tj.html_enabled,
            ml_text_enabled: tj.ml_compression_enabled,
            min_bytes_to_compress: tj.min_bytes_to_compress,
            ccr_min_tokens: tj.ccr_min_tokens,
            ..types::CompressOptions::default()
        },
        max_cache_entries: tj.max_cache_entries,
        max_cache_bytes: tj.max_cache_bytes,
        ccr_ttl_secs: tj.ccr_ttl_secs,
        disk_tier_root: tj
            .ccr_disk_enabled
            .then(|| config.workspace_dir.join(".tokenjuice").join("ccr"))
            .map(|path| path.to_string_lossy().into_owned()),
    };
    let fingerprint = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
    static INSTALLED: std::sync::OnceLock<tokio::sync::Mutex<Option<Vec<u8>>>> =
        std::sync::OnceLock::new();
    let mut installed = INSTALLED
        .get_or_init(|| tokio::sync::Mutex::new(None))
        .lock()
        .await;
    if installed.as_ref() == Some(&fingerprint) {
        return Ok(());
    }
    proxy(config)
        .await?
        .call::<()>(methods::INSTALL, (request,))
        .await
        .map_err(|e| e.to_string())?;
    *installed = Some(fingerprint);
    Ok(())
}

#[cfg(feature = "modules")]
pub(super) async fn proxy(
    config: &crate::openhuman::config::Config,
) -> Result<tinybus::Proxy, String> {
    let config = {
        let mut test_config = config.clone();
        if let Some(path) = std::env::var_os("TINYJUICE_TEST_MODULE") {
            // An explicit fixture is an opt-in to module execution even when
            // the ambient test workspace has persisted modules = disabled.
            test_config.modules.enabled = true;
            test_config
                .modules
                .overrides
                .push(crate::openhuman::config::schema::ModuleOverride {
                    id: "tinyjuice".to_string(),
                    path: path.to_string_lossy().into_owned(),
                });
        }
        test_config
    };
    let config = &config;

    crate::openhuman::modules::ensure_loaded(config, "tinyjuice").await?;
    let record = crate::openhuman::modules::registry::find("tinyjuice")
        .ok_or_else(|| "unknown module 'tinyjuice'".to_string())?;
    crate::openhuman::modules::host::runtime()
        .await
        .map_err(|e| e.to_string())?
        .proxy(record.bus_name, record.object_path)
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "modules"))]
pub(super) async fn proxy(
    _config: &crate::openhuman::config::Config,
) -> Result<tinybus::Proxy, String> {
    Err("native modules are not compiled into this build".to_string())
}

pub async fn compact_output_with_policy(
    content: String,
    tool_name: &str,
    enabled: bool,
    profile: AgentTokenjuiceCompression,
) -> String {
    if !enabled || profile == AgentTokenjuiceCompression::Off {
        return content;
    }
    let config = match crate::openhuman::config::Config::load_or_init().await {
        Ok(config) => config,
        Err(error) => {
            log::debug!("[tokenjuice] config unavailable, passing through: {error}");
            return content;
        }
    };
    #[cfg(test)]
    let config = if std::env::var_os("TINYJUICE_TEST_MODULE").is_some() {
        // The released-module regression must not inherit an operator's
        // persisted compression thresholds or disabled router flags.
        crate::openhuman::config::Config::default()
    } else {
        config
    };
    if let Err(error) = install_from_config(&config).await {
        log::debug!("[tokenjuice] module configuration failed, passing through: {error}");
        return content;
    }
    let proxy = match proxy(&config).await {
        Ok(proxy) => proxy,
        Err(error) => {
            log::debug!("[tokenjuice] module unavailable, passing through: {error}");
            return content;
        }
    };
    let response: types::CompactResponse = match proxy
        .call(
            methods::COMPACT,
            (content.clone(), tool_name.to_string(), enabled, profile),
        )
        .await
    {
        Ok(response) => response,
        Err(error) => {
            log::debug!("[tokenjuice] module compaction failed, passing through: {error}");
            return content;
        }
    };
    record_savings(&response);
    response.text
}

fn record_savings(response: &types::CompactResponse) {
    use std::str::FromStr as _;
    let kind = ContentKind::from_str(&response.content_kind).unwrap_or(ContentKind::PlainText);
    let compressor = CompressorKind::from_str(&response.compressor).unwrap_or(CompressorKind::None);
    savings::record(
        kind,
        compressor,
        response.original_tokens,
        response.compacted_tokens,
    );
}

pub async fn detect(content: String, hint: types::ContentHint) -> Result<String, String> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|error| error.to_string())?;
    proxy(&config)
        .await?
        .call(methods::DETECT, (content, hint))
        .await
        .map_err(|error| error.to_string())
}

pub async fn compress(
    content: String,
    hint: types::ContentHint,
) -> Result<types::CompressedOutput, String> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|error| error.to_string())?;
    install_from_config(&config).await?;
    let response: types::CompressedOutput = proxy(&config)
        .await?
        .call(methods::COMPRESS, (content, hint))
        .await
        .map_err(|error| error.to_string())?;
    savings::record(
        response.content_kind,
        response.compressor,
        (response.original_bytes as u64).div_ceil(4),
        (response.compacted_bytes as u64).div_ceil(4),
    );
    Ok(response)
}

pub async fn retrieve(
    token: String,
    range: Option<types::RetrieveRange>,
) -> Result<Option<String>, String> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|error| error.to_string())?;
    install_from_config(&config).await?;
    proxy(&config)
        .await?
        .call(methods::RETRIEVE, (token, range))
        .await
        .map_err(|error| error.to_string())
}

pub async fn cache_stats() -> Result<types::CacheStats, String> {
    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|error| error.to_string())?;
    install_from_config(&config).await?;
    proxy(&config)
        .await?
        .call(methods::CACHE_STATS, ())
        .await
        .map_err(|error| error.to_string())
}

pub fn all_tokenjuice_registered_controllers() -> Vec<crate::core::all::RegisteredController> {
    schemas::all_registered_controllers()
}

pub fn all_tokenjuice_controller_schemas() -> Vec<crate::core::ControllerSchema> {
    schemas::all_controller_schemas()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
