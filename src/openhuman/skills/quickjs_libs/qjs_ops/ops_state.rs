//! State and memory operations: skill state get/set, filesystem data access, and memory insertion.

use crate::openhuman::memory::store::profile::FacetType;
use crate::openhuman::memory::NamespaceDocumentInput;
use parking_lot::RwLock;
use rquickjs::{Ctx, Function, Object};
use serde::Deserialize;
use std::sync::Arc;

use super::types::{js_err, SkillContext, SkillState};

/// Input structure for the `memory_insert` operation.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsMemoryInsertInput {
    /// Title of the document to be stored in memory.
    title: String,
    /// Content of the document.
    content: String,
    /// Type of the source (e.g., "doc", "chat", "email").
    source_type: Option<String>,
    /// Optional metadata associated with the document.
    metadata: Option<serde_json::Value>,
    /// Priority of the document ("high", "medium", "low").
    priority: Option<String>,
    /// Unix timestamp when the document was created.
    created_at: Option<f64>,
    /// Unix timestamp when the document was last updated.
    updated_at: Option<f64>,
    /// Optional unique identifier for the document.
    document_id: Option<String>,
}

/// Payload shape for `memory.updateOwner`. Skills push identity facts
/// about the owner of this OpenHuman instance here; facts land in the
/// `user_profile` SQLite table, rich documents land in the dedicated
/// `owner` memory namespace.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsOwnerUpdateInput {
    /// Structured, evidence-counted facts about the owner.
    #[serde(default)]
    facts: Vec<JsOwnerFact>,
    /// Optional rich document (e.g. a bio scraped from a Gmail signature).
    #[serde(default)]
    document: Option<JsOwnerDoc>,
}

/// A single owner fact — one row in `user_profile` after upsert.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsOwnerFact {
    /// One of `"identity"`, `"preference"`, `"skill"`, `"role"`,
    /// `"personality"`, `"context"`. Validated at parse time.
    #[serde(rename = "type")]
    facet_type: String,
    /// Short key identifying the attribute (e.g. `"full_name"`).
    key: String,
    /// Value to upsert.
    value: String,
    /// Confidence in the fact, clamped to `[0.0, 1.0]`. Defaults to `0.8`.
    #[serde(default)]
    confidence: Option<f64>,
}

/// An optional rich document attached to an owner update.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsOwnerDoc {
    /// Display title (e.g. `"Gmail signature"`).
    title: String,
    /// Body text.
    content: String,
    /// Optional source type tag; defaults to `"doc"`.
    #[serde(default)]
    source_type: Option<String>,
}

/// Parse a facet-type string from JS into the enum, rejecting unknown
/// values so typos fail loud instead of silently falling back to
/// `Preference`.
fn parse_facet_type(raw: &str) -> Result<FacetType, String> {
    match raw.to_ascii_lowercase().as_str() {
        "identity" => Ok(FacetType::Identity),
        "preference" => Ok(FacetType::Preference),
        "skill" => Ok(FacetType::Skill),
        "role" => Ok(FacetType::Role),
        "personality" => Ok(FacetType::Personality),
        "context" => Ok(FacetType::Context),
        other => Err(format!(
            "unknown facet type '{other}' (expected one of: identity, preference, skill, role, personality, context)"
        )),
    }
}

