//! LLM-callable wrappers over the `learning` domain (the user-profile facet
//! cache).
//!
//! Learning has no public `ops` layer — its RPC handlers are private,
//! `ControllerFuture`-boxed functions in `schemas.rs` operating on a
//! [`FacetCache`] obtained via `memory::global`. These tools mirror that exact
//! handler logic (same `class/key` composition, same `FacetState` /
//! `UserState` transitions) so the agent-tool surface stays behaviourally
//! identical to the RPC surface. If a public `learning::ops` layer is added
//! later, both should delegate to it.
//!
//! The three reads (`learning_list_facets` / `learning_get_facet` /
//! `learning_cache_stats`) are default-enabled. Every mutator — pinning,
//! forgetting, rebuilding, resetting, and the profile writers — ships
//! default-OFF via `tools/user_filter.rs` (`learning_manage` toggle), because
//! they persistently rewrite the assistant's model of the user.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::agent::learning::cache::FacetCache;
use crate::openhuman::agent::learning::stability_detector::StabilityDetector;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use tinymemory_api::provider::{FacetState, ProfileFacet, UserState};

/// Acquire the profile facet cache, mirroring `learning::schemas::get_cache`.
///
/// Goes through the bound driver: facets moved behind the memory module, so
/// there is no process-global client to ask any more.
async fn get_cache() -> anyhow::Result<FacetCache> {
    let guard = crate::openhuman::memory::ops::guard::active_memory_guard()
        .await
        .map_err(|e| anyhow::anyhow!("memory unavailable: {e}"))?;
    Ok(FacetCache::new(guard))
}

/// Compose the full facet key from a class string + key suffix.
fn full_key(class_str: &str, key_suffix: &str) -> String {
    format!("{class_str}/{key_suffix}")
}

/// Validate a caller-supplied class name against the facet taxonomy.
///
/// Mirrors the RPC handlers' strict check so the agent-tool surface rejects an
/// unknown class instead of composing a key no facet can carry. Delegates to
/// the shared taxonomy validator and lifts its string error into `anyhow`.
fn validate_class(class_str: &str) -> anyhow::Result<()> {
    crate::openhuman::agent::learning::cache::parse_facet_class_name(class_str)
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!(e))
}

fn facet_to_json(f: &ProfileFacet) -> serde_json::Value {
    serde_json::to_value(f).unwrap_or(serde_json::Value::Null)
}

fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

/// List learned facets (active + provisional), optionally filtered by class.
pub struct LearningListFacetsTool;

#[async_trait]
impl Tool for LearningListFacetsTool {
    fn name(&self) -> &str {
        "learning_list_facets"
    }

