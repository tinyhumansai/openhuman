//! Document parsing helpers: chunking, alias resolution, header/metadata enrichment,
//! and the top-level `parse_document` pipeline.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{json, Map, Value};

use super::ingestion_regex::{
    action_item_regex, classify_entity, email_header_regex, explicit_owner_regex,
    explicit_preference_regex, graph_fact_regex, named_email_regex, recipient_regex,
    sanitize_entity_name, sanitize_fact_text, spatial_regex, will_review_regex,
};
use super::ingestion_types::{
    ExtractedEntity, ExtractedRelation, ExtractionAccumulator, ExtractionMode, ExtractionUnit,
    MemoryIngestionConfig, ParsedIngestion, RawEntity, RawRelation, DEFAULT_CHUNK_TOKENS,
};
use crate::openhuman::memory::store::types::NamespaceDocumentInput;
use crate::openhuman::memory::UnifiedMemory;

// ── Chunking helpers ──────────────────────────────────────────────────────────

/// Splits a document into individual sentences based on punctuation and line breaks.
pub(super) fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?' | '\n') {
            let candidate = sanitize_fact_text(&current);
            if !candidate.is_empty() {
                out.push(candidate);
            }
            current.clear();
        }
    }
    let tail = sanitize_fact_text(&current);
    if !tail.is_empty() {
        out.push(tail);
    }
    let mut merged: Vec<String> = Vec::new();
    for sentence in out {
        if sentence.len() < 5 && !merged.is_empty() {
            if let Some(last) = merged.last_mut() {
                last.push(' ');
                last.push_str(&sentence);
            }
        } else {
            merged.push(sentence);
        }
    }
    if merged.is_empty() && !text.trim().is_empty() {
        merged.push(sanitize_fact_text(text));
    }
    merged
}

/// Groups chunks into extraction units based on the configured mode.
pub(super) fn build_units(chunks: &[String], mode: ExtractionMode) -> Vec<ExtractionUnit> {
    let mut units = Vec::new();
    let mut order_index = 0_i64;
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        match mode {
            ExtractionMode::Chunk => {
                let text = sanitize_fact_text(chunk);
                if text.is_empty() {
                    continue;
                }
                units.push(ExtractionUnit {
                    text,
                    chunk_index,
                    order_index,
                });
                order_index += 1;
            }
            ExtractionMode::Sentence => {
                for sentence in split_sentences(chunk) {
                    if sentence.is_empty() {
                        continue;
                    }
                    units.push(ExtractionUnit {
                        text: sentence,
                        chunk_index,
                        order_index,
                    });
                    order_index += 1;
                }
            }
        }
    }
    units
}

/// Searches for the chunk index that most likely contains the given excerpt.
pub(super) fn find_chunk_index(chunks: &[String], excerpt: &str, hint: usize) -> usize {
    if chunks.is_empty() {
        return 0;
    }
    let needle = UnifiedMemory::normalize_search_text(excerpt);
    if needle.is_empty() {
        return hint.min(chunks.len().saturating_sub(1));
    }
    for (index, chunk) in chunks.iter().enumerate().skip(hint) {
        if UnifiedMemory::normalize_search_text(chunk).contains(&needle) {
            return index;
        }
    }
    for (index, chunk) in chunks.iter().enumerate().take(hint.min(chunks.len())) {
        if UnifiedMemory::normalize_search_text(chunk).contains(&needle) {
            return index;
        }
    }
    hint.min(chunks.len().saturating_sub(1))
}

// ── Alias resolution ──────────────────────────────────────────────────────────

pub(super) fn reverse_aliases(aliases: &HashMap<String, String>) -> BTreeMap<String, Vec<String>> {
    let mut reverse = BTreeMap::new();
    for (alias, canonical) in aliases {
        if alias == canonical {
            continue;
        }
        reverse
            .entry(canonical.clone())
            .or_insert_with(Vec::new)
            .push(alias.clone());
    }
    for values in reverse.values_mut() {
        values.sort();
        values.dedup();
    }
    reverse
}

