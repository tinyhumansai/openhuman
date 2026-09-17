//! Deterministic typed request-intent data and precedence resolver for Phase 7.

use serde::{Deserialize, Serialize};

use super::capability::{CapabilityModality, CapabilityOperation};
use super::mode::PrimaryTurnMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimaryIntentFamily {
    Conversation,
    Web,
    ImageRetrieval,
    ImageGeneration,
    Memory,
    Repository,
    Scheduling,
    Delegation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentCompletion {
    FinalText,
    SourcedAnswer,
    ImageResult,
    Artifact,
    MemoryResult,
    VerifiedChange,
    ScheduleState,
    DelegatedResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestIntent {
    pub family: PrimaryIntentFamily,
    pub operations: Vec<CapabilityOperation>,
    pub modalities: Vec<CapabilityModality>,
    pub completion: IntentCompletion,
    pub known_url: Option<String>,
    pub explicit_memory: bool,
    pub explicit_generation: bool,
    pub explicit_retrieval: bool,
}

/// Deterministically resolve user request intent.
///
/// Precedence rules are evaluated in fixed order using normalized user text.
/// Mixed or ambiguous unmatched requests narrow to `Conversation` with no capabilities.
pub fn resolve_request_intent(user_message: &str, mode: PrimaryTurnMode) -> RequestIntent {
    if mode == PrimaryTurnMode::Chat {
        return RequestIntent {
            family: PrimaryIntentFamily::Conversation,
            operations: Vec::new(),
            modalities: Vec::new(),
            completion: IntentCompletion::FinalText,
            known_url: None,
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        };
    }

    let normalized = normalize(user_message);
    if normalized.is_empty() {
        return RequestIntent {
            family: PrimaryIntentFamily::Conversation,
            operations: Vec::new(),
            modalities: Vec::new(),
            completion: IntentCompletion::FinalText,
            known_url: None,
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        };
    }

    // (1) an http:// or https:// token yields Web with FetchUrl, [WebPage, Text], sourced answer, and the punctuation-trimmed URL
    if let Some(url) = extract_http_url(user_message) {
        return RequestIntent {
            family: PrimaryIntentFamily::Web,
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage, CapabilityModality::Text],
            completion: IntentCompletion::SourcedAnswer,
            known_url: Some(url),
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        };
    }

    // (2) image words plus internet/search/find/show/get or explicit don't generate/do not generate
    // yield ImageRetrieval with [SearchWeb, FetchUrl, RetrieveImage], [Image, WebPage], image result, and retrieval true;
    // the negative instruction wins over generation words
    let has_image = const_any_keyword(
        &normalized,
        &[
            "image",
            "images",
            "picture",
            "pictures",
            "photo",
            "photos",
            "portrait",
            "portraits",
            "pic",
            "pics",
        ],
    );
    let has_negative_gen =
        normalized.contains("don't generate") || normalized.contains("do not generate");
    let has_retrieval_word =
        const_any_keyword(&normalized, &["internet", "search", "find", "show", "get"]);
    let has_generation_word = const_any_keyword(
        &normalized,
        &["generate", "create", "draw", "render", "edit"],
    );

    if has_image && (has_negative_gen || (has_retrieval_word && !has_generation_word)) {
        return RequestIntent {
            family: PrimaryIntentFamily::ImageRetrieval,
            operations: vec![
                CapabilityOperation::SearchWeb,
                CapabilityOperation::FetchUrl,
                CapabilityOperation::RetrieveImage,
            ],
            modalities: vec![CapabilityModality::Image, CapabilityModality::WebPage],
            completion: IntentCompletion::ImageResult,
            known_url: None,
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: true,
        };
    }

    // (3) image words plus generate/create/draw/render/edit yield ImageGeneration with GenerateImage, [Image], artifact, and generation true
    if has_image && has_generation_word {
        return RequestIntent {
            family: PrimaryIntentFamily::ImageGeneration,
            operations: vec![CapabilityOperation::GenerateImage],
            modalities: vec![CapabilityModality::Image],
            completion: IntentCompletion::Artifact,
            known_url: None,
            explicit_memory: false,
            explicit_generation: true,
            explicit_retrieval: false,
        };
    }

    // (4) explicit recall/remember/store-memory phrases yield Memory with recall and/or store operations in that order, [Memory, Text], memory result, and memory true
    let (has_recall, has_store) = detect_memory_intent(&normalized);
    if has_recall || has_store {
        let mut operations = Vec::new();
        if has_recall {
            operations.push(CapabilityOperation::RecallMemory);
        }
        if has_store {
            operations.push(CapabilityOperation::StoreMemory);
        }
        return RequestIntent {
            family: PrimaryIntentFamily::Memory,
            operations,
            modalities: vec![CapabilityModality::Memory, CapabilityModality::Text],
            completion: IntentCompletion::MemoryResult,
            known_url: None,
            explicit_memory: true,
            explicit_generation: false,
            explicit_retrieval: false,
        };
    }

    // (5) repository/code edit, implement, fix, refactor, test, or build phrases yield Repository with [ReadWorkspace, WriteWorkspace, ExecuteCommand], [Code, File], verified change
    if ordered_phrase_match(
        &normalized,
        &[
            "edit the repository",
            "change the repository",
            "modify the repository",
            "update the repository",
            "edit repository",
            "change repository",
            "modify repository",
            "update repository",
            "edit the code",
            "change the code",
            "modify the code",
            "update the code",
            "edit code",
            "change code",
            "modify code",
            "code edit",
            "edit the readme",
            "change the readme",
            "update the readme",
            "edit readme",
            "fix the code",
            "fix code",
            "fix the bug",
            "fix bug",
            "fix the issue",
            "fix issue",
            "fix ",
            "implement ",
            "implement",
            "refactor ",
            "refactor",
            "run the tests",
            "run tests",
            "run test",
            "test the code",
            "build the project",
            "build project",
            "build ",
        ],
    ) {
        return RequestIntent {
            family: PrimaryIntentFamily::Repository,
            operations: vec![
                CapabilityOperation::ReadWorkspace,
                CapabilityOperation::WriteWorkspace,
                CapabilityOperation::ExecuteCommand,
            ],
            modalities: vec![CapabilityModality::Code, CapabilityModality::File],
            completion: IntentCompletion::VerifiedChange,
            known_url: None,
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        };
    }

    // (6) schedule/reminder/monitor phrases yield Scheduling with Schedule, [Schedule], schedule state
    if ordered_phrase_match(
        &normalized,
        &[
            "schedule ",
            "schedule",
            "set a reminder",
            "remind me",
            "reminder",
            "monitor ",
            "monitor",
            "keep an eye on",
            "watch for ",
            "cron",
        ],
    ) {
        return RequestIntent {
            family: PrimaryIntentFamily::Scheduling,
            operations: vec![CapabilityOperation::Schedule],
            modalities: vec![CapabilityModality::Schedule],
            completion: IntentCompletion::ScheduleState,
            known_url: None,
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        };
    }

    // (7) delegate/subagent phrases yield Delegation with Delegate, [Text], delegated result
    if ordered_phrase_match(
        &normalized,
        &[
            "delegate this",
            "delegate ",
            "delegate",
            "spawn an agent",
            "subagent",
            "sub-agent",
            "parallel agent",
        ],
    ) {
        return RequestIntent {
            family: PrimaryIntentFamily::Delegation,
            operations: vec![CapabilityOperation::Delegate],
            modalities: vec![CapabilityModality::Text],
            completion: IntentCompletion::DelegatedResult,
            known_url: None,
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        };
    }

    // (8) current/latest/today news, headline, search-web, browse, look-up, or online phrases yield Web with [SearchWeb, FetchUrl], [WebPage, Text], sourced answer
    if ordered_phrase_match(
        &normalized,
        &[
            "current news",
            "latest news",
            "today's news",
            "todays news",
            "today news",
            "headline",
            "headlines",
            "search the web",
            "search web",
            "search-web",
            "browse ",
            "browse",
            "look up ",
            "look up",
            "lookup",
            "look-up",
            "search online",
            "find online",
            "online",
        ],
    ) {
        return RequestIntent {
            family: PrimaryIntentFamily::Web,
            operations: vec![
                CapabilityOperation::SearchWeb,
                CapabilityOperation::FetchUrl,
            ],
            modalities: vec![CapabilityModality::WebPage, CapabilityModality::Text],
            completion: IntentCompletion::SourcedAnswer,
            known_url: None,
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        };
    }

    // Otherwise narrow to Conversation with no capabilities. Mixed/ambiguous unmatched requests must not accumulate families.
    RequestIntent {
        family: PrimaryIntentFamily::Conversation,
        operations: Vec::new(),
        modalities: Vec::new(),
        completion: IntentCompletion::FinalText,
        known_url: None,
        explicit_memory: false,
        explicit_generation: false,
        explicit_retrieval: false,
    }
}

fn normalize(message: &str) -> String {
    message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn contains_keyword(text: &str, keyword: &str) -> bool {
    text.match_indices(keyword).any(|(start, matched)| {
        let end = start + matched.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        before.is_none_or(|ch| !ch.is_ascii_alphanumeric())
            && after.is_none_or(|ch| !ch.is_ascii_alphanumeric())
    })
}

fn const_any_keyword(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| contains_keyword(text, word))
}

fn ordered_phrase_match(message: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| message.contains(phrase))
}