    fn description(&self) -> &str {
        "List the assistant's learned facets about the user (active and \
         provisional states only), optionally filtered by `class` (e.g. \
         style, identity, tooling, goal). Each facet has a key, value, \
         confidence, and state. Use to see what the assistant has inferred."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "class": { "type": "string", "description": "Optional class filter (style|identity|tooling|veto|goal|channel)." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] list_facets invoked");
        let class_filter = args
            .get("class")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        // Reject an unknown class before touching the store, so a filter that
        // could never match a facet is an error rather than a silent empty list.
        if let Some(cls) = &class_filter {
            validate_class(cls)?;
        }
        let cache = get_cache().await?;
        let all = cache
            .list_all()
            .await
            .map_err(|e| anyhow::anyhow!("learning_list_facets: {e:#}"))?;
        let facets: Vec<serde_json::Value> = all
            .iter()
            .filter(|f| f.state == FacetState::Active || f.state == FacetState::Provisional)
            // Match on the class column when it is set — that stays
            // authoritative, so a row explicitly tagged with another class can
            // never match `cls` via its key prefix (the #6077 leak stays
            // closed). A row with no class column falls back to its key prefix,
            // where a canonical key like `style/verbosity` carries the class the
            // column omits — the behaviour the dropped `|| key.starts_with(...)`
            // arm provided for legitimate classless rows.
            .filter(|f| match &class_filter {
                Some(cls) => {
                    f.class.as_deref() == Some(cls.as_str())
                        || (f.class.is_none() && f.key.starts_with(&format!("{cls}/")))
                }
                None => true,
            })
            .map(facet_to_json)
            .collect();
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "count": facets.len(),
            "facets": facets,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Read a single facet by class + key.
pub struct LearningGetFacetTool;

#[async_trait]
impl Tool for LearningGetFacetTool {
    fn name(&self) -> &str {
        "learning_get_facet"
    }

    fn description(&self) -> &str {
        "Read one learned facet by `class` + `key`, returning the facet (or \
         `found: false`)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "class": { "type": "string" },
                "key": { "type": "string", "description": "Key suffix within the class." }
            },
            "required": ["class", "key"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] get_facet invoked");
        let class_str = read_required_str(&args, "class")?;
        let key_suffix = read_required_str(&args, "key")?;
        validate_class(&class_str)?;
        let fk = full_key(&class_str, &key_suffix);
        let cache = get_cache().await?;
        let facet = cache
            .get(&fk)
            .await
            .map_err(|e| anyhow::anyhow!("learning_get_facet: {e:#}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "found": facet.is_some(),
            "facet": facet.as_ref().map(facet_to_json),
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Aggregate facet-cache statistics.
pub struct LearningCacheStatsTool;

#[async_trait]
impl Tool for LearningCacheStatsTool {
    fn name(&self) -> &str {
        "learning_cache_stats"
    }

    fn description(&self) -> &str {
        "Report facet-cache health: total facets, counts by state \
         (active/provisional/candidate/dropped), and a per-class breakdown of \
         active facets."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] cache_stats invoked");
        let cache = get_cache().await?;
        let all = cache
            .list_all()
            .await
            .map_err(|e| anyhow::anyhow!("learning_cache_stats: {e:#}"))?;
        let count_state = |s: FacetState| all.iter().filter(|f| f.state == s).count();
        let mut by_class: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for f in all.iter().filter(|f| f.state == FacetState::Active) {
            let cls = f
                .class
                .clone()
                .or_else(|| f.key.split('/').next().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            *by_class.entry(cls).or_insert(0) += 1;
        }
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "total": all.len(),
            "active": count_state(FacetState::Active),
            "provisional": count_state(FacetState::Provisional),
            "candidate": count_state(FacetState::Candidate),
            "dropped": count_state(FacetState::Dropped),
            "by_class": by_class,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Update a facet's value and pin it. Default-OFF.
pub struct LearningUpdateFacetTool;

#[async_trait]
impl Tool for LearningUpdateFacetTool {
    fn name(&self) -> &str {
        "learning_update_facet"
    }

    fn description(&self) -> &str {
        "Set the `value` of a learned facet (`class` + `key`) and pin it so the \
         stability detector won't override it. Use to correct what the \
         assistant believes about the user."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "class": { "type": "string" },
                "key": { "type": "string" },
                "value": { "type": "string" }
            },
            "required": ["class", "key", "value"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] update_facet invoked");
        let class_str = read_required_str(&args, "class")?;
        let key_suffix = read_required_str(&args, "key")?;
        let value = read_required_str(&args, "value")?;
        validate_class(&class_str)?;
        let fk = full_key(&class_str, &key_suffix);
        let cache = get_cache().await?;
        let mut facet = cache
            .get(&fk)
            .await
            .map_err(|e| anyhow::anyhow!("learning_update_facet: {e:#}"))?
            .ok_or_else(|| anyhow::anyhow!("learning_update_facet: facet not found: {fk}"))?;
        facet.value = value;
        facet.user_state = UserState::Pinned;
        cache
            .upsert(&facet)
            .await
            .map_err(|e| anyhow::anyhow!("learning_update_facet: upsert failed: {e:#}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "facet": facet_to_json(&facet),
        }))?))
    }
}

/// Set/clear a facet's pin via `set_user_state`. Shared by pin/unpin.
async fn set_pin(
    args: serde_json::Value,
    tool: &str,
    state: UserState,
) -> anyhow::Result<ToolResult> {
    let class_str = read_required_str(&args, "class")?;
    let key_suffix = read_required_str(&args, "key")?;
    validate_class(&class_str)?;
    let fk = full_key(&class_str, &key_suffix);
    let cache = get_cache().await?;
    let updated = cache
        .set_user_state(&fk, state)
        .await
        .map_err(|e| anyhow::anyhow!("{tool}: set_user_state failed: {e:#}"))?;
    if !updated {
        return Err(anyhow::anyhow!("{tool}: facet not found: {fk}"));
    }
    let facet = cache
        .get(&fk)
        .await
        .map_err(|e| anyhow::anyhow!("{tool}: re-read failed: {e:#}"))?;
    Ok(ToolResult::success(serde_json::to_string(&json!({
        "facet": facet.as_ref().map(facet_to_json),
    }))?))
}

/// Pin a facet. Default-OFF.
pub struct LearningPinFacetTool;

#[async_trait]
impl Tool for LearningPinFacetTool {
    fn name(&self) -> &str {
        "learning_pin_facet"
    }

    fn description(&self) -> &str {
        "Pin a learned facet (`class` + `key`) so it stays active regardless of \
         the stability detector."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "class": { "type": "string" }, "key": { "type": "string" } },
            "required": ["class", "key"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] pin_facet invoked");
        set_pin(args, "learning_pin_facet", UserState::Pinned).await
    }
}

/// Unpin a facet (return to auto management). Default-OFF.
pub struct LearningUnpinFacetTool;

#[async_trait]
impl Tool for LearningUnpinFacetTool {
    fn name(&self) -> &str {
        "learning_unpin_facet"
    }

    fn description(&self) -> &str {
        "Unpin a learned facet (`class` + `key`), returning it to automatic \
         stability management."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "class": { "type": "string" }, "key": { "type": "string" } },
            "required": ["class", "key"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] unpin_facet invoked");
        set_pin(args, "learning_unpin_facet", UserState::Auto).await
    }
}

/// Forget a facet (semantic delete). Default-OFF.
pub struct LearningForgetFacetTool;

#[async_trait]
impl Tool for LearningForgetFacetTool {
    fn name(&self) -> &str {
        "learning_forget_facet"
    }

