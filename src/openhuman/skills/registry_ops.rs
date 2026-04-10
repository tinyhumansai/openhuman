//! Core operations for the OpenHuman Skills registry.
//!
//! This module handles fetching the skill registry (from remote or local sources),
//! searching available skills, and performing installation/uninstallation of
//! skill bundles into the workspace.

use std::path::Path;

use sha2::{Digest, Sha256};

use super::registry_cache::{
    is_cache_fresh, is_local_path, local_skills_dir, read_cache, read_local_file, registry_url,
    tag_categories, write_cache,
};
use super::registry_types::{
    AvailableSkillEntry, InstalledSkillInfo, RegistrySkillEntry, RemoteSkillRegistry,
};

/// Fetch the skill registry. Supports both remote HTTP URLs and local file paths.
///
/// When `SKILLS_REGISTRY_URL` points to a local file (absolute path or `file://` URI),
/// the registry is read directly from disk — no caching is applied so changes are
/// picked up immediately (ideal for local development).
///
/// For remote URLs, uses a 1-hour disk cache unless `force` is true.
pub async fn registry_fetch(
    workspace_dir: &Path,
    force: bool,
) -> Result<RemoteSkillRegistry, String> {
    let url = registry_url();

    // --- Local file path: read directly, skip cache for instant dev feedback ---
    if is_local_path(&url) {
        log::info!("[registry] reading local registry from {url}");
        let bytes = read_local_file(&url)?;
        let body = String::from_utf8(bytes)
            .map_err(|e| format!("registry file is not valid UTF-8: {e}"))?;
        let mut registry: RemoteSkillRegistry = serde_json::from_str(&body)
            .map_err(|e| format!("failed to parse local registry JSON: {e}"))?;

        // Ensure category flags are set correctly based on the registry structure
        tag_categories(&mut registry);

        log::info!(
            "[registry] loaded {} core + {} third-party skills from local file",
            registry.skills.core.len(),
            registry.skills.third_party.len()
        );
        return Ok(registry);
    }

    // --- Remote URL: use disk cache ---
    if !force {
        if let Some(cached) = read_cache(workspace_dir) {
            if is_cache_fresh(&cached) {
                log::debug!("[registry] returning cached registry");
                return Ok(cached.registry);
            }
        }
    }

    log::info!("[registry] fetching registry from {url}");

    // Use rustls explicitly so we never fall back to native-tls (which can hang
    // on macOS under the Hardened Runtime when the system keychain is restricted).
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .use_rustls_tls()
        .build()
        .map_err(|e| format!("failed to create HTTP client: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("failed to fetch registry from {url}: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "registry fetch failed with status {}",
            resp.status()
        ));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("failed to read registry body: {e}"))?;

    let mut registry: RemoteSkillRegistry =
        serde_json::from_str(&body).map_err(|e| format!("failed to parse registry JSON: {e}"))?;

    tag_categories(&mut registry);
    write_cache(workspace_dir, &registry)?;

    log::info!(
        "[registry] fetched {} core + {} third-party skills",
        registry.skills.core.len(),
        registry.skills.third_party.len()
    );

    Ok(registry)
}

/// Search the registry by query string, optionally filtering by category.
///
/// Matches against the skill ID, name, and description.
pub async fn registry_search(
    workspace_dir: &Path,
    query: &str,
    category: Option<&str>,
) -> Result<Vec<RegistrySkillEntry>, String> {
    let registry = registry_fetch(workspace_dir, false).await?;
    let query_lower = query.to_lowercase();

    // Closure to check if a skill entry matches the search query
    let matches_query = |entry: &RegistrySkillEntry| -> bool {
        entry.id.to_lowercase().contains(&query_lower)
            || entry.name.to_lowercase().contains(&query_lower)
            || entry.description.to_lowercase().contains(&query_lower)
    };

    let mut results: Vec<RegistrySkillEntry> = Vec::new();

    let include_core = category.is_none_or(|c| c == "core");
    let include_third_party = category.is_none_or(|c| c == "third_party");

    if include_core {
        results.extend(
            registry
                .skills
                .core
                .into_iter()
                .filter(|e| matches_query(e)),
        );
    }
    if include_third_party {
        results.extend(
            registry
                .skills
                .third_party
                .into_iter()
                .filter(|e| matches_query(e)),
        );
    }

    Ok(results)
}

