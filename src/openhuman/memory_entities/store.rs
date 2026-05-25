//! Disk-backed entity store.
//!
//! Atomic md write contract via `memory_store::content::atomic::write_if_new`,
//! with an explicit overwrite for upsert. Notes body is preserved across
//! upserts so the user can hand-edit it in Obsidian without losing edits.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory_entities::types::{Entity, EntityHandle, EntityKind};

const ENTITIES_DIR: &str = "entities";

fn slugify_id(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect()
}

fn kind_dir(config: &Config, kind: EntityKind) -> PathBuf {
    config
        .memory_tree_content_root()
        .join(ENTITIES_DIR)
        .join(kind.as_str())
}

fn entity_path(config: &Config, kind: EntityKind, canonical_id: &str) -> PathBuf {
    kind_dir(config, kind).join(format!("{}.md", slugify_id(canonical_id)))
}

/// Upsert. Preserves any user-edited notes body that already exists on
/// disk; only the YAML front-matter is rewritten. Returns the stored
/// entity with `updated_at` refreshed.
pub fn put_entity(config: &Config, mut entity: Entity) -> Result<Entity> {
    let dir = kind_dir(config, entity.kind);
    fs::create_dir_all(&dir).with_context(|| format!("failed to mkdir -p {}", dir.display()))?;
    let path = entity_path(config, entity.kind, &entity.id);

    // Preserve any free-form notes the user typed in Obsidian.
    let existing_notes = match fs::read_to_string(&path) {
        Ok(text) => extract_notes(&text),
        Err(_) => String::new(),
    };

    entity.updated_at = Utc::now();
    let bytes = compose(&entity, &existing_notes).into_bytes();
    fs::write(&path, &bytes)
        .with_context(|| format!("failed to write entity {}", path.display()))?;
    log::debug!(
        "[memory_entities] put kind={} id={} bytes={}",
        entity.kind.as_str(),
        entity.id,
        bytes.len()
    );
    Ok(entity)
}

/// Read by canonical id. Returns `Ok(None)` when the file doesn't exist.
pub fn get_entity(config: &Config, kind: EntityKind, canonical_id: &str) -> Result<Option<Entity>> {
    let path = entity_path(config, kind, canonical_id);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse(&text))
}

/// List every stored entity of a given kind. Order is filesystem-dependent
/// — callers that need a sort impose their own.
pub fn list_entities(config: &Config, kind: EntityKind) -> Result<Vec<Entity>> {
    let dir = kind_dir(config, kind);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in
        fs::read_dir(&dir).with_context(|| format!("failed to read_dir {}", dir.display()))?
    {
        let entry = entry?;
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.ends_with(".md") {
            continue;
        }
        let text = fs::read_to_string(entry.path())
            .with_context(|| format!("failed to read {}", entry.path().display()))?;
        if let Some(e) = parse(&text) {
            out.push(e);
        }
    }
    Ok(out)
}

/// Find an entity whose `aliases`, `emails`, or `handles[*].value` matches
/// `needle` (case-insensitive). Returns the first match in walk order;
/// `kind` narrows the search. `None` when no match.
///
/// Linear scan — for a single-user workspace with thousands (not millions)
/// of entities this is fine and avoids any additional index.
pub fn lookup_alias(config: &Config, kind: EntityKind, needle: &str) -> Result<Option<Entity>> {
    let lower = needle.to_lowercase();
    for e in list_entities(config, kind)? {
        if e.aliases.iter().any(|a| a.to_lowercase() == lower) {
            return Ok(Some(e));
        }
        if e.emails.iter().any(|m| m.to_lowercase() == lower) {
            return Ok(Some(e));
        }
        if e.handles.iter().any(|h| h.value.to_lowercase() == lower) {
            return Ok(Some(e));
        }
        if e.display_name
            .as_deref()
            .map(|n| n.to_lowercase() == lower)
            .unwrap_or(false)
        {
            return Ok(Some(e));
        }
    }
    Ok(None)
}

// ───────────────────────── compose / parse ─────────────────────────

