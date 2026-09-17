use super::completion::*;
use super::intent::{IntentCompletion, PrimaryIntentFamily, RequestIntent};

#[test]
fn test_chat_contract_completion() {
    let contract = CompletionContract::Chat;

    // Non-empty assistant text completes
    let mut obs = CompletionObservation {
        final_assistant_text: Some("Hello! How can I help you?".into()),
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&contract, &obs),
        CompletionStatus::Complete
    );

    // Empty assistant text is incomplete
    obs.final_assistant_text = Some("   ".into());
    assert!(evaluate_completion(&contract, &obs).is_incomplete());

    // None text with no tools is incomplete
    obs.final_assistant_text = None;
    assert!(evaluate_completion(&contract, &obs).is_incomplete());

    // Informational tools executed without final assistant text is incomplete
    obs.informational_tools_executed = vec!["memory_recall".into()];
    let res = evaluate_completion(&contract, &obs);
    assert!(res.is_incomplete());
    if let CompletionStatus::Incomplete {
        needs_final_model_call,
        ..
    } = res
    {
        assert!(needs_final_model_call);
    }
}

#[test]
fn test_sourced_web_contract_completion() {
    let contract = CompletionContract::SourcedWeb { min_sources: 1 };

    // Informational search executed but no final text yet -> incomplete, needs model call
    let obs_search_only = CompletionObservation {
        informational_tools_executed: vec!["web_search".into()],
        web_sources: vec!["https://example.com/article".into()],
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_search_only).is_incomplete());

    // Final text with web_sources completes
    let obs_complete = CompletionObservation {
        final_assistant_text: Some("According to the report, the mission succeeded.".into()),
        web_sources: vec!["https://example.com/article".into()],
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&contract, &obs_complete),
        CompletionStatus::Complete
    );

    // Final text with embedded source URL completes even if list was not populated
    let obs_embedded_source = CompletionObservation {
        final_assistant_text: Some(
            "Details can be found at https://news.example.org/launch".into(),
        ),
        web_sources: Vec::new(),
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&contract, &obs_embedded_source),
        CompletionStatus::Complete
    );

    // Final text without any sources is false completion
    let obs_no_sources = CompletionObservation {
        final_assistant_text: Some("I know this fact without any citations.".into()),
        web_sources: Vec::new(),
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_no_sources).is_false_completion());
}

#[test]
fn test_image_retrieval_contract_completion() {
    let contract = CompletionContract::ImageRetrieval {
        require_direct_media: true,
    };

    // Informational search/navigation executed without validated image is false completion
    let obs_nav_only = CompletionObservation {
        informational_tools_executed: vec!["browser_search".into(), "fetch_url".into()],
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_nav_only).is_false_completion());

    // Search results page URL is false completion
    let obs_search_url = CompletionObservation {
        validated_images: vec![ValidatedImage {
            url_or_path: "https://www.google.com/search?q=space+telescope&tbm=isch".into(),
            source_origin: "google.com".into(),
            mime_type: None,
        }],
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_search_url).is_false_completion());

    let obs_bing_search = CompletionObservation {
        validated_images: vec![ValidatedImage {
            url_or_path: "https://www.bing.com/images/search?q=mars".into(),
            source_origin: "bing.com".into(),
            mime_type: None,
        }],
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_bing_search).is_false_completion());

    // Validated direct image URL completes
    let obs_valid_img = CompletionObservation {
        validated_images: vec![ValidatedImage {
            url_or_path: "https://images.example.com/hubble.jpg".into(),
            source_origin: "nasa.gov".into(),
            mime_type: Some("image/jpeg".into()),
        }],
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&contract, &obs_valid_img),
        CompletionStatus::Complete
    );
}

#[test]
fn test_artifact_generation_contract_completion() {
    let contract = CompletionContract::ArtifactGeneration {
        require_explanation: true,
    };

    // No artifact is incomplete
    let obs_none = CompletionObservation::default();
    assert!(evaluate_completion(&contract, &obs_none).is_incomplete());

    // Artifact without explanation is incomplete
    let obs_no_expl = CompletionObservation {
        generated_artifacts: vec![GeneratedArtifact {
            path_or_url: "/tmp/generated_diagram.svg".into(),
            description: Some("Architecture diagram".into()),
        }],
        final_assistant_text: None,
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_no_expl).is_incomplete());

    // Artifact with explanation completes
    let obs_with_expl = CompletionObservation {
        generated_artifacts: vec![GeneratedArtifact {
            path_or_url: "/tmp/generated_diagram.svg".into(),
            description: Some("Architecture diagram".into()),
        }],
        final_assistant_text: Some("Here is the diagram illustrating the workflow.".into()),
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&contract, &obs_with_expl),
        CompletionStatus::Complete
    );
}

