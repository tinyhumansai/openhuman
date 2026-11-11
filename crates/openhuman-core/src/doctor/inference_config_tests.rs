use crate::config::Config;
use crate::doctor::runner;
use crate::doctor::{DoctorCtx, DoctorOptions, DoctorTier, Finding, FindingSeverity};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

const PROVIDER_REF: &str = "inference/provider-ref";
const CONSTRUCT: &str = "inference/construct";
const TEMPERATURE: &str = "inference/temperature";

async fn run_only(config: Config, check_id: &str) -> Vec<Finding> {
    let context = DoctorCtx::from_config(config);
    runner::run(
        &context,
        DoctorOptions {
            tier: DoctorTier::Static,
            only: vec![check_id.to_owned()],
            ..DoctorOptions::default()
        },
    )
    .await
    .expect("the selected doctor check should run")
    .findings
}

fn isolated_config(root: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = root.join("config.toml");
    config.workspace_dir = root.join("workspace");
    config
}

fn filesystem_snapshot(root: &std::path::Path) -> Vec<(std::path::PathBuf, bool, Vec<u8>)> {
    fn visit(
        root: &std::path::Path,
        dir: &std::path::Path,
        snapshot: &mut Vec<(std::path::PathBuf, bool, Vec<u8>)>,
    ) {
        for entry in std::fs::read_dir(dir).expect("read temporary config fixture") {
            let entry = entry.expect("read fixture entry");
            let path = entry.path();
            let metadata = entry.metadata().expect("read fixture metadata");
            let is_dir = metadata.is_dir();
            let contents = if is_dir {
                Vec::new()
            } else {
                std::fs::read(&path).expect("read fixture file")
            };
            snapshot.push((
                path.strip_prefix(root)
                    .expect("fixture path is under root")
                    .to_path_buf(),
                is_dir,
                contents,
            ));
            if is_dir {
                visit(root, &path, snapshot);
            }
        }
    }

    let mut snapshot = Vec::new();
    visit(root, root, &mut snapshot);
    snapshot.sort_by(|left, right| left.0.cmp(&right.0));
    snapshot
}

fn finding<'a>(findings: &'a [Finding], check_id: &str) -> &'a Finding {
    findings
        .iter()
        .find(|finding| finding.check_id == check_id)
        .unwrap_or_else(|| panic!("expected finding from {check_id}: {findings:#?}"))
}

fn finding_for_path<'a>(findings: &'a [Finding], check_id: &str, config_path: &str) -> &'a Finding {
    findings
        .iter()
        .find(|finding| {
            finding.check_id == check_id && finding.config_path.as_deref() == Some(config_path)
        })
        .unwrap_or_else(|| panic!("expected {check_id} finding for {config_path}: {findings:#?}"))
}

#[tokio::test]
async fn an_unregistered_chat_provider_reports_its_config_path() {
    let mut config = Config::default();
    config.chat_provider = Some("nope".to_owned());

    let findings = run_only(config, PROVIDER_REF).await;
    let result = finding(&findings, PROVIDER_REF);

    assert_eq!(result.severity, FindingSeverity::Error);
    assert_eq!(result.config_path.as_deref(), Some("chat_provider"));
}