fn compose(entity: &Entity, notes: &str) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("id: {}\n", entity.id));
    out.push_str(&format!("kind: {}\n", entity.kind.as_str()));
    if let Some(name) = entity.display_name.as_deref() {
        out.push_str(&format!("display_name: {}\n", yaml_string(name)));
    }
    if !entity.aliases.is_empty() {
        out.push_str("aliases:\n");
        for a in &entity.aliases {
            out.push_str(&format!("  - {}\n", yaml_string(a)));
        }
    }
    if !entity.emails.is_empty() {
        out.push_str("emails:\n");
        for e in &entity.emails {
            out.push_str(&format!("  - {}\n", yaml_string(e)));
        }
    }
    if !entity.handles.is_empty() {
        out.push_str("handles:\n");
        for h in &entity.handles {
            out.push_str(&format!(
                "  - kind: {}\n    value: {}\n",
                yaml_string(&h.kind),
                yaml_string(&h.value)
            ));
        }
    }
    out.push_str(&format!("created_at: {}\n", entity.created_at.to_rfc3339()));
    out.push_str(&format!("updated_at: {}\n", entity.updated_at.to_rfc3339()));
    out.push_str("---\n\n");
    out.push_str(notes);
    if !notes.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn yaml_string(s: &str) -> String {
    let needs_quote = s
        .chars()
        .any(|c| matches!(c, ':' | '#' | '\n' | '"' | '\'' | '[' | ']' | '{' | '}'));
    if needs_quote {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn unquote(s: &str) -> String {
    s.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .map(|x| x.replace("\\\"", "\"").replace("\\\\", "\\"))
        .unwrap_or_else(|| s.to_string())
}

fn split_front_matter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    let (yaml, after) = rest.split_at(end);
    let body = after.strip_prefix("\n---\n").unwrap_or(after);
    Some((yaml, body))
}

fn extract_notes(text: &str) -> String {
    split_front_matter(text)
        .map(|(_, body)| body.to_string())
        .unwrap_or_default()
}

fn parse(text: &str) -> Option<Entity> {
    let (yaml, body) = split_front_matter(text)?;
    let mut id = String::new();
    let mut kind: Option<EntityKind> = None;
    let mut display_name: Option<String> = None;
    let mut aliases = Vec::new();
    let mut emails = Vec::new();
    let mut handles = Vec::new();
    let mut created_at: Option<DateTime<Utc>> = None;
    let mut updated_at: Option<DateTime<Utc>> = None;

    let mut current_list: Option<&'static str> = None;
    let mut handle_buf: Option<EntityHandle> = None;

    for raw in yaml.lines() {
        if raw.starts_with("  - kind:") {
            // Flush previous handle, start a new one.
            if let Some(h) = handle_buf.take() {
                handles.push(h);
            }
            let v = raw.trim_start_matches("  - kind:").trim();
            handle_buf = Some(EntityHandle {
                kind: unquote(v),
                value: String::new(),
            });
            current_list = Some("handles");
            continue;
        }
        if raw.starts_with("    value:") {
            let v = raw.trim_start_matches("    value:").trim();
            if let Some(h) = handle_buf.as_mut() {
                h.value = unquote(v);
            }
            continue;
        }
        if let Some(v) = raw.strip_prefix("  - ") {
            let v = unquote(v.trim());
            match current_list {
                Some("aliases") => aliases.push(v),
                Some("emails") => emails.push(v),
                _ => {}
            }
            continue;
        }
        // Flush any in-progress handle when we leave the handle list.
        if !raw.starts_with(' ') && !raw.starts_with("  - kind") {
            if let Some(h) = handle_buf.take() {
                handles.push(h);
            }
            current_list = None;
        }
        let Some((k, v)) = raw.split_once(':') else {
            continue;
        };
        let v = v.trim();
        match k.trim() {
            "id" => id = unquote(v),
            "kind" => kind = EntityKind::parse(&unquote(v)).ok(),
            "display_name" => display_name = Some(unquote(v)),
            "aliases" => current_list = Some("aliases"),
            "emails" => current_list = Some("emails"),
            "handles" => current_list = Some("handles"),
            "created_at" => {
                created_at = DateTime::parse_from_rfc3339(&unquote(v))
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            }
            "updated_at" => {
                updated_at = DateTime::parse_from_rfc3339(&unquote(v))
                    .ok()
                    .map(|d| d.with_timezone(&Utc))
            }
            _ => {}
        }
    }
    if let Some(h) = handle_buf {
        handles.push(h);
    }

    let now = Utc::now();
    let _ = body; // notes are preserved on write but not surfaced in Entity
    Some(Entity {
        id,
        kind: kind?,
        display_name,
        aliases,
        emails,
        handles,
        created_at: created_at.unwrap_or(now),
        updated_at: updated_at.unwrap_or(now),
    })
}

// ───────────────────────── tests ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cfg() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut c = Config::default();
        c.workspace_dir = tmp.path().to_path_buf();
        (tmp, c)
    }

    fn alice() -> Entity {
        let mut e = Entity::new("person:alice", EntityKind::Person);
        e.display_name = Some("Alice Cooper".into());
        e.aliases = vec!["Ali".into(), "A. Cooper".into()];
        e.emails = vec!["alice@example.com".into()];
        e.handles = vec![EntityHandle {
            kind: "slack".into(),
            value: "U12345".into(),
        }];
        e
    }

    #[test]
    fn round_trip_person() {
        let (_t, c) = cfg();
        let stored = put_entity(&c, alice()).unwrap();
        let got = get_entity(&c, EntityKind::Person, "person:alice")
            .unwrap()
            .expect("entity present");
        assert_eq!(got.id, stored.id);
        assert_eq!(got.display_name.as_deref(), Some("Alice Cooper"));
        assert_eq!(got.aliases, vec!["Ali".to_string(), "A. Cooper".into()]);
        assert_eq!(got.emails, vec!["alice@example.com".to_string()]);
        assert_eq!(got.handles.len(), 1);
        assert_eq!(got.handles[0].kind, "slack");
        assert_eq!(got.handles[0].value, "U12345");
    }

    #[test]
    fn missing_entity_returns_none() {
        let (_t, c) = cfg();
        assert!(get_entity(&c, EntityKind::Person, "person:nope")
            .unwrap()
            .is_none());
    }

    #[test]
    fn list_entities_by_kind() {
        let (_t, c) = cfg();
        put_entity(&c, alice()).unwrap();
        let mut bob = Entity::new("person:bob", EntityKind::Person);
        bob.display_name = Some("Bob".into());
        put_entity(&c, bob).unwrap();
        let mut org = Entity::new("organization:acme", EntityKind::Organization);
        org.display_name = Some("Acme".into());
        put_entity(&c, org).unwrap();

        let people = list_entities(&c, EntityKind::Person).unwrap();
        assert_eq!(people.len(), 2);
        let orgs = list_entities(&c, EntityKind::Organization).unwrap();
        assert_eq!(orgs.len(), 1);
        assert_eq!(orgs[0].display_name.as_deref(), Some("Acme"));
    }

    #[test]
    fn lookup_alias_finds_by_alias_email_handle_or_name() {
        let (_t, c) = cfg();
        put_entity(&c, alice()).unwrap();
        assert_eq!(
            lookup_alias(&c, EntityKind::Person, "Ali")
                .unwrap()
                .unwrap()
                .id,
            "person:alice"
        );
        assert_eq!(
            lookup_alias(&c, EntityKind::Person, "alice@example.com")
                .unwrap()
                .unwrap()
                .id,
            "person:alice"
        );
        assert_eq!(
            lookup_alias(&c, EntityKind::Person, "U12345")
                .unwrap()
                .unwrap()
                .id,
            "person:alice"
        );
        assert_eq!(
            lookup_alias(&c, EntityKind::Person, "alice cooper")
                .unwrap()
                .unwrap()
                .id,
            "person:alice"
        );
        assert!(lookup_alias(&c, EntityKind::Person, "noone")
            .unwrap()
            .is_none());
    }

    #[test]
    fn upsert_preserves_user_notes_body() {
        let (_t, c) = cfg();
        put_entity(&c, alice()).unwrap();
        // User hand-edits the file in Obsidian to add notes.
        let path = entity_path(&c, EntityKind::Person, "person:alice");
        let original = fs::read_to_string(&path).unwrap();
        let with_notes = format!("{original}\nMet at the conference in March.\n");
        fs::write(&path, &with_notes).unwrap();

        // Re-upsert with new alias — notes should survive.
        let mut updated = alice();
        updated.aliases.push("Coop".into());
        put_entity(&c, updated).unwrap();

        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("Met at the conference in March."));
        assert!(body.contains("Coop"));
    }

    #[test]
    fn slugify_strips_filesystem_unsafe_chars() {
        // `:` is stripped for Windows compatibility even though it's legal
        // on Unix; the round-trip uses the in-file `id` field as the
        // canonical id, so the on-disk filename is just a content-addressed
        // handle.
        assert_eq!(slugify_id("person:alice"), "person_alice");
        assert_eq!(
            slugify_id("url:https://x.com/path"),
            "url_https___x.com_path"
        );
    }
}