fn extract_http_url(message: &str) -> Option<String> {
    for token in message.split_whitespace() {
        let trimmed = token
            .trim_start_matches(['(', '[', '<', '{', '"', '\'', '`'])
            .trim_end_matches([
                '.', ',', ';', ':', '!', '?', ')', ']', '>', '}', '"', '\'', '`',
            ]);
        let lowered = trimmed.to_ascii_lowercase();
        if lowered.starts_with("http://") || lowered.starts_with("https://") {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn detect_memory_intent(normalized: &str) -> (bool, bool) {
    let recall_phrases = [
        "what do you remember",
        "do you remember",
        "can you remember",
        "from memory",
        "search memory",
        "recall from memory",
    ];
    let mut has_recall = recall_phrases.iter().any(|p| normalized.contains(p))
        || contains_keyword(normalized, "recall");

    let store_phrases = [
        "remember that",
        "remember this",
        "store in memory",
        "store to memory",
        "save in memory",
        "save to memory",
        "store memory",
        "keep in memory",
    ];
    let mut has_store = store_phrases.iter().any(|p| normalized.contains(p));
    if !has_store && contains_keyword(normalized, "remember") {
        let is_asking = normalized.contains("what do you remember")
            || normalized.contains("do you remember")
            || normalized.contains("can you remember");
        if !is_asking {
            has_store = true;
        } else {
            has_recall = true;
        }
    }
    (has_recall, has_store)
}

#[cfg(test)]
#[path = "intent_tests.rs"]
mod intent_tests;