    fn description(&self) -> &str {
        "Forget a learned facet (`class` + `key`): marks it dropped and \
         forgotten so it won't resurface. Use when the assistant should \
         permanently unlearn something about the user."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "class": { "type": "string" }, "key": { "type": "string" } },
            "required": ["class", "key"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] forget_facet invoked");
        let class_str = read_required_str(&args, "class")?;
        let key_suffix = read_required_str(&args, "key")?;
        validate_class(&class_str)?;
        let fk = full_key(&class_str, &key_suffix);
        let cache = get_cache().await?;
        let facet_json = match cache
            .get(&fk)
            .await
            .map_err(|e| anyhow::anyhow!("learning_forget_facet: {e:#}"))?
        {
            Some(mut f) => {
                f.user_state = UserState::Forgotten;
                f.state = FacetState::Dropped;
                cache
                    .upsert(&f)
                    .await
                    .map_err(|e| anyhow::anyhow!("learning_forget_facet: upsert failed: {e:#}"))?;
                facet_to_json(&f)
            }
            None => serde_json::Value::Null,
        };
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "facet": facet_json,
        }))?))
    }
}

/// Rebuild the facet cache (heavyweight stability cycle). Default-OFF.
pub struct LearningRebuildCacheTool;

#[async_trait]
impl Tool for LearningRebuildCacheTool {
    fn name(&self) -> &str {
        "learning_rebuild_cache"
    }