#[test]
fn test_repository_mutation_contract_completion() {
    let contract = CompletionContract::RepositoryMutation {
        require_verification: true,
    };

    // Informational tool only (read file) is incomplete
    let obs_info = CompletionObservation {
        informational_tools_executed: vec!["read_file".into(), "git_status".into()],
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_info).is_incomplete());

    // Mutation unverified is false completion
    let obs_unverified = CompletionObservation {
        repository_changes: vec![RepositoryChangeRecord {
            file_paths: vec!["src/main.rs".into()],
            verification: VerificationStatus::Unverified,
            summary: Some("Updated main".into()),
        }],
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_unverified).is_false_completion());

    // Mutation with failed verification is false completion
    let obs_failed = CompletionObservation {
        repository_changes: vec![RepositoryChangeRecord {
            file_paths: vec!["src/main.rs".into()],
            verification: VerificationStatus::Failed,
            summary: Some("Compiler error".into()),
        }],
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_failed).is_false_completion());

    // Mutation with verified status completes
    let obs_verified = CompletionObservation {
        repository_changes: vec![RepositoryChangeRecord {
            file_paths: vec!["src/main.rs".into()],
            verification: VerificationStatus::Verified,
            summary: Some("Tests passed".into()),
        }],
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&contract, &obs_verified),
        CompletionStatus::Complete
    );
}

#[test]
fn test_scheduling_contract_completion() {
    let contract = CompletionContract::Scheduling;

    // No record is incomplete
    let obs_none = CompletionObservation::default();
    assert!(evaluate_completion(&contract, &obs_none).is_incomplete());

    // Empty id is false completion
    let obs_empty = CompletionObservation {
        scheduling_records: vec![ScheduleRecord {
            schedule_id: "  ".into(),
            state: "active".into(),
        }],
        ..Default::default()
    };
    assert!(evaluate_completion(&contract, &obs_empty).is_false_completion());

    // Valid record completes
    let obs_valid = CompletionObservation {
        scheduling_records: vec![ScheduleRecord {
            schedule_id: "sched_12345".into(),
            state: "scheduled".into(),
        }],
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&contract, &obs_valid),
        CompletionStatus::Complete
    );
}

#[test]
fn test_clarification_and_approval_contracts() {
    let clar = CompletionContract::Clarification;
    let appr = CompletionContract::Approval;

    assert!(evaluate_completion(&clar, &CompletionObservation::default()).is_incomplete());
    assert!(evaluate_completion(&appr, &CompletionObservation::default()).is_incomplete());

    let obs_clar = CompletionObservation {
        yielded_question: Some("Which directory should I clean?".into()),
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&clar, &obs_clar),
        CompletionStatus::Complete
    );

    let obs_appr = CompletionObservation {
        yielded_approval: Some("Delete 5 files in C:\\temp?".into()),
        ..Default::default()
    };
    assert_eq!(
        evaluate_completion(&appr, &obs_appr),
        CompletionStatus::Complete
    );
}

#[test]
fn test_deterministic_rendering() {
    // Image retrieval deterministic rendering
    let img_contract = CompletionContract::ImageRetrieval {
        require_direct_media: true,
    };
    let img_obs = CompletionObservation {
        validated_images: vec![ValidatedImage {
            url_or_path: "https://example.com/space.png".into(),
            source_origin: "nasa.gov".into(),
            mime_type: Some("image/png".into()),
        }],
        ..Default::default()
    };
    let rendered = render_deterministic_completion(&img_contract, &img_obs);
    assert!(rendered.is_some());
    let r = rendered.unwrap();
    assert!(r.contains("https://example.com/space.png"));
    assert!(r.contains("nasa.gov"));

    // Scheduling deterministic rendering
    let sched_contract = CompletionContract::Scheduling;
    let sched_obs = CompletionObservation {
        scheduling_records: vec![ScheduleRecord {
            schedule_id: "job_99".into(),
            state: "enabled".into(),
        }],
        ..Default::default()
    };
    let sched_rendered = render_deterministic_completion(&sched_contract, &sched_obs).unwrap();
    assert!(sched_rendered.contains("job_99"));
    assert!(sched_rendered.contains("enabled"));

    // Chat has no deterministic rendering (requires model summarization)
    let chat_contract = CompletionContract::Chat;
    assert!(
        render_deterministic_completion(&chat_contract, &CompletionObservation::default())
            .is_none()
    );
}

#[test]
fn test_contract_from_intent_mapping() {
    let base_intent = RequestIntent {
        family: PrimaryIntentFamily::Conversation,
        operations: Vec::new(),
        modalities: Vec::new(),
        completion: IntentCompletion::FinalText,
        known_url: None,
        explicit_memory: false,
        explicit_generation: false,
        explicit_retrieval: false,
    };

    assert_eq!(contract_from_intent(&base_intent), CompletionContract::Chat);

    let mut intent = base_intent.clone();
    intent.completion = IntentCompletion::SourcedAnswer;
    assert_eq!(
        contract_from_intent(&intent),
        CompletionContract::SourcedWeb { min_sources: 1 }
    );

    intent.completion = IntentCompletion::ImageResult;
    assert_eq!(
        contract_from_intent(&intent),
        CompletionContract::ImageRetrieval {
            require_direct_media: true
        }
    );

    intent.completion = IntentCompletion::Artifact;
    assert_eq!(
        contract_from_intent(&intent),
        CompletionContract::ArtifactGeneration {
            require_explanation: true
        }
    );

    intent.completion = IntentCompletion::VerifiedChange;
    assert_eq!(
        contract_from_intent(&intent),
        CompletionContract::RepositoryMutation {
            require_verification: true
        }
    );

    intent.completion = IntentCompletion::ScheduleState;
    assert_eq!(
        contract_from_intent(&intent),
        CompletionContract::Scheduling
    );
}
