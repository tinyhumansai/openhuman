use super::super::capability::{CapabilityModality, CapabilityOperation};
use super::super::mode::PrimaryTurnMode;
use super::*;

#[test]
fn chat_mode_and_greetings_produce_conversation_without_capabilities() {
    // Explicit Chat mode always produces Conversation and FinalText with no capabilities,
    // even when action words are present.
    let action_prompts_in_chat = [
        "search the web for current news",
        "generate a portrait of an astronaut",
        "edit the repository and run tests",
        "remember that my name is Alice",
        "schedule a reminder for tomorrow",
        "fetch https://example.com/data",
    ];
    for prompt in action_prompts_in_chat {
        let resolved = resolve_request_intent(prompt, PrimaryTurnMode::Chat);
        assert_eq!(
            resolved,
            RequestIntent {
                family: PrimaryIntentFamily::Conversation,
                operations: Vec::new(),
                modalities: Vec::new(),
                completion: IntentCompletion::FinalText,
                known_url: None,
                explicit_memory: false,
                explicit_generation: false,
                explicit_retrieval: false,
            },
            "Action prompt in Chat mode should produce Conversation with no capabilities: {prompt}"
        );
    }

    // Greetings and general questions in any mode produce Conversation, FinalText, no operations/modalities/URL
    let general_prompts = [
        "hello there",
        "hi",
        "good morning",
        "why is the sky blue?",
        "what is the meaning of life?",
        "how does photosynthesis work?",
    ];
    for mode in [
        PrimaryTurnMode::Chat,
        PrimaryTurnMode::Assist,
        PrimaryTurnMode::Agent,
    ] {
        for prompt in general_prompts {
            let resolved = resolve_request_intent(prompt, mode);
            assert_eq!(
                resolved,
                RequestIntent {
                    family: PrimaryIntentFamily::Conversation,
                    operations: Vec::new(),
                    modalities: Vec::new(),
                    completion: IntentCompletion::FinalText,
                    known_url: None,
                    explicit_memory: false,
                    explicit_generation: false,
                    explicit_retrieval: false,
                },
                "Greeting/general prompt in {mode:?} mode should produce Conversation with no capabilities: {prompt}"
            );
        }
    }
}

#[test]
fn assist_current_news_produces_web_sourced_answer() {
    let resolved = resolve_request_intent("current news", PrimaryTurnMode::Assist);
    assert_eq!(
        resolved,
        RequestIntent {
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
        }
    );

    assert_eq!(resolved.family, PrimaryIntentFamily::Web);
    assert_eq!(
        resolved.operations,
        vec![
            CapabilityOperation::SearchWeb,
            CapabilityOperation::FetchUrl,
        ]
    );
    assert_eq!(
        resolved.modalities,
        vec![CapabilityModality::WebPage, CapabilityModality::Text]
    );
    assert_eq!(resolved.completion, IntentCompletion::SourcedAnswer);
    assert_eq!(resolved.known_url, None);
    assert!(!resolved.explicit_memory);
    assert!(!resolved.explicit_generation);
    assert!(!resolved.explicit_retrieval);
}

#[test]
fn punctuation_wrapped_url_extracted_unchanged_and_produces_fetch_url_only() {
    let input = "Check this page: (https://example.com/a?q=1)!";
    let resolved = resolve_request_intent(input, PrimaryTurnMode::Assist);
    assert_eq!(
        resolved,
        RequestIntent {
            family: PrimaryIntentFamily::Web,
            operations: vec![CapabilityOperation::FetchUrl],
            modalities: vec![CapabilityModality::WebPage, CapabilityModality::Text],
            completion: IntentCompletion::SourcedAnswer,
            known_url: Some("https://example.com/a?q=1".to_string()),
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        }
    );

    assert_eq!(
        resolved.known_url.as_deref(),
        Some("https://example.com/a?q=1")
    );
    assert_eq!(resolved.operations, vec![CapabilityOperation::FetchUrl]);
    assert_eq!(resolved.family, PrimaryIntentFamily::Web);
    assert_eq!(resolved.completion, IntentCompletion::SourcedAnswer);
    assert_eq!(
        resolved.modalities,
        vec![CapabilityModality::WebPage, CapabilityModality::Text]
    );
    assert!(!resolved.explicit_memory);
    assert!(!resolved.explicit_generation);
    assert!(!resolved.explicit_retrieval);
}