    fn description(&self) -> &str {
        "Run a full stability-detector rebuild cycle over the facet cache \
         (re-scores and prunes facets). Heavyweight; returns added/evicted/kept \
         counts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] rebuild_cache invoked");
        let cache = get_cache().await?;
        let detector = StabilityDetector::new(cache);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let outcome = detector
            .rebuild(now)
            .await
            .map_err(|e| anyhow::anyhow!("learning_rebuild_cache: rebuild failed: {e:#}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "added": outcome.added,
            "evicted": outcome.evicted,
            "kept": outcome.kept,
            "total_size": outcome.total_size,
        }))?))
    }
}

/// Reset the facet cache (delete all auto facets, keep pinned). Default-OFF.
pub struct LearningResetCacheTool;

#[async_trait]
impl Tool for LearningResetCacheTool {
    fn name(&self) -> &str {
        "learning_reset_cache"
    }

    fn description(&self) -> &str {
        "Delete every automatically-managed facet from the cache, preserving \
         only user-pinned facets. Irreversible. Only use when the user wants to \
         wipe the assistant's learned model of them."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] reset_cache invoked");
        let cache = get_cache().await?;
        let (deleted, pinned_preserved) =
            crate::openhuman::agent::learning::cache::reset_non_pinned(&cache)
                .await
                .map_err(|e| anyhow::anyhow!("learning_reset_cache: {e:#}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "deleted": deleted,
            "pinned_preserved": pinned_preserved,
        }))?))
    }
}

/// Write PROFILE.md from supplied markdown. Default-OFF.
pub struct LearningSaveProfileTool;

#[async_trait]
impl Tool for LearningSaveProfileTool {
    fn name(&self) -> &str {
        "learning_save_profile"
    }

    fn description(&self) -> &str {
        "Write the supplied `markdown` to PROFILE.md in the workspace (the \
         assistant's durable profile of the user). When `summarize` is true, \
         compress it with the model first."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "markdown": { "type": "string", "description": "Profile markdown body (required)." },
                "summarize": { "type": "boolean", "description": "Compress with the model first (default false)." }
            },
            "required": ["markdown"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] save_profile invoked");
        let markdown = read_required_str(&args, "markdown")?;
        let summarize = args
            .get("summarize")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let config = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("learning_save_profile: {e}"))?;
        let body = if summarize {
            crate::openhuman::agent::learning::linkedin_enrichment::summarise_profile_with_llm(
                &config, &markdown,
            )
            .await
            .map_err(|e| anyhow::anyhow!("learning_save_profile: summarisation failed: {e:#}"))?
        } else {
            markdown
        };
        let path = config.workspace_dir.join("PROFILE.md");
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| anyhow::anyhow!("learning_save_profile: create dir failed: {e}"))?;
        }
        tokio::fs::write(&path, &body)
            .await
            .map_err(|e| anyhow::anyhow!("learning_save_profile: write failed: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "path": path.display().to_string(),
            "bytes": body.len(),
        }))?))
    }
}

/// Enrich the profile via LinkedIn (external scrape). Default-OFF.
pub struct LearningEnrichProfileTool;

#[async_trait]
impl Tool for LearningEnrichProfileTool {
    fn name(&self) -> &str {
        "learning_enrich_profile"
    }

    fn description(&self) -> &str {
        "Run the LinkedIn profile-enrichment pipeline: optionally with a preset \
         `profile_url`, scrape the profile (Apify) and build/persist a profile. \
         Reaches external services. Only use when the user asks to enrich their \
         profile."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "profile_url": { "type": "string", "description": "Optional preset LinkedIn profile URL." }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn external_effect(&self) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][learning] enrich_profile invoked");
        let preset = args
            .get("profile_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let config = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("learning_enrich_profile: {e}"))?;
        let result =
            crate::openhuman::agent::learning::linkedin_enrichment::run_linkedin_enrichment(
                &config, preset,
            )
            .await
            .map_err(|e| anyhow::anyhow!("learning_enrich_profile: {e:#}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "profile_url": result.profile_url,
            "profile_data": result.profile_data,
            "stages": result.stages,
            "log": result.log,
        }))?))
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