/// Registers state and memory operations onto the provided JavaScript object.
pub fn register<'js>(
    ctx: &Ctx<'js>,
    ops: &Object<'js>,
    skill_state: Arc<RwLock<SkillState>>,
    skill_context: SkillContext,
) -> rquickjs::Result<()> {
    // ========================================================================
    // State Bridge
    // Allows skills to get and set shared state.
    // ========================================================================

    {
        let ss = skill_state.clone();
        ops.set(
            "state_get",
            Function::new(
                ctx.clone(),
                move |key: String| -> rquickjs::Result<String> {
                    let state = ss.read();
                    let value = state
                        .data
                        .get(&key)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    serde_json::to_string(&value).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let ss = skill_state.clone();
        ops.set(
            "state_set",
            Function::new(
                ctx.clone(),
                move |key: String, value_json: String| -> rquickjs::Result<()> {
                    let value: serde_json::Value =
                        serde_json::from_str(&value_json).map_err(|e| js_err(e.to_string()))?;
                    let mut state = ss.write();
                    state.data.insert(key, value);
                    state.dirty = true;
                    Ok(())
                },
            ),
        )?;
    }

    {
        let ss = skill_state;
        ops.set(
            "state_set_partial",
            Function::new(
                ctx.clone(),
                move |partial_json: String| -> rquickjs::Result<()> {
                    let partial: serde_json::Map<String, serde_json::Value> =
                        serde_json::from_str(&partial_json).map_err(|e| js_err(e.to_string()))?;
                    let mut state = ss.write();
                    for (k, v) in partial {
                        state.data.insert(k, v);
                    }
                    state.dirty = true;
                    Ok(())
                },
            ),
        )?;
    }

    // ========================================================================
    // Data Bridge
    // Direct filesystem access within the skill's data directory.
    // ========================================================================

    {
        let sc = skill_context.clone();
        ops.set(
            "data_read",
            Function::new(
                ctx.clone(),
                move |filename: String| -> rquickjs::Result<String> {
                    let path = sc.data_dir.join(&filename);
                    std::fs::read_to_string(&path).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    {
        let sc = skill_context.clone();
        ops.set(
            "data_write",
            Function::new(
                ctx.clone(),
                move |filename: String, content: String| -> rquickjs::Result<()> {
                    let path = sc.data_dir.join(&filename);
                    std::fs::write(&path, content).map_err(|e| js_err(e.to_string()))
                },
            ),
        )?;
    }

    // ========================================================================
    // Memory Bridge
    // Integration with the OpenHuman memory system.
    // ========================================================================

    {
        let sc = skill_context.clone();
        ops.set(
            "memory_insert",
            Function::new(
                ctx.clone(),
                move |metadata_json: String| -> rquickjs::Result<()> {
                    let input: JsMemoryInsertInput =
                        serde_json::from_str(&metadata_json).map_err(|e| js_err(e.to_string()))?;

                    // Basic validation
                    if input.title.trim().is_empty() {
                        return Err(js_err("memory.insert requires a non-empty title"));
                    }
                    if input.content.trim().is_empty() {
                        return Err(js_err("memory.insert requires non-empty content"));
                    }

                    let client = sc
                        .memory_client
                        .clone()
                        .ok_or_else(|| js_err("Memory client is not initialized"))?;

                    let skill_id = sc.skill_id.clone();
                    let namespace = format!("skill-{skill_id}");

                    let source_type = input
                        .source_type
                        .unwrap_or_else(|| "doc".to_string())
                        .to_ascii_lowercase();
                    if !matches!(source_type.as_str(), "doc" | "chat" | "email") {
                        return Err(js_err("sourceType must be one of: doc, chat, email"));
                    }

                    let priority = input
                        .priority
                        .unwrap_or_else(|| "medium".to_string())
                        .to_ascii_lowercase();
                    if !matches!(priority.as_str(), "high" | "medium" | "low") {
                        return Err(js_err("priority must be one of: high, medium, low"));
                    }

                    let metadata = input.metadata.unwrap_or_else(|| serde_json::json!({}));

                    // Spawn the memory insertion in the background to avoid blocking the JS loop
                    tokio::spawn(async move {
                        if let Err(e) = client
                            .put_doc(NamespaceDocumentInput {
                                namespace,
                                key: input.title.clone(),
                                title: input.title.clone(),
                                content: input.content.clone(),
                                source_type,
                                priority,
                                tags: Vec::new(),
                                metadata,
                                category: "core".to_string(),
                                session_id: None,
                                document_id: input.document_id,
                            })
                            .await
                        {
                            log::warn!("[quickjs] memory_insert failed for '{}': {}", skill_id, e);
                        } else {
                            log::info!(
                                "[quickjs] memory_insert stored '{}': title='{}'",
                                skill_id,
                                input.title
                            );
                        }
                    });

                    Ok(())
                },
            ),
        )?;
    }

    // ========================================================================
    // Owner Identity Bridge
    // memory.updateOwner — skills push facts and rich docs about the owner.
    // Writes funnel through MemoryClient::profile_upsert_owner (for facts)
    // and MemoryClient::store_owner_doc (for docs) so skills, the learning
    // hook, and the discovery agent all share one storage path.
    // ========================================================================

    {
        let sc = skill_context;
        ops.set(
            "memory_update_owner",
            Function::new(
                ctx.clone(),
                move |payload_json: String| -> rquickjs::Result<()> {
                    let input: JsOwnerUpdateInput = serde_json::from_str(&payload_json)
                        .map_err(|e| js_err(format!("updateOwner: invalid JSON: {e}")))?;

                    if input.facts.is_empty() && input.document.is_none() {
                        return Err(js_err(
                            "memory.updateOwner requires at least one fact or a document",
                        ));
                    }

                    // Synchronously parse + validate every fact so skill
                    // authors get immediate feedback. The actual writes are
                    // deferred to a spawned task below.
                    let mut parsed_facts: Vec<(FacetType, String, String, Option<f64>)> =
                        Vec::with_capacity(input.facts.len());
                    for (idx, fact) in input.facts.iter().enumerate() {
                        let facet_type = parse_facet_type(&fact.facet_type)
                            .map_err(|e| js_err(format!("updateOwner: facts[{idx}]: {e}")))?;
                        if fact.key.trim().is_empty() {
                            return Err(js_err(format!(
                                "updateOwner: facts[{idx}]: key must be non-empty"
                            )));
                        }
                        if fact.value.trim().is_empty() {
                            return Err(js_err(format!(
                                "updateOwner: facts[{idx}]: value must be non-empty"
                            )));
                        }
                        parsed_facts.push((
                            facet_type,
                            fact.key.clone(),
                            fact.value.clone(),
                            fact.confidence,
                        ));
                    }

                    let doc = input.document;
                    if let Some(ref d) = doc {
                        if d.title.trim().is_empty() {
                            return Err(js_err(
                                "updateOwner: document.title must be non-empty",
                            ));
                        }
                        if d.content.trim().is_empty() {
                            return Err(js_err(
                                "updateOwner: document.content must be non-empty",
                            ));
                        }
                    }

                    let client = sc
                        .memory_client
                        .clone()
                        .ok_or_else(|| js_err("Memory client is not initialized"))?;

                    let skill_id = sc.skill_id.clone();
                    let origin = format!("skill-owner-{skill_id}");
                    let fact_count = parsed_facts.len();
                    let has_doc = doc.is_some();

                    // Fire-and-forget, matching memory_insert's pattern.
                    // profile_upsert_owner is synchronous (briefly holds a
                    // SQLite mutex); store_owner_doc is async.
                    tokio::spawn(async move {
                        let mut fact_errors = 0usize;
                        for (facet_type, key, value, confidence) in parsed_facts {
                            if let Err(e) = client.profile_upsert_owner(
                                facet_type.clone(),
                                &key,
                                &value,
                                confidence,
                                &origin,
                            ) {
                                fact_errors += 1;
                                log::warn!(
                                    "[quickjs] updateOwner fact failed skill='{}' type={} key='{}': {}",
                                    skill_id,
                                    facet_type.as_str(),
                                    key,
                                    e
                                );
                            }
                        }

                        if let Some(d) = doc {
                            if let Err(e) = client
                                .store_owner_doc(&d.title, &d.content, d.source_type, &origin)
                                .await
                            {
                                log::warn!(
                                    "[quickjs] updateOwner doc failed skill='{}' title='{}': {}",
                                    skill_id,
                                    d.title,
                                    e
                                );
                            }
                        }

                        log::info!(
                            "[quickjs] updateOwner skill='{}' facts={} fact_errors={} doc={}",
                            skill_id,
                            fact_count,
                            fact_errors,
                            has_doc
                        );
                    });

                    Ok(())
                },
            ),
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    //! Unit tests for the parts of the update-owner op that don't require
    //! a live QuickJS context or MemoryClient — payload parsing and facet
    //! validation. The full end-to-end path is exercised in
    //! `tests/skills_sync_memory_test.rs`-style integration tests.

    use super::*;

    #[test]
    fn parse_facet_type_accepts_all_known_variants() {
        assert!(matches!(
            parse_facet_type("identity"),
            Ok(FacetType::Identity)
        ));
        assert!(matches!(
            parse_facet_type("IDENTITY"),
            Ok(FacetType::Identity)
        ));
        assert!(matches!(
            parse_facet_type("preference"),
            Ok(FacetType::Preference)
        ));
        assert!(matches!(parse_facet_type("skill"), Ok(FacetType::Skill)));
        assert!(matches!(parse_facet_type("role"), Ok(FacetType::Role)));
        assert!(matches!(
            parse_facet_type("personality"),
            Ok(FacetType::Personality)
        ));
        assert!(matches!(
            parse_facet_type("context"),
            Ok(FacetType::Context)
        ));
    }

    #[test]
    fn parse_facet_type_rejects_unknown() {
        let err = parse_facet_type("favorite_color").unwrap_err();
        assert!(err.contains("unknown facet type"));
        assert!(err.contains("favorite_color"));
    }

    #[test]
    fn owner_payload_deserialises_facts_and_document() {
        let json = r#"{
            "facts": [
                {"type": "identity", "key": "full_name", "value": "Ada Lovelace"},
                {"type": "role", "key": "title", "value": "Principal Engineer", "confidence": 0.9}
            ],
            "document": {
                "title": "Gmail signature",
                "content": "Ada Lovelace — Principal Engineer",
                "sourceType": "doc"
            }
        }"#;
        let parsed: JsOwnerUpdateInput = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.facts.len(), 2);
        assert_eq!(parsed.facts[0].facet_type, "identity");
        assert_eq!(parsed.facts[0].key, "full_name");
        assert_eq!(parsed.facts[0].confidence, None);
        assert_eq!(parsed.facts[1].confidence, Some(0.9));
        let doc = parsed.document.expect("document should be present");
        assert_eq!(doc.title, "Gmail signature");
        assert_eq!(doc.source_type.as_deref(), Some("doc"));
    }

    #[test]
    fn owner_payload_allows_facts_only() {
        let json = r#"{
            "facts": [{"type": "identity", "key": "email", "value": "ada@example.com"}]
        }"#;
        let parsed: JsOwnerUpdateInput = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.facts.len(), 1);
        assert!(parsed.document.is_none());
    }

    #[test]
    fn owner_payload_allows_document_only() {
        let json = r#"{
            "document": {"title": "Bio", "content": "Hello"}
        }"#;
        let parsed: JsOwnerUpdateInput = serde_json::from_str(json).unwrap();
        assert!(parsed.facts.is_empty());
        assert!(parsed.document.is_some());
    }
}