#[tokio::test]
async fn a_byok_cloud_fallback_without_a_key_reports_construction_failure() {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    use crate::config::test_env::EnvVarGuard;

    let _inference_guard = crate::inference::inference_test_guard_async().await;
    let temp = tempfile::tempdir().expect("temporary configuration directory");
    let _env_guard = EnvVarGuard::locked_async()
        .await
        .with("HOME", temp.path())
        .with("OPENHUMAN_WORKSPACE", temp.path())
        .without("OPENAI_API_KEY")
        .without("OPENHUMAN_CORE_TOKEN");

    let mut config = isolated_config(temp.path());
    config.chat_provider = Some("ollama:qwen2.5".to_owned());
    config.primary_cloud = Some("provider-doctor-test".to_owned());
    config.cloud_providers = vec![CloudProviderCreds {
        id: "provider-doctor-test".to_owned(),
        slug: "doctor-test".to_owned(),
        label: "Doctor test provider".to_owned(),
        endpoint: "https://doctor-test.invalid/v1".to_owned(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("test-model".to_owned()),
    }];
    config.vision_provider = Some("cloud".to_owned());

    let before = filesystem_snapshot(temp.path());
    let findings = run_only(config, CONSTRUCT).await;
    assert_eq!(filesystem_snapshot(temp.path()), before);
    let result = finding_for_path(&findings, CONSTRUCT, "vision_provider");

    assert_eq!(result.severity, FindingSeverity::Error);
    assert_eq!(result.config_path.as_deref(), Some("vision_provider"));
    assert!(result.message.to_ascii_lowercase().contains("credential"));
    assert!(!temp.path().join("auth-profiles.json").exists());
    assert!(!temp.path().join("auth-profiles.lock").exists());
}

#[tokio::test]
async fn a_fully_configured_managed_default_constructs_cleanly() {
    use crate::config::schema::cloud_providers::{
        register_hosted_cloud_provider_policy_in, AuthStyle, CloudProviderCreds,
    };
    use crate::config::test_env::EnvVarGuard;
    use crate::inference::provider::factory::PROVIDER_OPENHUMAN;
    use crate::inference::registry::InspectionChatModelFactory;
    use tinyinference_llm::providers::overrides::{ChatModelEntry, ProviderModelMetadata};

    struct ManagedModel {
        generations: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ChatModel<()> for ManagedModel {
        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            self.generations.fetch_add(1, Ordering::SeqCst);
            panic!("doctor's construction check must not generate a model response")
        }
    }

    let _inference_guard = crate::inference::inference_test_guard_async().await;
    let temp = tempfile::tempdir().expect("temporary configuration directory");
    let _env_guard = EnvVarGuard::locked_async()
        .await
        .with("HOME", temp.path())
        .with("OPENHUMAN_WORKSPACE", temp.path())
        .without("OPENAI_API_KEY")
        .without("OPENHUMAN_CORE_TOKEN");
    let extensions = crate::core::slots::Extensions::new();
    register_hosted_cloud_provider_policy_in(
        &extensions,
        Arc::new(|slug| slug == "openhuman"),
        Arc::new(|style| style == AuthStyle::OpenhumanJwt),
    );
    let constructions = Arc::new(AtomicUsize::new(0));
    let generations = Arc::new(AtomicUsize::new(0));
    let build_constructions = Arc::clone(&constructions);
    let model_generations = Arc::clone(&generations);
    let inspection_factory: InspectionChatModelFactory = Arc::new(move |_| {
        let build_constructions = Arc::clone(&build_constructions);
        let model_generations = Arc::clone(&model_generations);
        Box::pin(async move {
            build_constructions.fetch_add(1, Ordering::SeqCst);
            Ok(crate::inference::registry::InspectionChatModelBuild {
                model: Arc::new(ManagedModel {
                    generations: Arc::clone(&model_generations),
                }) as Arc<dyn ChatModel<()>>,
                model_id: "managed-test-model".to_owned(),
                endpoint: "https://api.openhuman.invalid/openai/v1".to_owned(),
                credential: "doctor-test-session-token".to_owned(),
            })
        })
    });
    crate::inference::registry::register_inspection_chat_model_in(
        &extensions,
        PROVIDER_OPENHUMAN,
        inspection_factory,
    );
    crate::inference::registry::register_chat_model_in(
        &extensions,
        PROVIDER_OPENHUMAN,
        ChatModelEntry {
            build: Arc::new(|request| {
                Ok((
                    Arc::new(ManagedModel {
                        generations: Arc::new(AtomicUsize::new(0)),
                    }) as Arc<dyn ChatModel<()>>,
                    request
                        .model_override
                        .or(request.default_model)
                        .unwrap_or("managed-test-model")
                        .to_owned(),
                ))
            }),
            probe: None,
            models: Some(ProviderModelMetadata {
                default_model: Arc::new(|| Some("managed-test-model".to_owned())),
                role_for_model: Arc::new(|model| {
                    (model == "hint:reasoning").then(|| "reasoning".to_owned())
                }),
                is_alias: Arc::new(|model| model.starts_with("hint:")),
                is_raw_model: Arc::new(|model| !model.starts_with("hint:")),
                supports_vision: Arc::new(|_| false),
                resolve_model: Arc::new(|_, config, model| match model {
                    Some(model) if !model.starts_with("hint:") => Some(model.to_owned()),
                    Some(_) => Some("managed-test-model".to_owned()),
                    None => config
                        .config
                        .default_model
                        .clone()
                        .or_else(|| Some("managed-test-model".to_owned())),
                }),
                context_window: Arc::new(|_| None),
                default_pricing: Arc::new(|| None),
                without_reasoning: Arc::new(|request| request),
            }),
        },
    );
    let mut config = isolated_config(temp.path());
    config.api_url = Some("https://api.openhuman.invalid".to_owned());
    config.api_key = Some("doctor-test-session-token".to_owned());
    config.primary_cloud = Some("provider-openhuman".to_owned());
    config.cloud_providers = vec![CloudProviderCreds {
        id: "provider-openhuman".to_owned(),
        slug: "openhuman".to_owned(),
        label: "OpenHuman".to_owned(),
        endpoint: "https://api.openhuman.invalid/v1".to_owned(),
        auth_style: AuthStyle::OpenhumanJwt,
        default_model: Some("managed-test-model".to_owned()),
    }];
    let before = filesystem_snapshot(temp.path());
    let core_context = crate::core::runtime::CoreContext::for_test_with_extensions(
        crate::core::runtime::DomainSet::full(),
        Some(temp.path().to_path_buf()),
        extensions,
    );
    let findings = crate::core::runtime::CoreContext::scope(core_context, async {
        let dispatch = crate::core::runtime::DispatchCtx::ambient();
        let resolved = dispatch
            .resolved()
            .expect("doctor inspection should run in the scoped core context");
        assert!(crate::inference::registry::chat_model_for(
            Some(resolved.extensions()),
            PROVIDER_OPENHUMAN,
        )
        .is_some());
        assert!(crate::inference::registry::inspection_chat_model_for(
            Some(resolved.extensions()),
            PROVIDER_OPENHUMAN,
        )
        .is_some());
        run_only(config, CONSTRUCT).await
    })
    .await;
    assert_eq!(filesystem_snapshot(temp.path()), before);

    assert!(
        !findings.is_empty()
            && findings
                .iter()
                .all(|result| result.severity == FindingSeverity::Pass),
        "managed defaults should not report construction errors: {findings:#?}"
    );
    assert!(
        constructions.load(Ordering::SeqCst) > 0,
        "the managed provider factory must be consulted"
    );
    assert_eq!(generations.load(Ordering::SeqCst), 0);
    assert!(findings
        .iter()
        .all(|result| !result.message.contains("doctor-test-session-token")));
    assert!(!temp.path().join("auth-profiles.json").exists());
    assert!(!temp.path().join("auth-profiles.lock").exists());
}

#[tokio::test]
async fn an_out_of_range_temperature_reports_the_role_setting() {
    let mut config = Config::default();
    config.default_temperature = 3.5;

    let findings = run_only(config, TEMPERATURE).await;
    let result = finding(&findings, TEMPERATURE);

    assert_eq!(result.severity, FindingSeverity::Error);
    assert_eq!(result.config_path.as_deref(), Some("default_temperature"));
}