#[test]
fn negative_generation_instruction_wins_for_image_retrieval() {
    let prompt = "get me a picture from the internet; don't generate it";
    let resolved = resolve_request_intent(prompt, PrimaryTurnMode::Assist);
    assert_eq!(
        resolved,
        RequestIntent {
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
        }
    );

    assert_eq!(resolved.family, PrimaryIntentFamily::ImageRetrieval);
    assert_eq!(
        resolved.operations,
        vec![
            CapabilityOperation::SearchWeb,
            CapabilityOperation::FetchUrl,
            CapabilityOperation::RetrieveImage,
        ]
    );
    assert!(
        !resolved
            .operations
            .contains(&CapabilityOperation::GenerateImage),
        "Must never contain GenerateImage when negative generation instruction is present"
    );
    assert_eq!(
        resolved.modalities,
        vec![CapabilityModality::Image, CapabilityModality::WebPage]
    );
    assert_eq!(resolved.completion, IntentCompletion::ImageResult);
    assert_eq!(resolved.known_url, None);
    assert!(!resolved.explicit_memory);
    assert!(!resolved.explicit_generation);
    assert!(resolved.explicit_retrieval);
}

#[test]
fn generate_portrait_produces_image_generation_only() {
    let resolved = resolve_request_intent("generate a portrait", PrimaryTurnMode::Assist);
    assert_eq!(
        resolved,
        RequestIntent {
            family: PrimaryIntentFamily::ImageGeneration,
            operations: vec![CapabilityOperation::GenerateImage],
            modalities: vec![CapabilityModality::Image],
            completion: IntentCompletion::Artifact,
            known_url: None,
            explicit_memory: false,
            explicit_generation: true,
            explicit_retrieval: false,
        }
    );

    assert_eq!(resolved.family, PrimaryIntentFamily::ImageGeneration);
    assert_eq!(
        resolved.operations,
        vec![CapabilityOperation::GenerateImage]
    );
    assert_eq!(resolved.modalities, vec![CapabilityModality::Image]);
    assert_eq!(resolved.completion, IntentCompletion::Artifact);
    assert_eq!(resolved.known_url, None);
    assert!(!resolved.explicit_memory);
    assert!(resolved.explicit_generation);
    assert!(!resolved.explicit_retrieval);
}

#[test]
fn memory_prompts_set_explicit_memory_and_yield_ordered_operations() {
    // Single recall operation
    let recall_resolved = resolve_request_intent(
        "what do you remember about our project?",
        PrimaryTurnMode::Assist,
    );
    assert_eq!(
        recall_resolved,
        RequestIntent {
            family: PrimaryIntentFamily::Memory,
            operations: vec![CapabilityOperation::RecallMemory],
            modalities: vec![CapabilityModality::Memory, CapabilityModality::Text],
            completion: IntentCompletion::MemoryResult,
            known_url: None,
            explicit_memory: true,
            explicit_generation: false,
            explicit_retrieval: false,
        }
    );
    assert_eq!(recall_resolved.family, PrimaryIntentFamily::Memory);
    assert_eq!(
        recall_resolved.operations,
        vec![CapabilityOperation::RecallMemory]
    );
    assert!(recall_resolved.explicit_memory);

    // Single store operation
    let store_resolved = resolve_request_intent(
        "remember that my favorite editor is vim",
        PrimaryTurnMode::Assist,
    );
    assert_eq!(
        store_resolved,
        RequestIntent {
            family: PrimaryIntentFamily::Memory,
            operations: vec![CapabilityOperation::StoreMemory],
            modalities: vec![CapabilityModality::Memory, CapabilityModality::Text],
            completion: IntentCompletion::MemoryResult,
            known_url: None,
            explicit_memory: true,
            explicit_generation: false,
            explicit_retrieval: false,
        }
    );
    assert_eq!(store_resolved.family, PrimaryIntentFamily::Memory);
    assert_eq!(
        store_resolved.operations,
        vec![CapabilityOperation::StoreMemory]
    );
    assert!(store_resolved.explicit_memory);

    // Prompt explicitly asking both yields [RecallMemory, StoreMemory]
    let both_resolved = resolve_request_intent(
        "recall what my favorite editor is, then remember that I switched to neovim",
        PrimaryTurnMode::Assist,
    );
    assert_eq!(
        both_resolved,
        RequestIntent {
            family: PrimaryIntentFamily::Memory,
            operations: vec![
                CapabilityOperation::RecallMemory,
                CapabilityOperation::StoreMemory,
            ],
            modalities: vec![CapabilityModality::Memory, CapabilityModality::Text],
            completion: IntentCompletion::MemoryResult,
            known_url: None,
            explicit_memory: true,
            explicit_generation: false,
            explicit_retrieval: false,
        }
    );
    assert_eq!(both_resolved.family, PrimaryIntentFamily::Memory);
    assert_eq!(
        both_resolved.operations,
        vec![
            CapabilityOperation::RecallMemory,
            CapabilityOperation::StoreMemory,
        ]
    );
    assert_eq!(
        both_resolved.modalities,
        vec![CapabilityModality::Memory, CapabilityModality::Text]
    );
    assert_eq!(both_resolved.completion, IntentCompletion::MemoryResult);
    assert_eq!(both_resolved.known_url, None);
    assert!(both_resolved.explicit_memory);
    assert!(!both_resolved.explicit_generation);
    assert!(!both_resolved.explicit_retrieval);
}

