//! Deterministic, prompt-driven tool exposure for the primary chat agent.

use std::collections::HashSet;

use tinyagents_harness::tool::{rank_tools_by_prompt, SelectableTool};

use crate::tools::ToolSpec;

const MAX_PROMPT_SELECTED_TOOLS: usize = 8;
const RECOVERY_TOOLS: &[&str] = &[
    "ask_user_clarification",
    "use_skill",
    "tool_search",
    // The primary prompt's memory contract names this tool whenever the
    // builder registered it. Keep the advertised and executable sets aligned;
    // an unavailable module does not register the tool.
    "memory_recall",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrimaryIntentFamily {
    Conversation,
    Web,
    ImageRetrieval,
    ImageGeneration,
    Repository,
    Memory,
    Scheduling,
    Planning,
    Delegation,
    Mixed,
}

#[derive(Debug)]
pub(crate) struct PrimaryTurnContract {
    pub(crate) intent_family: PrimaryIntentFamily,
    pub(crate) allowed_tools: HashSet<String>,
}

/// Narrow an already-authorized tool surface for one primary-agent turn.
///
/// `ceiling == None` means every candidate is authorized. A supplied ceiling is
/// fail-closed, including an empty one. Recovery tools are retained only when
/// they are present inside that ceiling; this function can never grant a tool.
pub(crate) fn plan_primary_tool_exposure(
    prompt: &str,
    candidates: &[ToolSpec],
    ceiling: Option<&HashSet<String>>,
    state_required: &HashSet<String>,
) -> HashSet<String> {
    plan_primary_turn_contract(prompt, candidates, ceiling, state_required).allowed_tools
}

pub(crate) fn plan_primary_turn_contract(
    prompt: &str,
    candidates: &[ToolSpec],
    ceiling: Option<&HashSet<String>>,
    state_required: &HashSet<String>,
) -> PrimaryTurnContract {
    let authorized: Vec<&ToolSpec> = candidates
        .iter()
        .filter(|spec| ceiling.is_none_or(|allowed| allowed.contains(&spec.name)))
        .collect();
    let authorized_names: HashSet<&str> =
        authorized.iter().map(|spec| spec.name.as_str()).collect();
    let mut selected = HashSet::new();

    for name in RECOVERY_TOOLS {
        if authorized_names.contains(name) {
            selected.insert((*name).to_string());
        }
    }
    for name in state_required {
        if authorized_names.contains(name.as_str()) {
            selected.insert(name.clone());
        }
    }

    if prompt.trim().is_empty() {
        return PrimaryTurnContract {
            intent_family: PrimaryIntentFamily::Conversation,
            allowed_tools: selected,
        };
    }

    let lowered = prompt.to_ascii_lowercase();
    let intent_family = classify_primary_intent(&lowered);
    if intent_family == PrimaryIntentFamily::ImageRetrieval {
        // `use_skill` can delegate to the image-generation agent, which changes
        // modality despite an explicit "from the internet / don't generate"
        // request. Direct retrieval tools are sufficient for this contract.
        selected.remove("use_skill");
    }
    let families = matching_families(&lowered, intent_family);
    let ordinary: Vec<&ToolSpec> = authorized
        .iter()
        .copied()
        .filter(|spec| !RECOVERY_TOOLS.contains(&spec.name.as_str()))
        .filter(|spec| {
            families.is_empty()
                || families
                    .iter()
                    .any(|family| family.iter().any(|marker| spec.name.contains(marker)))
        })
        .collect();
    let selectable: Vec<SelectableTool<'_>> = ordinary
        .iter()
        .map(|spec| SelectableTool::new(&spec.name, &spec.description))
        .collect();
    let ordinary_names: Vec<&str> = ordinary.iter().map(|spec| spec.name.as_str()).collect();

    for index in rank_tools_by_prompt(prompt, &selectable, MAX_PROMPT_SELECTED_TOOLS) {
        if let (Some(name), Some(spec)) = (ordinary_names.get(index), ordinary.get(index)) {
            if !families.is_empty() || has_resource_overlap(prompt, spec) {
                selected.insert((*name).to_string());
            }
        }
    }

    // Lexical ranking can be thin for proper nouns ("Google News") even when
    // the capability family is unambiguous. Fill the remaining slots from that
    // family, in stable registry order; never fall back to the whole catalogue.
    let mut ordinary_count = selected
        .iter()
        .filter(|name| !RECOVERY_TOOLS.contains(&name.as_str()))
        .count();
    for spec in ordinary {
        if ordinary_count >= MAX_PROMPT_SELECTED_TOOLS {
            break;
        }
        if !selected.contains(&spec.name)
            && families
                .iter()
                .any(|family| family.iter().any(|marker| spec.name.contains(marker)))
        {
            selected.insert(spec.name.clone());
            ordinary_count += 1;
        }
    }

    PrimaryTurnContract {
        intent_family,
        allowed_tools: selected,
    }
}

fn classify_primary_intent(prompt: &str) -> PrimaryIntentFamily {
    let has_image = ["image", "picture", "photo", "portrait", "pic"]
        .iter()
        .any(|word| contains_keyword(prompt, word));
    let generation = ["generate", "create", "draw", "render", "synthesize"]
        .iter()
        .any(|word| contains_keyword(prompt, word));
    let retrieval = [
        "internet", "web", "online", "site", "website", "search", "find",
    ]
    .iter()
    .any(|word| contains_keyword(prompt, word))
        || prompt.contains("don't generate")
        || prompt.contains("do not generate");
    if has_image
        && retrieval
        && (!generation || prompt.contains("don't generate") || prompt.contains("do not generate"))
    {
        return PrimaryIntentFamily::ImageRetrieval;
    }
    if has_image && generation {
        return PrimaryIntentFamily::ImageGeneration;
    }

    let matches = [
        (
            PrimaryIntentFamily::Web,
            &[
                "web", "internet", "online", "website", "url", "news", "headline", "google",
            ][..],
        ),
        (
            PrimaryIntentFamily::Repository,
            &[
                "repo",
                "repository",
                "code",
                "file",
                "readme",
                "test",
                "git",
                "commit",
                "build",
            ][..],
        ),
        (
            PrimaryIntentFamily::Memory,
            &["remember", "recall", "memory"][..],
        ),
        (
            PrimaryIntentFamily::Scheduling,
            &["schedule", "remind", "calendar", "timer", "cron"][..],
        ),
        (PrimaryIntentFamily::Planning, &["plan", "todo"][..]),
        (
            PrimaryIntentFamily::Delegation,
            &["delegate", "subagent", "sub-agent", "parallel agent"][..],
        ),
    ];
    let found: Vec<_> = matches
        .iter()
        .filter(|(_, words)| words.iter().any(|word| contains_keyword(prompt, word)))
        .map(|(family, _)| *family)
        .collect();
    match found.as_slice() {
        [] => PrimaryIntentFamily::Conversation,
        [family] => *family,
        _ => PrimaryIntentFamily::Mixed,
    }
}

fn matching_families(
    prompt: &str,
    intent_family: PrimaryIntentFamily,
) -> Vec<&'static [&'static str]> {
    const WEB_WORDS: &[&str] = &[
        "web", "internet", "online", "website", "url", "news", "headline", "google",
    ];
    const WEB_TOOLS: &[&str] = &["web", "browser", "http", "curl"];
    const REPO_WORDS: &[&str] = &[
        "repo",
        "pr",
        "pull request",
        "repository",
        "code",
        "file",
        "readme",
        "test",
        "git",
        "commit",
        "build",
    ];
    const REPO_TOOLS: &[&str] = &[
        "file",
        "shell",
        "patch",
        "apply",
        "grep",
        "glob",
        "git",
        "diff",
        "test",
        "lint",
        "workspace",
    ];
    const MEDIA_WORDS: &[&str] = &[
        "image", "picture", "photo", "portrait", "pic", "video", "audio", "music",
    ];
    const MEDIA_TOOLS: &[&str] = &["image", "video", "audio", "media"];
    const MEMORY_WORDS: &[&str] = &["remember", "recall", "memory"];
    const MEMORY_TOOLS: &[&str] = &["memory", "recall"];
    const TIME_WORDS: &[&str] = &["schedule", "remind", "calendar", "timer", "cron", "time"];
    const TIME_TOOLS: &[&str] = &["schedule", "remind", "calendar", "time", "cron"];
    const PLAN_WORDS: &[&str] = &["plan", "todo"];
    const PLAN_TOOLS: &[&str] = &["plan", "todo", "task"];
    const DELEGATE_WORDS: &[&str] = &["delegate", "subagent", "sub-agent", "parallel agent"];
    const DELEGATE_TOOLS: &[&str] = &["subagent", "sub_agent", "agent", "delegate"];

    if intent_family == PrimaryIntentFamily::ImageRetrieval {
        return vec![WEB_TOOLS];
    }
    if intent_family == PrimaryIntentFamily::ImageGeneration {
        return vec![MEDIA_TOOLS];
    }

    let definitions: &[(&[&str], &[&str])] = &[
        (WEB_WORDS, WEB_TOOLS),
        (REPO_WORDS, REPO_TOOLS),
        (MEDIA_WORDS, MEDIA_TOOLS),
        (MEMORY_WORDS, MEMORY_TOOLS),
        (TIME_WORDS, TIME_TOOLS),
        (PLAN_WORDS, PLAN_TOOLS),
        (DELEGATE_WORDS, DELEGATE_TOOLS),
    ];
    definitions
        .iter()
        .filter_map(|(words, tools)| {
            words
                .iter()
                .any(|word| contains_keyword(prompt, word))
                .then_some(*tools)
        })
        .collect()
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

fn has_resource_overlap(prompt: &str, spec: &ToolSpec) -> bool {
    const GENERIC: &[&str] = &[
        "show", "read", "get", "fetch", "list", "search", "find", "create", "make", "add", "send",
        "write", "update", "edit", "change", "delete", "remove", "run", "use", "the", "and", "for",
        "with", "from", "this", "that", "my", "your",
    ];
    let haystack = format!("{} {}", spec.name, spec.description).to_ascii_lowercase();
    prompt
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|word| word.len() > 2 && !GENERIC.contains(word))
        .any(|word| contains_keyword(&haystack, word))
}

#[cfg(test)]
#[path = "primary_tool_exposure_tests.rs"]
mod primary_tool_exposure_tests;
