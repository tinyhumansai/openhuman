//! Lazily-initialised regex patterns and text-sanitization helpers for document ingestion.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::openhuman::memory::UnifiedMemory;

/// Regex for identifying standard email headers (From, To, Cc).
pub(super) fn email_header_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"^(From|To|Cc):\s*(?P<value>.+)$").expect("email header regex"))
}

/// Regex for identifying named email addresses (e.g., "John Doe <john@example.com>").
pub(super) fn named_email_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?P<name>[^,<]+?)\s*<(?P<email>[^>]+)>").expect("named email regex")
    })
}

/// Regex for identifying explicit graph facts (e.g., "Alice works_on Project-X").
pub(super) fn graph_fact_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"^(?P<subject>[A-Za-z0-9][A-Za-z0-9 ._\-/]+?)\s+(?P<predicate>works_on|depends_on|uses|evaluates|owns|prefers)\s+(?P<object>.+)$",
        )
        .expect("graph fact regex")
    })
}

/// Regex for identifying ownership patterns (e.g., "Bob owns the repository").
pub(super) fn explicit_owner_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<subject>[A-Za-z][A-Za-z ._-]+?) owns (?P<object>.+)$")
            .expect("explicit owner regex")
    })
}

/// Regex for identifying preference patterns (e.g., "Carol prefers light mode").
pub(super) fn explicit_preference_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<subject>[A-Za-z][A-Za-z ._-]+?) prefers (?P<object>.+)$")
            .expect("explicit preference regex")
    })
}

/// Regex for identifying action items or assignments (e.g., "Dave: finish the API").
pub(super) fn action_item_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<subject>[A-Za-z][A-Za-z ._-]+?):\s*(?P<object>.+)$")
            .expect("action item regex")
    })
}

/// Regex for identifying review assignments.
pub(super) fn will_review_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^(?P<subject>[A-Za-z][A-Za-z ._-]+?) will review (?P<object>.+)$")
            .expect("will review regex")
    })
}

/// Regex for identifying complex giving/receiving interactions.
pub(super) fn recipient_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<giver>[A-Z][A-Za-z]+(?: [A-Z][A-Za-z]+)?)\s+(gave|sent|handed|passed)\s+(?P<object>.+?)\s+to\s+(?P<recipient>[A-Z][A-Za-z]+(?: [A-Z][A-Za-z]+)?)",
        )
        .expect("recipient regex")
    })
}

/// Regex for identifying spatial relationships (e.g., "Kitchen is north of the Garden").
pub(super) fn spatial_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<head>[A-Za-z][A-Za-z0-9 _-]+?)\s+is\s+(?P<direction>north|south|east|west)\s+of\s+(?P<tail>[A-Za-z][A-Za-z0-9 _-]+)",
        )
        .expect("spatial regex")
    })
}

/// Regex for identifying dates in "Month DD, YYYY" format.
pub(super) fn month_date_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Sept|Oct|Nov|Dec)[a-z]*\s+\d{1,2},\s+\d{4}\b")
            .expect("month date regex")
    })
}

/// Regex for identifying ISO-8601 dates (YYYY-MM-DD).
pub(super) fn iso_date_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\b\d{4}-\d{2}-\d{2}\b").expect("iso date regex"))
}

/// Regex for identifying potential person names (Title Case).
pub(super) fn person_name_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"\b[A-Z][a-z]+(?: [A-Z][a-z]+)+\b").expect("person name regex"))
}

/// Normalizes an entity name by trimming punctuation, collapsing whitespace, and converting to uppercase.
pub(super) fn sanitize_entity_name(name: &str) -> String {
    let trimmed = name.trim().trim_matches(|ch: char| {
        matches!(ch, '-' | ':' | ';' | ',' | '.' | '"' | '\'' | '(' | ')')
    });
    if trimmed.is_empty() {
        return String::new();
    }
    UnifiedMemory::collapse_whitespace(trimmed).to_uppercase()
}

/// Normalizes text content by trimming and collapsing whitespace.
pub(super) fn sanitize_fact_text(text: &str) -> String {
    let trimmed = text
        .trim()
        .trim_start_matches('-')
        .trim()
        .trim_matches(|ch: char| matches!(ch, ':' | ';' | ',' | '.'));
    UnifiedMemory::collapse_whitespace(trimmed)
}

/// Heuristically classifies an entity based on its name and known person map.
pub(super) fn classify_entity(name: &str, known_people: &HashMap<String, String>) -> &'static str {
    let upper = sanitize_entity_name(name);
    if upper.is_empty() {
        return "TOPIC";
    }
    if month_date_regex().is_match(name) || iso_date_regex().is_match(name) {
        return "DATE";
    }
    if upper.contains('@') {
        return "ORGANIZATION";
    }
    if known_people.contains_key(&upper) || person_name_regex().is_match(name) {
        return "PERSON";
    }
    if matches!(
        upper.as_str(),
        "OPENHUMAN" | "JSON-RPC" | "JSON-RPC 2.0" | "NEOCORTEX_V2" | "NEOCORTEX V2"
    ) {
        return "PRODUCT";
    }
    if upper.contains("MODEL") {
        return "TOOL";
    }
    if upper.contains("MODE") {
        return "MODE";
    }
    if upper.contains("MILESTONE")
        || upper.contains("ROADMAP")
        || upper.contains("CONTRACT")
        || upper.contains("API")
        || upper.contains("MEMORY")
        || upper.contains("FIXTURE")
        || upper.contains("THREAD")
        || upper.contains("WORK")
    {
        return "WORK_ITEM";
    }
    if upper.contains("OFFICE")
        || upper.contains("ROOM")
        || upper.contains("GARDEN")
        || upper.contains("KITCHEN")
    {
        return "ROOM";
    }
    if upper.contains("TINYHUMANS") || upper.ends_with("CORE") {
        return "ORGANIZATION";
    }
    if (upper.contains('-') || upper.contains('_')) && !upper.contains(' ') {
        return "PROJECT";
    }
    "TOPIC"
}