pub(super) fn build_alias_map(entities: &HashMap<String, RawEntity>) -> HashMap<String, String> {
    let mut by_type = HashMap::<String, Vec<String>>::new();
    for entity in entities.values() {
        by_type
            .entry(entity.entity_type.clone())
            .or_default()
            .push(entity.name.clone());
    }

    let mut aliases = HashMap::new();
    for names in by_type.values_mut() {
        names.sort_by_key(|name| std::cmp::Reverse(name.len()));
        for short in names.iter() {
            for long in names.iter() {
                if short == long || long.len() <= short.len() {
                    continue;
                }
                if long.starts_with(&format!("{short} ")) || long.ends_with(&format!(" {short}")) {
                    aliases.entry(short.clone()).or_insert_with(|| long.clone());
                    break;
                }
            }
        }
    }
    aliases
}

pub(super) fn resolve_alias(name: &str, aliases: &HashMap<String, String>) -> String {
    let mut current = name.to_string();
    let mut seen = BTreeSet::new();
    while let Some(next) = aliases.get(&current) {
        if !seen.insert(current.clone()) {
            break;
        }
        current = next.clone();
    }
    current
}

// ── Header / metadata helpers ─────────────────────────────────────────────────

pub(super) fn extract_people_from_header(
    value: &str,
    accumulator: &mut ExtractionAccumulator,
) -> Vec<String> {
    let mut people = Vec::new();
    for captures in named_email_regex().captures_iter(value) {
        let name = sanitize_fact_text(
            captures
                .name("name")
                .map(|value| value.as_str())
                .unwrap_or(""),
        );
        if name.is_empty() {
            continue;
        }
        let canonical = sanitize_entity_name(&name);
        let _ = accumulator.add_entity(&canonical, "PERSON", 0.95);
        accumulator.remember_person_aliases(&canonical);
        people.push(canonical);
    }
    people
}

pub(super) fn detect_primary_subject(text: &str) -> Option<String> {
    if text.contains("OpenHuman") {
        return Some("OPENHUMAN".to_string());
    }
    None
}