/// Fetch bytes from a URL — supports both local file paths and HTTP URLs.
async fn fetch_url_bytes(url: &str) -> Result<Vec<u8>, String> {
    if is_local_path(url) {
        return read_local_file(url);
    }

    log::debug!("[registry] fetch_url_bytes: connecting to {url}");

    // Use rustls explicitly so we never fall back to native-tls (which can hang
    // on macOS under the Hardened Runtime when the system keychain is restricted).
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .use_rustls_tls()
        .build()
        .map_err(|e| format!("failed to create HTTP client: {e}"))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("[registry] network error fetching {url}: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "[registry] fetch {url} returned HTTP {}",
            resp.status()
        ));
    }

    let bytes = resp
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("[registry] failed to read response body from {url}: {e}"))?;

    log::debug!(
        "[registry] fetch_url_bytes: {} bytes from {url}",
        bytes.len()
    );
    Ok(bytes)
}

/// Install a skill from the registry or local directory.
///
/// When `SKILLS_LOCAL_DIR` is set, copies files directly from the local skills
/// directory (e.g. `$SKILLS_LOCAL_DIR/<skill_id>/`) instead of downloading.
/// This also works when registry entry URLs are local file paths.
pub async fn skill_install(workspace_dir: &Path, skill_id: &str) -> Result<(), String> {
    log::info!(
        "[registry] skills_install: starting — skill_id={skill_id} workspace={}",
        workspace_dir.display()
    );

    // --- Fast path: SKILLS_LOCAL_DIR copies directly from local dev directory ---
    // This allows developers to work on skills locally and see changes reflected instantly
    // in the app without having to publish to a registry.
    if let Some(local_dir) = local_skills_dir() {
        let local_skill = local_dir.join(skill_id);
        if local_skill.exists() {
            log::info!(
                "[registry] installing '{skill_id}' from local dir: {}",
                local_skill.display()
            );
            let skill_dir = workspace_dir.join("skills").join(skill_id);
            copy_dir_recursive(&local_skill, &skill_dir)?;
            log::info!("[registry] skill '{skill_id}' installed from local dir");
            return Ok(());
        }
        log::warn!(
            "[registry] SKILLS_LOCAL_DIR set but '{skill_id}' not found at {}; falling back to registry",
            local_skill.display()
        );
    }

    // --- Standard path: fetch from registry (remote or local URLs) ---
    log::debug!("[registry] skills_install: fetching registry for '{skill_id}'");
    let registry = registry_fetch(workspace_dir, false).await?;
    log::debug!("[registry] skills_install: registry fetched, looking up '{skill_id}'");

    let entry = registry
        .skills
        .core
        .iter()
        .chain(registry.skills.third_party.iter())
        .find(|e| e.id == skill_id)
        .ok_or_else(|| format!("skill '{skill_id}' not found in registry"))?
        .clone();

    log::debug!(
        "[registry] skills_install: found entry '{skill_id}' v{} manifest={} bundle={}",
        entry.version,
        entry.manifest_url,
        entry.download_url
    );

    let skill_dir = workspace_dir.join("skills").join(skill_id);
    std::fs::create_dir_all(&skill_dir).map_err(|e| format!("failed to create skill dir: {e}"))?;

    // Fetch manifest (local or remote)
    log::info!(
        "[registry] skills_install: fetching manifest for '{skill_id}' from {}",
        entry.manifest_url
    );
    let manifest_bytes = fetch_url_bytes(&entry.manifest_url).await?;
    log::debug!(
        "[registry] skills_install: manifest fetched ({} bytes)",
        manifest_bytes.len()
    );

    // Fetch JS bundle (local or remote)
    log::info!(
        "[registry] skills_install: fetching JS bundle for '{skill_id}' from {}",
        entry.download_url
    );
    let js_bytes = fetch_url_bytes(&entry.download_url).await?;
    log::debug!(
        "[registry] skills_install: JS bundle fetched ({} bytes)",
        js_bytes.len()
    );

    // Verify checksum if present to ensure integrity of the downloaded bundle
    if let Some(expected) = &entry.checksum_sha256 {
        let mut hasher = Sha256::new();
        hasher.update(&js_bytes);
        let actual = format!("{:x}", hasher.finalize());
        if actual != *expected {
            // Clean up the directory if verification fails
            let _ = std::fs::remove_dir_all(&skill_dir);
            return Err(format!(
                "checksum mismatch for '{skill_id}': expected {expected}, got {actual}"
            ));
        }
        log::debug!("[registry] checksum verified for '{skill_id}'");
    }

    // Write the fetched files to the local skill directory
    std::fs::write(skill_dir.join("manifest.json"), &manifest_bytes)
        .map_err(|e| format!("failed to write manifest: {e}"))?;
    std::fs::write(skill_dir.join(&entry.entry), &js_bytes)
        .map_err(|e| format!("failed to write JS bundle: {e}"))?;

    log::info!("[registry] skill '{skill_id}' installed successfully");
    Ok(())
}