#[test]
fn repository_mutation_produces_workspace_ops_and_verified_change() {
    let resolved = resolve_request_intent(
        "edit the repository to add unit tests",
        PrimaryTurnMode::Assist,
    );
    assert_eq!(
        resolved,
        RequestIntent {
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
        }
    );
    assert_eq!(resolved.family, PrimaryIntentFamily::Repository);
    assert_eq!(
        resolved.operations,
        vec![
            CapabilityOperation::ReadWorkspace,
            CapabilityOperation::WriteWorkspace,
            CapabilityOperation::ExecuteCommand,
        ]
    );
    assert_eq!(
        resolved.modalities,
        vec![CapabilityModality::Code, CapabilityModality::File]
    );
    assert_eq!(resolved.completion, IntentCompletion::VerifiedChange);
    assert_eq!(resolved.known_url, None);
    assert!(!resolved.explicit_memory);
    assert!(!resolved.explicit_generation);
    assert!(!resolved.explicit_retrieval);
}

#[test]
fn scheduling_produces_schedule_operation_and_state() {
    let resolved = resolve_request_intent("schedule a daily backup check", PrimaryTurnMode::Assist);
    assert_eq!(
        resolved,
        RequestIntent {
            family: PrimaryIntentFamily::Scheduling,
            operations: vec![CapabilityOperation::Schedule],
            modalities: vec![CapabilityModality::Schedule],
            completion: IntentCompletion::ScheduleState,
            known_url: None,
            explicit_memory: false,
            explicit_generation: false,
            explicit_retrieval: false,
        }
    );
    assert_eq!(resolved.family, PrimaryIntentFamily::Scheduling);
    assert_eq!(resolved.operations, vec![CapabilityOperation::Schedule]);
    assert_eq!(resolved.modalities, vec![CapabilityModality::Schedule]);
    assert_eq!(resolved.completion, IntentCompletion::ScheduleState);
    assert_eq!(resolved.known_url, None);
    assert!(!resolved.explicit_memory);
    assert!(!resolved.explicit_generation);
    assert!(!resolved.explicit_retrieval);
}

#[test]
fn unmatched_and_ambiguous_prose_narrows_to_conversation_with_no_capabilities() {
    let ambiguous_inputs = [
        "",
        "   ",
        "can you help me understand how this works?",
        "what do you think about distributed systems?",
        "summarize our recent discussion in bullet points",
    ];
    for prompt in ambiguous_inputs {
        let resolved = resolve_request_intent(prompt, PrimaryTurnMode::Assist);
        assert_eq!(
            resolved,
            RequestIntent {
                family: PrimaryIntentFamily::Conversation,
                operations: Vec::new(),
                modalities: Vec::new(),
                completion: IntentCompletion::FinalText,
                known_url: None,
                explicit_memory: false,
                explicit_generation: false,
                explicit_retrieval: false,
            },
            "Ambiguous/unmatched prompt should narrow to Conversation: {prompt:?}"
        );
        assert_eq!(resolved.family, PrimaryIntentFamily::Conversation);
        assert!(resolved.operations.is_empty());
        assert!(resolved.modalities.is_empty());
        assert_eq!(resolved.completion, IntentCompletion::FinalText);
        assert_eq!(resolved.known_url, None);
        assert!(!resolved.explicit_memory);
        assert!(!resolved.explicit_generation);
        assert!(!resolved.explicit_retrieval);
    }
}