pub(super) fn enrich_document_metadata(
    input: &NamespaceDocumentInput,
    parsed: &ParsedIngestion,
    config: &MemoryIngestionConfig,
) -> (NamespaceDocumentInput, Vec<String>) {
    let mut metadata = match input.metadata.clone() {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    for (key, value) in parsed.metadata.as_object().cloned().unwrap_or_default() {
        metadata.insert(key, value);
    }
    metadata.insert(
        "ingestion".to_string(),
        json!({
            "backend": "openhuman_rust_heuristic",
            "model_name": config.model_name,
            "extraction_mode": config.extraction_mode.as_str(),
            "entity_count": parsed.entities.len(),
            "relation_count": parsed.relations.len(),
            "preference_count": parsed.preference_count,
            "decision_count": parsed.decision_count,
            "chunk_count": parsed.chunk_count,
        }),
    );
    if parsed.preference_count > 0 || parsed.decision_count > 0 {
        metadata.insert("kind".to_string(), json!("profile"));
    }

    let mut tags = input.tags.iter().cloned().collect::<BTreeSet<_>>();
    tags.extend(parsed.tags.iter().cloned());
    let tags = tags.into_iter().collect::<Vec<_>>();

    (
        NamespaceDocumentInput {
            namespace: input.namespace.clone(),
            key: input.key.clone(),
            title: input.title.clone(),
            content: input.content.clone(),
            source_type: input.source_type.clone(),
            priority: input.priority.clone(),
            tags: tags.clone(),
            metadata: Value::Object(metadata),
            category: input.category.clone(),
            session_id: input.session_id.clone(),
            document_id: input.document_id.clone(),
        },
        tags,
    )
}

// ── Top-level document parser ─────────────────────────────────────────────────

pub(super) async fn parse_document(
    content: &str,
    title: &str,
    config: &MemoryIngestionConfig,
) -> ParsedIngestion {
    let chunks = UnifiedMemory::chunk_document_content(content, DEFAULT_CHUNK_TOKENS);
    log::info!(
        "[memory:ingestion] parse_document title={title:?} model={} \
         content_len={} chunk_count={} — heuristic extraction active",
        config.model_name,
        content.len(),
        chunks.len(),
    );
    let mut accumulator = ExtractionAccumulator {
        document_title: Some(sanitize_entity_name(title)),
        primary_subject: detect_primary_subject(title),
        ..ExtractionAccumulator::default()
    };

    let mut chunk_hint = 0_usize;
    for raw_line in content.lines() {
        let line = sanitize_fact_text(raw_line);
        if line.is_empty() {
            continue;
        }

        let chunk_index = find_chunk_index(&chunks, &line, chunk_hint);
        chunk_hint = chunk_index;
        let order_index = i64::try_from(chunk_index).unwrap_or(i64::MAX);

        if raw_line.trim_start().starts_with('#') {
            let heading = sanitize_entity_name(raw_line.trim_start_matches('#'));
            if !heading.is_empty() {
                if accumulator.document_title.is_none() {
                    accumulator.document_title = Some(heading.clone());
                }
                accumulator.current_subject = Some(heading);
            }
            continue;
        }

        if let Some(captures) = email_header_regex().captures(&line) {
            let header_name = captures
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default()
                .to_ascii_uppercase();
            let value = captures
                .name("value")
                .map(|value| value.as_str())
                .unwrap_or("");
            let people = extract_people_from_header(value, &mut accumulator);
            if header_name == "FROM" {
                accumulator.current_sender = people.first().cloned();
            } else if header_name == "TO" || header_name == "CC" {
                if let Some(sender) = accumulator.current_sender.clone() {
                    for recipient in &people {
                        accumulator.add_relation(
                            &sender,
                            "PERSON",
                            "communicates_with",
                            recipient,
                            "PERSON",
                            0.82,
                            chunk_index,
                            order_index,
                            Map::new(),
                        );
                    }
                }
            }
            continue;
        }

        if let Some(subject) = line.strip_prefix("Subject:") {
            let subject_text = sanitize_fact_text(subject);
            if let Some(primary_subject) = detect_primary_subject(&subject_text) {
                accumulator.primary_subject = Some(primary_subject);
            }
            continue;
        }

        if let Some(date_text) = line.strip_prefix("Date:") {
            let date_text = sanitize_fact_text(date_text);
            if let Some(sender) = accumulator.current_sender.clone() {
                accumulator.add_relation(
                    &sender,
                    "PERSON",
                    "has_deadline",
                    &date_text,
                    "DATE",
                    0.75,
                    chunk_index,
                    order_index,
                    Map::new(),
                );
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("Project name:") {
            let project = sanitize_entity_name(value);
            if !project.is_empty() {
                accumulator.primary_subject = Some(project.clone());
                let _ = accumulator.add_entity(&project, "PROJECT", 0.96);
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("Subproject:") {
            let subproject = sanitize_entity_name(value);
            if !subproject.is_empty() {
                let _ = accumulator.add_entity(&subproject, "PROJECT", 0.92);
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("Owner:") {
            let owner = sanitize_entity_name(value);
            let owned = accumulator
                .current_subject
                .clone()
                .or_else(|| accumulator.primary_subject.clone())
                .or_else(|| accumulator.document_title.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &owner,
                "PERSON",
                "owns",
                &owned,
                "WORK_ITEM",
                0.94,
                chunk_index,
                order_index,
                Map::new(),
            );
            continue;
        }

        if let Some(value) = line.strip_prefix("Name:") {
            let name = sanitize_entity_name(value);
            if !name.is_empty() {
                accumulator.current_subject = Some(name.clone());
                let _ = accumulator.add_entity(&name, "WORK_ITEM", 0.93);
            }
            continue;
        }

        if let Some(value) = line.strip_prefix("Due date:") {
            let due_date = sanitize_fact_text(value);
            let subject = accumulator
                .current_subject
                .clone()
                .or_else(|| accumulator.primary_subject.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &subject,
                "WORK_ITEM",
                "has_deadline",
                &due_date,
                "DATE",
                0.92,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.tags.insert("deadline".to_string());
            continue;
        }

        if let Some(value) = line.strip_prefix("Target milestone:") {
            let due_date = sanitize_fact_text(value);
            let subject = accumulator
                .primary_subject
                .clone()
                .or_else(|| accumulator.document_title.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &subject,
                "PROJECT",
                "has_deadline",
                &due_date,
                "DATE",
                0.92,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.tags.insert("deadline".to_string());
            continue;
        }

        if let Some(value) = line.strip_prefix("Preferred embedding model for local experiments:") {
            let model = sanitize_fact_text(value);
            let subject = accumulator
                .primary_subject
                .clone()
                .or_else(|| accumulator.document_title.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &subject,
                "PROJECT",
                "uses",
                &model,
                "TOOL",
                0.88,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{subject} uses {model}"));
            accumulator.tags.insert("decision".to_string());
            continue;
        }

        if let Some(value) = line.strip_prefix("Preferred extraction mode to try first:") {
            let mode = sanitize_fact_text(value);
            let subject = accumulator
                .primary_subject
                .clone()
                .or_else(|| accumulator.document_title.clone())
                .unwrap_or_else(|| "DOCUMENT".to_string());
            accumulator.add_relation(
                &subject,
                "PROJECT",
                "uses",
                &mode,
                "MODE",
                0.88,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{subject} uses {mode}"));
            accumulator.tags.insert("decision".to_string());
            continue;
        }

        if let Some(captures) = graph_fact_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let predicate = captures
                .name("predicate")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            let subject_type = classify_entity(subject, &accumulator.known_people);
            let object_type = classify_entity(object, &accumulator.known_people);
            accumulator.add_relation(
                subject,
                subject_type,
                predicate,
                object,
                object_type,
                0.87,
                chunk_index,
                order_index,
                Map::new(),
            );
            if UnifiedMemory::normalize_graph_predicate(predicate) == "PREFERS" {
                accumulator.preferences.insert(format!(
                    "{} prefers {}",
                    sanitize_entity_name(subject),
                    sanitize_fact_text(object)
                ));
                accumulator.tags.insert("preference".to_string());
                accumulator.doc_kind = Some("profile".to_string());
            }
            continue;
        }

        if let Some(captures) = explicit_owner_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            accumulator.add_relation(
                subject,
                "PERSON",
                "owns",
                object,
                classify_entity(object, &accumulator.known_people),
                0.94,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.tags.insert("owner".to_string());
            continue;
        }

        if let Some(captures) = will_review_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            accumulator.add_relation(
                subject,
                "PERSON",
                "reviews",
                object,
                classify_entity(object, &accumulator.known_people),
                0.80,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.tags.insert("owner".to_string());
            continue;
        }

        if let Some(captures) = explicit_preference_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            accumulator.add_relation(
                subject,
                "PERSON",
                "prefers",
                object,
                classify_entity(object, &accumulator.known_people),
                0.90,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator.preferences.insert(format!(
                "{} prefers {}",
                sanitize_entity_name(subject),
                sanitize_fact_text(object)
            ));
            accumulator.tags.insert("preference".to_string());
            accumulator.doc_kind = Some("profile".to_string());
            continue;
        }

        if let Some(value) = line.strip_prefix("I prefer ") {
            if let Some(subject) = accumulator.current_sender.clone() {
                let preference = sanitize_fact_text(value);
                accumulator.add_relation(
                    &subject,
                    "PERSON",
                    "prefers",
                    &preference,
                    classify_entity(&preference, &accumulator.known_people),
                    0.92,
                    chunk_index,
                    order_index,
                    Map::new(),
                );
                accumulator
                    .preferences
                    .insert(format!("{subject} prefers {preference}"));
                accumulator.tags.insert("preference".to_string());
                accumulator.doc_kind = Some("profile".to_string());
                continue;
            }
        }

        if let Some(captures) = action_item_regex().captures(&line) {
            let subject = captures
                .name("subject")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            if accumulator
                .known_people
                .contains_key(&sanitize_entity_name(subject))
                || classify_entity(subject, &accumulator.known_people) == "PERSON"
            {
                accumulator.add_relation(
                    subject,
                    "PERSON",
                    "owns",
                    object,
                    classify_entity(object, &accumulator.known_people),
                    0.83,
                    chunk_index,
                    order_index,
                    Map::new(),
                );
                accumulator.tags.insert("owner".to_string());
                continue;
            }
        }

        let upper = sanitize_entity_name(&line);
        let decision_subject = accumulator
            .primary_subject
            .clone()
            .or_else(|| accumulator.document_title.clone())
            .unwrap_or_else(|| "DOCUMENT".to_string());
        if upper.contains("JSON-RPC") {
            accumulator.add_relation(
                &decision_subject,
                "PROJECT",
                "uses",
                "JSON-RPC",
                "PRODUCT",
                0.86,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{decision_subject} uses JSON-RPC"));
            accumulator.tags.insert("decision".to_string());
            continue;
        }
        if upper.contains("SHOULD USE NAMESPACE")
            || upper.contains("USE NAMESPACE AS THE STORAGE")
            || upper.contains("NAMESPACE AS THE MAIN SCOPE KEY")
        {
            accumulator.add_relation(
                &decision_subject,
                "PROJECT",
                "uses",
                "namespace",
                "TOPIC",
                0.84,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{decision_subject} uses namespace"));
            accumulator.tags.insert("decision".to_string());
            continue;
        }
        if upper.contains("USER_ID") && (upper.contains("DO NOT NEED") || upper.contains("AVOID")) {
            accumulator.add_relation(
                &decision_subject,
                "PROJECT",
                "avoids",
                "user_id",
                "TOPIC",
                0.82,
                chunk_index,
                order_index,
                Map::new(),
            );
            accumulator
                .decisions
                .insert(format!("{decision_subject} avoids user_id"));
            accumulator.tags.insert("decision".to_string());
        }
    }

    for unit in build_units(&chunks, config.extraction_mode) {
        if let Some(captures) = recipient_regex().captures(&unit.text) {
            let giver = captures
                .name("giver")
                .map(|value| value.as_str())
                .unwrap_or("");
            let object = captures
                .name("object")
                .map(|value| value.as_str())
                .unwrap_or("");
            let recipient = captures
                .name("recipient")
                .map(|value| value.as_str())
                .unwrap_or("");
            accumulator.add_relation(
                giver,
                "PERSON",
                "uses",
                object,
                classify_entity(object, &accumulator.known_people),
                config.adjacency_threshold.max(0.62),
                unit.chunk_index,
                unit.order_index,
                Map::new(),
            );
            accumulator.add_relation(
                recipient,
                "PERSON",
                "uses",
                object,
                classify_entity(object, &accumulator.known_people),
                (config.adjacency_threshold * 0.9).max(0.55),
                unit.chunk_index,
                unit.order_index,
                Map::new(),
            );
        }

        if let Some(captures) = spatial_regex().captures(&unit.text) {
            let head = captures
                .name("head")
                .map(|value| value.as_str())
                .unwrap_or("");
            let direction = captures
                .name("direction")
                .map(|value| value.as_str())
                .unwrap_or("");
            let tail = captures
                .name("tail")
                .map(|value| value.as_str())
                .unwrap_or("");
            let inverse = match direction.to_ascii_lowercase().as_str() {
                "north" => "south_of",
                "south" => "north_of",
                "east" => "west_of",
                "west" => "east_of",
                _ => "",
            };
            let predicate = format!("{direction}_of");
            accumulator.add_relation(
                head,
                "ROOM",
                &predicate,
                tail,
                "ROOM",
                config.adjacency_threshold.max(0.70),
                unit.chunk_index,
                unit.order_index,
                Map::new(),
            );
            if !inverse.is_empty() {
                accumulator.add_relation(
                    tail,
                    "ROOM",
                    inverse,
                    head,
                    "ROOM",
                    config.adjacency_threshold.max(0.70),
                    unit.chunk_index,
                    unit.order_index,
                    Map::new(),
                );
            }
        }
    }

    let aliases = build_alias_map(&accumulator.entities);
    let reverse_alias = reverse_aliases(&aliases);
    let mut canonical_entities = BTreeMap::<String, RawEntity>::new();
    for entity in accumulator.entities.values() {
        let canonical = resolve_alias(&entity.name, &aliases);
        let entry = canonical_entities
            .entry(canonical.clone())
            .or_insert_with(|| RawEntity {
                name: canonical.clone(),
                entity_type: entity.entity_type.clone(),
                confidence: entity.confidence,
            });
        if entity.confidence > entry.confidence {
            entry.confidence = entity.confidence;
            entry.entity_type = entity.entity_type.clone();
        }
    }

    let mut aggregated_relations = BTreeMap::<(String, String, String), _>::new();
    for relation in accumulator.relations {
        let subject = resolve_alias(&relation.subject, &aliases);
        let object = resolve_alias(&relation.object, &aliases);
        if subject == object {
            continue;
        }
        let key = (subject.clone(), relation.predicate.clone(), object.clone());
        let entry = aggregated_relations
            .entry(key)
            .or_insert_with(|| RawRelation {
                subject,
                subject_type: relation.subject_type.clone(),
                predicate: relation.predicate.clone(),
                object,
                object_type: relation.object_type.clone(),
                confidence: relation.confidence,
                chunk_indexes: relation.chunk_indexes.clone(),
                order_index: relation.order_index,
                metadata: relation.metadata.clone(),
            });
        entry.confidence = entry.confidence.max(relation.confidence);
        entry.order_index = entry.order_index.min(relation.order_index);
        entry.chunk_indexes.extend(relation.chunk_indexes);
    }

    let entities = canonical_entities
        .into_values()
        .filter(|entity| entity.confidence >= config.entity_threshold)
        .map(|entity| ExtractedEntity {
            name: entity.name.clone(),
            entity_type: entity.entity_type,
            aliases: reverse_alias.get(&entity.name).cloned().unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    let relations = aggregated_relations
        .into_values()
        .filter(|relation| relation.confidence >= config.relation_threshold)
        .map(|relation| ExtractedRelation {
            subject: relation.subject,
            subject_type: relation.subject_type,
            predicate: relation.predicate,
            object: relation.object,
            object_type: relation.object_type,
            confidence: relation.confidence,
            evidence_count: u32::try_from(relation.chunk_indexes.len()).unwrap_or(u32::MAX),
            chunk_ids: relation
                .chunk_indexes
                .iter()
                .map(|index| format!("chunk:{index}"))
                .collect::<Vec<_>>(),
            order_index: Some(relation.order_index),
            metadata: Value::Object(relation.metadata),
        })
        .collect::<Vec<_>>();

    let mut tags = accumulator.tags.into_iter().collect::<Vec<_>>();
    tags.sort();
    let metadata = json!({
        "kind": accumulator.doc_kind.or_else(|| {
            if !accumulator.preferences.is_empty() || !accumulator.decisions.is_empty() {
                Some("profile".to_string())
            } else {
                None
            }
        }),
        "primary_subject": accumulator.primary_subject,
        "decisions": accumulator.decisions.iter().cloned().collect::<Vec<_>>(),
        "preferences": accumulator.preferences.iter().cloned().collect::<Vec<_>>(),
        "extracted_entities": entities.iter().map(|entity| {
            json!({
                "name": entity.name,
                "entity_type": entity.entity_type,
                "aliases": entity.aliases,
            })
        }).collect::<Vec<_>>(),
    });

    log::debug!(
        "[memory:ingestion] parse_document complete title={title:?} \
         entities={} relations={} preferences={} decisions={}",
        entities.len(),
        relations.len(),
        accumulator.preferences.len(),
        accumulator.decisions.len(),
    );

    ParsedIngestion {
        tags,
        metadata,
        entities,
        relations,
        chunk_count: chunks.len(),
        preference_count: accumulator.preferences.len(),
        decision_count: accumulator.decisions.len(),
    }
}