/// Recursively copy a directory tree from `src` to `dst`.
///
/// Used primarily for local skill development to sync files from a source directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("failed to create dir {}: {e}", dst.display()))?;

    let entries =
        std::fs::read_dir(src).map_err(|e| format!("failed to read dir {}: {e}", src.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read dir entry: {e}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "failed to copy {} -> {}: {e}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }

    Ok(())
}

/// Uninstall a skill by removing its directory from the workspace.
pub async fn skill_uninstall(workspace_dir: &Path, skill_id: &str) -> Result<(), String> {
    let skill_dir = workspace_dir.join("skills").join(skill_id);
    if !skill_dir.exists() {
        return Err(format!("skill '{skill_id}' is not installed"));
    }

    std::fs::remove_dir_all(&skill_dir)
        .map_err(|e| format!("failed to remove skill directory: {e}"))?;

    log::info!("[registry] skill '{skill_id}' uninstalled");
    Ok(())
}

/// List all installed skills by scanning the workspace skills directory.
///
/// Parses the `manifest.json` in each subdirectory to gather skill information.
pub async fn skills_list_installed(
    workspace_dir: &Path,
) -> Result<Vec<InstalledSkillInfo>, String> {
    let skills_dir = workspace_dir.join("skills");
    let entries = match std::fs::read_dir(&skills_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(Vec::new()),
    };

    let mut installed = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let dir_name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden directories
        if dir_name.starts_with('.') {
            continue;
        }

        let manifest_path = path.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }

        let content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) {
            installed.push(InstalledSkillInfo {
                id: manifest
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&dir_name)
                    .to_string(),
                name: manifest
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&dir_name)
                    .to_string(),
                version: manifest
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                description: manifest
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                runtime: manifest
                    .get("runtime")
                    .and_then(|v| v.as_str())
                    .unwrap_or("quickjs")
                    .to_string(),
            });
        }
    }

    Ok(installed)
}

/// List all available skills from the registry, enriched with installed status.
///
/// Compares the remote registry with the locally installed skills to determine
/// if a skill is installed and if updates are available.
pub async fn skills_list_available(
    workspace_dir: &Path,
) -> Result<Vec<AvailableSkillEntry>, String> {
    let registry = registry_fetch(workspace_dir, false).await?;
    let installed = skills_list_installed(workspace_dir).await?;

    let installed_map: std::collections::HashMap<&str, &InstalledSkillInfo> =
        installed.iter().map(|s| (s.id.as_str(), s)).collect();

    let mut available = Vec::new();

    let all_entries = registry
        .skills
        .core
        .into_iter()
        .chain(registry.skills.third_party.into_iter());

    for entry in all_entries {
        let is_installed = installed_map.contains_key(entry.id.as_str());
        let installed_version = installed_map
            .get(entry.id.as_str())
            .map(|s| s.version.clone());
        let update_available = installed_version
            .as_ref()
            .is_some_and(|v| !v.is_empty() && *v != entry.version);

        available.push(AvailableSkillEntry {
            registry: entry,
            installed: is_installed,
            installed_version,
            update_available,
        });
    }

    Ok(available)
}

#[cfg(test)]
mod tests {
    use super::super::registry_cache::{is_cache_fresh, write_cache};
    use super::super::registry_types::{CachedRegistry, RegistrySkillCategories, SkillCategory};
    use super::*;

    fn make_workspace() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("skills")).unwrap();
        dir
    }

    fn sample_registry() -> RemoteSkillRegistry {
        RemoteSkillRegistry {
            version: 1,
            generated_at: "2026-03-30T12:00:00Z".to_string(),
            skills: RegistrySkillCategories {
                core: vec![
                    RegistrySkillEntry {
                        id: "gmail".to_string(),
                        name: "Gmail".to_string(),
                        version: "1.0.0".to_string(),
                        description: "Gmail integration for email".to_string(),
                        runtime: "quickjs".to_string(),
                        entry: "index.js".to_string(),
                        auto_start: false,
                        platforms: Some(vec!["macos".into(), "windows".into()]),
                        setup: None,
                        ignore_in_production: false,
                        download_url: String::new(),
                        manifest_url: String::new(),
                        checksum_sha256: None,
                        author: None,
                        repository: None,
                        category: SkillCategory::Core,
                    },
                    RegistrySkillEntry {
                        id: "notion".to_string(),
                        name: "Notion".to_string(),
                        version: "1.1.0".to_string(),
                        description: "Notion workspace integration".to_string(),
                        runtime: "quickjs".to_string(),
                        entry: "index.js".to_string(),
                        auto_start: false,
                        platforms: None,
                        setup: None,
                        ignore_in_production: false,
                        download_url: String::new(),
                        manifest_url: String::new(),
                        checksum_sha256: None,
                        author: None,
                        repository: None,
                        category: SkillCategory::Core,
                    },
                ],
                third_party: vec![RegistrySkillEntry {
                    id: "custom-tracker".to_string(),
                    name: "Custom Tracker".to_string(),
                    version: "0.1.0".to_string(),
                    description: "A custom price tracker".to_string(),
                    runtime: "quickjs".to_string(),
                    entry: "index.js".to_string(),
                    auto_start: false,
                    platforms: None,
                    setup: None,
                    ignore_in_production: false,
                    download_url: String::new(),
                    manifest_url: String::new(),
                    checksum_sha256: None,
                    author: Some("dev".into()),
                    repository: Some("https://github.com/dev/tracker".into()),
                    category: SkillCategory::ThirdParty,
                }],
            },
        }
    }

    fn create_installed_skill(workspace: &Path, id: &str, version: &str) {
        let skill_dir = workspace.join("skills").join(id);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let manifest = serde_json::json!({
            "id": id,
            "name": id,
            "version": version,
            "description": format!("Test skill {id}"),
            "runtime": "quickjs",
            "entry": "index.js"
        });
        std::fs::write(
            skill_dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(skill_dir.join("index.js"), "function init() {}").unwrap();
    }

    // --- Cache tests ---

    #[test]
    fn test_registry_cache_write_and_read() {
        let ws = make_workspace();
        let registry = sample_registry();
        write_cache(ws.path(), &registry).unwrap();

        let cached = read_cache(ws.path()).unwrap();
        assert_eq!(cached.registry.version, 1);
        assert_eq!(cached.registry.skills.core.len(), 2);
    }

    #[test]
    fn test_registry_cache_ttl_expired() {
        let _ws = make_workspace();
        let cached = CachedRegistry {
            fetched_at: "2020-01-01T00:00:00Z".to_string(),
            registry: sample_registry(),
        };
        assert!(!is_cache_fresh(&cached));

        let fresh = CachedRegistry {
            fetched_at: chrono::Utc::now().to_rfc3339(),
            registry: sample_registry(),
        };
        assert!(is_cache_fresh(&fresh));
    }

    // --- Search tests (using cache directly to avoid HTTP) ---

    fn search_entries(entries: &[RegistrySkillEntry], query: &str) -> Vec<RegistrySkillEntry> {
        let query_lower = query.to_lowercase();
        entries
            .iter()
            .filter(|e| {
                e.id.to_lowercase().contains(&query_lower)
                    || e.name.to_lowercase().contains(&query_lower)
                    || e.description.to_lowercase().contains(&query_lower)
            })
            .cloned()
            .collect()
    }

    #[test]
    fn test_registry_search_by_name() {
        let registry = sample_registry();
        let all: Vec<_> = registry
            .skills
            .core
            .iter()
            .chain(registry.skills.third_party.iter())
            .cloned()
            .collect();
        let results = search_entries(&all, "gmail");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "gmail");
    }

    #[test]
    fn test_registry_search_by_description() {
        let registry = sample_registry();
        let all: Vec<_> = registry
            .skills
            .core
            .iter()
            .chain(registry.skills.third_party.iter())
            .cloned()
            .collect();
        let results = search_entries(&all, "price tracker");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "custom-tracker");
    }

    #[test]
    fn test_registry_search_case_insensitive() {
        let registry = sample_registry();
        let all: Vec<_> = registry
            .skills
            .core
            .iter()
            .chain(registry.skills.third_party.iter())
            .cloned()
            .collect();
        let results = search_entries(&all, "NOTION");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "notion");
    }

    #[test]
    fn test_registry_search_with_category_filter() {
        let registry = sample_registry();
        // Core only
        let results = search_entries(&registry.skills.core, "email");
        assert_eq!(results.len(), 1); // gmail matches "Gmail integration for email"
        assert_eq!(results[0].id, "gmail");

        // Third party only
        let results = search_entries(&registry.skills.third_party, "tracker");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "custom-tracker");
    }

    // --- Install/Uninstall tests using mock HTTP server ---

    #[tokio::test]
    async fn test_skill_install_creates_files() {
        let manifest = serde_json::json!({
            "id": "test-skill",
            "name": "Test Skill",
            "version": "1.0.0",
            "runtime": "quickjs",
            "entry": "index.js"
        });
        let src = tempfile::TempDir::new().unwrap();
        let manifest_source_path = src.path().join("manifest.json");
        let js_source_path = src.path().join("index.js");
        let manifest_source_url = reqwest::Url::from_file_path(&manifest_source_path)
            .expect("manifest source path must convert to file:// URL")
            .to_string();
        let js_source_url = reqwest::Url::from_file_path(&js_source_path)
            .expect("js source path must convert to file:// URL")
            .to_string();
        std::fs::write(
            &manifest_source_path,
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        std::fs::write(&js_source_path, "function init() { console.log('hello'); }").unwrap();

        // Build a registry pointing at our mock server
        let registry = RemoteSkillRegistry {
            version: 1,
            generated_at: chrono::Utc::now().to_rfc3339(),
            skills: RegistrySkillCategories {
                core: vec![RegistrySkillEntry {
                    id: "test-skill".to_string(),
                    name: "Test Skill".to_string(),
                    version: "1.0.0".to_string(),
                    description: "A test skill".to_string(),
                    runtime: "quickjs".to_string(),
                    entry: "index.js".to_string(),
                    auto_start: false,
                    platforms: None,
                    setup: None,
                    ignore_in_production: false,
                    download_url: js_source_url.clone(),
                    manifest_url: manifest_source_url.clone(),
                    checksum_sha256: None,
                    author: None,
                    repository: None,
                    category: SkillCategory::Core,
                }],
                third_party: vec![],
            },
        };

        let ws = make_workspace();
        // Pre-populate cache so skill_install doesn't need to fetch registry via HTTP
        write_cache(ws.path(), &registry).unwrap();

        skill_install(ws.path(), "test-skill").await.unwrap();

        let manifest_path = ws.path().join("skills/test-skill/manifest.json");
        let js_path = ws.path().join("skills/test-skill/index.js");
        assert!(manifest_path.exists(), "manifest.json should exist");
        assert!(js_path.exists(), "index.js should exist");

        let installed_manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(installed_manifest["id"], "test-skill");

        let installed_js = std::fs::read_to_string(&js_path).unwrap();
        assert!(installed_js.contains("init"));
    }

    #[tokio::test]
    async fn test_skill_install_checksum_verification() {
        let js_content = "function init() { return 42; }";
        let mut hasher = Sha256::new();
        hasher.update(js_content.as_bytes());
        let correct_checksum = format!("{:x}", hasher.finalize());
        let src = tempfile::TempDir::new().unwrap();
        let manifest_source_path = src.path().join("manifest.json");
        let js_source_path = src.path().join("index.js");
        let manifest_source_url = reqwest::Url::from_file_path(&manifest_source_path)
            .expect("manifest source path must convert to file:// URL")
            .to_string();
        let js_source_url = reqwest::Url::from_file_path(&js_source_path)
            .expect("js source path must convert to file:// URL")
            .to_string();
        std::fs::write(
            &manifest_source_path,
            r#"{"id":"cs-skill","name":"CS Skill","version":"1.0.0"}"#,
        )
        .unwrap();
        std::fs::write(&js_source_path, js_content).unwrap();

        // Test with correct checksum
        let registry = RemoteSkillRegistry {
            version: 1,
            generated_at: chrono::Utc::now().to_rfc3339(),
            skills: RegistrySkillCategories {
                core: vec![RegistrySkillEntry {
                    id: "cs-skill".to_string(),
                    name: "CS Skill".to_string(),
                    version: "1.0.0".to_string(),
                    description: "".to_string(),
                    runtime: "quickjs".to_string(),
                    entry: "index.js".to_string(),
                    auto_start: false,
                    platforms: None,
                    setup: None,
                    ignore_in_production: false,
                    download_url: js_source_url.clone(),
                    manifest_url: manifest_source_url.clone(),
                    checksum_sha256: Some(correct_checksum.clone()),
                    author: None,
                    repository: None,
                    category: SkillCategory::Core,
                }],
                third_party: vec![],
            },
        };

        let ws = make_workspace();
        write_cache(ws.path(), &registry).unwrap();
        assert!(skill_install(ws.path(), "cs-skill").await.is_ok());

        // Test with wrong checksum
        let ws2 = make_workspace();
        let mut bad_registry = registry.clone();
        bad_registry.skills.core[0].checksum_sha256 = Some("wrong_checksum".to_string());
        write_cache(ws2.path(), &bad_registry).unwrap();
        let result = skill_install(ws2.path(), "cs-skill").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("checksum mismatch"));
    }

    #[tokio::test]
    async fn test_skill_install_not_in_registry() {
        let ws = make_workspace();
        let registry = RemoteSkillRegistry {
            version: 1,
            generated_at: chrono::Utc::now().to_rfc3339(),
            skills: RegistrySkillCategories {
                core: vec![],
                third_party: vec![],
            },
        };
        write_cache(ws.path(), &registry).unwrap();

        let result = skill_install(ws.path(), "nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found in registry"));
    }

    #[tokio::test]
    async fn test_skill_uninstall_removes_directory() {
        let ws = make_workspace();
        create_installed_skill(ws.path(), "to-remove", "1.0.0");

        assert!(ws.path().join("skills/to-remove").exists());
        skill_uninstall(ws.path(), "to-remove").await.unwrap();
        assert!(!ws.path().join("skills/to-remove").exists());
    }

    #[tokio::test]
    async fn test_skill_uninstall_nonexistent() {
        let ws = make_workspace();
        let result = skill_uninstall(ws.path(), "does-not-exist").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not installed"));
    }

    #[tokio::test]
    async fn test_list_installed_empty() {
        let ws = make_workspace();
        let installed = skills_list_installed(ws.path()).await.unwrap();
        assert!(installed.is_empty());
    }

    #[tokio::test]
    async fn test_list_installed_with_skills() {
        let ws = make_workspace();
        create_installed_skill(ws.path(), "gmail", "1.0.0");
        create_installed_skill(ws.path(), "notion", "1.1.0");

        let installed = skills_list_installed(ws.path()).await.unwrap();
        assert_eq!(installed.len(), 2);

        let ids: Vec<&str> = installed.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"gmail"));
        assert!(ids.contains(&"notion"));
    }

    #[tokio::test]
    async fn test_list_available_marks_installed() {
        let ws = make_workspace();

        // Install gmail at version 1.0.0 (registry has 1.0.0)
        create_installed_skill(ws.path(), "gmail", "1.0.0");
        // Install notion at version 1.0.0 (registry has 1.1.0 → update available)
        create_installed_skill(ws.path(), "notion", "1.0.0");

        // Write a registry to cache
        let registry = sample_registry();
        write_cache(ws.path(), &registry).unwrap();

        let available = skills_list_available(ws.path()).await.unwrap();
        assert_eq!(available.len(), 3); // gmail, notion, custom-tracker

        let gmail = available.iter().find(|a| a.registry.id == "gmail").unwrap();
        assert!(gmail.installed);
        assert!(!gmail.update_available); // same version

        let notion = available
            .iter()
            .find(|a| a.registry.id == "notion")
            .unwrap();
        assert!(notion.installed);
        assert!(notion.update_available); // 1.0.0 vs 1.1.0

        let tracker = available
            .iter()
            .find(|a| a.registry.id == "custom-tracker")
            .unwrap();
        assert!(!tracker.installed);
        assert!(!tracker.update_available);
    }
}
