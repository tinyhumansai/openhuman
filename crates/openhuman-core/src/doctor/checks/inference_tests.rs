use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
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
    config.action_dir = root.join("action");
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
async fn invalid_role_primary_cloud_and_route_references_report_each_path() {
    let mut config = Config::default();
    config.chat_provider = Some("missing-chat-provider:model".to_owned());
    config.reasoning_provider = Some("missing-chat-provider:model".to_owned());
    config.agentic_provider = Some("missing-agentic-provider:model".to_owned());
    config.coding_provider = Some("missing-coding-provider:model".to_owned());
    config.vision_provider = Some("missing-vision-provider:model".to_owned());
    config.memory_provider = Some("missing-memory-provider:model".to_owned());
    config.embeddings_provider = Some("missing-embedding-provider".to_owned());
    config.primary_cloud = Some("missing-primary-id".to_owned());
    config.stt_provider = Some("missing-stt-provider".to_owned());
    config.tts_provider = Some("missing-tts-provider".to_owned());
    config.model_routes = vec![crate::config::ModelRouteConfig {
        hint: "reasoning".to_owned(),
        model: "missing-route-provider:model".to_owned(),
    }];
    config.embedding_routes = vec![crate::config::EmbeddingRouteConfig {
        hint: "recall".to_owned(),
        provider: "missing-embedding-provider".to_owned(),
        model: "embedding-model".to_owned(),
        dimensions: Some(768),
    }];

    let findings = run_only(config, PROVIDER_REF).await;
    let paths = findings
        .iter()
        .filter(|result| result.check_id == PROVIDER_REF)
        .filter_map(|result| result.config_path.as_deref())
        .collect::<Vec<_>>();

    for expected in [
        "chat_provider",
        "reasoning_provider",
        "agentic_provider",
        "coding_provider",
        "vision_provider",
        "memory_provider",
        "embeddings_provider",
        "primary_cloud",
        "model_routes[0].model",
        "embedding_routes[0].provider",
        "stt_provider",
        "tts_provider",
    ] {
        assert!(
            paths.contains(&expected),
            "missing {expected} provider-reference finding: {findings:#?}"
        );
    }
}

#[cfg(feature = "voice")]
#[tokio::test]
async fn registered_voice_provider_ids_resolve_without_calling_the_factory() {
    let _registered = RegisterVoiceProviders;
    crate::voice::factory::registry::register_stt_provider(
        "doctor-fake-stt",
        |_, _| -> anyhow::Result<Box<dyn crate::voice::SttProvider>> {
            panic!("provider-reference checks must not invoke STT constructors")
        },
    );
    crate::voice::factory::registry::register_tts_provider(
        "doctor-fake-tts",
        |_, _| -> anyhow::Result<Box<dyn crate::voice::TtsProvider>> {
            panic!("provider-reference checks must not invoke TTS constructors")
        },
    );

    let mut config = Config::default();
    config.stt_provider = Some("doctor-fake-stt:small".to_owned());
    config.tts_provider = Some("doctor-fake-tts:voice".to_owned());

    let findings = run_only(config, PROVIDER_REF).await;

    assert_eq!(findings.len(), 1, "{findings:#?}");
    assert_eq!(findings[0].check_id, PROVIDER_REF);
    assert_eq!(findings[0].severity, FindingSeverity::Pass);
}

#[cfg(feature = "voice")]
struct RegisterVoiceProviders;

#[cfg(feature = "voice")]
impl Drop for RegisterVoiceProviders {
    fn drop(&mut self) {
        crate::voice::factory::registry::unregister_stt_provider("doctor-fake-stt");
        crate::voice::factory::registry::unregister_tts_provider("doctor-fake-tts");
    }
}

#[tokio::test]
async fn a_byok_cloud_fallback_without_a_key_reports_construction_failure() {
    use crate::config::schema::cloud_providers::CloudProviderCreds;
    use crate::config::test_env::EnvVarGuard;

    let _inference_guard = crate::inference::inference_test_guard_async().await;
    let temp = tempfile::tempdir().expect("temporary configuration directory");
    let _env_guard = EnvVarGuard::locked_async()
        .await
        .with("HOME", temp.path())
        .with("OPENHUMAN_WORKSPACE", temp.path())
        .without("OPENHUMAN_ACTION_DIR")
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
    let result = findings
        .iter()
        .find(|finding| {
            finding.check_id == CONSTRUCT
                && finding.config_path.as_deref() == Some("vision_provider")
        })
        .expect("the configured vision fallback should report its own config path");

    assert_eq!(result.severity, FindingSeverity::Error);
    assert_eq!(result.config_path.as_deref(), Some("vision_provider"));
    assert!(result.message.to_ascii_lowercase().contains("credential"));
    assert!(!temp.path().join("auth-profiles.json").exists());
    assert!(!temp.path().join("auth-profiles.lock").exists());
}

#[tokio::test]
async fn inspection_reports_refresh_required_without_refreshing_or_publishing_session_expiry() {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    use crate::config::test_env::EnvVarGuard;
    use crate::core::events::DomainEvent;
    use crate::core::runtime::DispatchCtx;
    use crate::inference::provider::factory::create_chat_model_with_model_id_in_for_inspection;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct LocalProxyTrap {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for LocalProxyTrap {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn no_session_expiry_event(receiver: &mut tinybus::EventReceiver<DomainEvent>) -> bool {
        loop {
            match receiver.try_recv() {
                Ok(DomainEvent::SessionExpired { .. }) => return false,
                Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
                Err(_) => return true,
            }
        }
    }

    fn drain_events(receiver: &mut tinybus::EventReceiver<DomainEvent>) {
        loop {
            match receiver.try_recv() {
                Ok(_) | Err(tinybus::TryRecvError::Lagged(_)) => continue,
                Err(_) => return,
            }
        }
    }

    let _inference_guard = crate::inference::inference_test_guard_async().await;
    let temp = tempfile::tempdir().expect("temporary configuration directory");
    let proxy = TcpListener::bind(("127.0.0.1", 0)).expect("local proxy trap listener");
    proxy
        .set_nonblocking(true)
        .expect("nonblocking proxy listener");
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    let proxy_requests = Arc::new(AtomicUsize::new(0));
    let server_requests = Arc::clone(&proxy_requests);
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let proxy_thread = std::thread::spawn(move || {
        while !server_stop.load(Ordering::SeqCst) {
            match proxy.accept() {
                Ok((mut stream, _)) => {
                    server_requests.fetch_add(1, Ordering::SeqCst);
                    let mut request = [0_u8; 512];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(
                        b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    let _proxy_trap = LocalProxyTrap {
        stop,
        thread: Some(proxy_thread),
    };
    let _env_guard = EnvVarGuard::locked_async()
        .await
        .with("HOME", temp.path())
        .with("OPENHUMAN_WORKSPACE", temp.path())
        .without("OPENHUMAN_ACTION_DIR")
        .with("HTTPS_PROXY", &proxy_url)
        .with("https_proxy", &proxy_url)
        .with("ALL_PROXY", &proxy_url)
        .with("all_proxy", &proxy_url)
        .with("NO_PROXY", "")
        .with("no_proxy", "")
        .without("OPENAI_API_KEY")
        .without("OPENHUMAN_CORE_TOKEN");

    let mut config = isolated_config(temp.path());
    config.secrets.encrypt = false;
    config.chat_provider = Some("ollama:qwen2.5".to_owned());
    config.vision_provider = Some("openai:gpt-4o".to_owned());
    config.primary_cloud = Some("provider-openai".to_owned());
    config.cloud_providers = vec![CloudProviderCreds {
        id: "provider-openai".to_owned(),
        slug: "openai".to_owned(),
        label: "OpenAI".to_owned(),
        endpoint: "https://api.openai.invalid/v1".to_owned(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("gpt-4o".to_owned()),
    }];
    config.resolved_endpoints.control_plane_url = Some("https://api.openhuman.invalid".to_owned());

    let profile_path = temp.path().join("auth-profiles.json");
    std::fs::write(
        &profile_path,
        serde_json::json!({
            "schema_version": 1,
            "active_profiles": {},
            "profiles": {
                "provider:openai:oauth": {
                    "provider": "provider:openai",
                    "profile_name": "oauth",
                    "kind": "oauth",
                    "access_token": "expired-openai-access-token",
                    "refresh_token": "refresh-trap-token",
                    "expires_at": "2000-01-01T00:00:00Z",
                    "metadata": {}
                },
                "app-session:default": {
                    "provider": "app-session",
                    "profile_name": "default",
                    "kind": "token",
                    "token": "expired-app-session-token",
                    "metadata": { "session_expires_at": "2000-01-01T00:00:00Z" }
                }
            }
        })
        .to_string(),
    )
    .expect("write isolated read-only credential fixture");
    let before = filesystem_snapshot(temp.path());

    crate::core::bus::init().await.expect("event bus init");
    let mut events = crate::core::bus::BUS
        .get()
        .expect("event bus initialized")
        .receiver();
    drain_events(&mut events);
    let error = match create_chat_model_with_model_id_in_for_inspection(
        &DispatchCtx::unbound(),
        "vision",
        &config,
        config.default_temperature,
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("an expired OAuth credential must not pass static inspection"),
    };
    let message = error.to_string();

    assert!(
        message.contains("OpenAI OAuth credential requires refresh"),
        "inspection should report the safe refresh classification: {message}"
    );
    assert!(!message.contains("expired-openai-access-token"));
    assert!(!message.contains("refresh-trap-token"));
    assert_eq!(proxy_requests.load(Ordering::SeqCst), 0);
    assert!(no_session_expiry_event(&mut events));
    assert_eq!(filesystem_snapshot(temp.path()), before);
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
        .without("OPENHUMAN_ACTION_DIR")
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
    let _registered_model = RegisterManagedModel;

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
    let findings =
        crate::core::runtime::CoreContext::scope(core_context, run_only(config, CONSTRUCT)).await;
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
async fn missing_byok_model_is_reported_by_the_native_construct_check() {
    let workspace = tempfile::tempdir().expect("create isolated config fixture");
    let mut config = isolated_config(workspace.path());
    config.default_model = None;
    config.primary_cloud = None;
    config.chat_provider = Some("doctor-empty-model:".to_owned());
    config.reasoning_provider = None;
    config.agentic_provider = None;
    config.coding_provider = None;
    config.vision_provider = None;
    config.memory_provider = None;
    config.cloud_providers = vec![CloudProviderCreds {
        id: "doctor-empty-model-provider".to_owned(),
        slug: "doctor-empty-model".to_owned(),
        label: "Empty model fixture".to_owned(),
        endpoint: "https://provider.example.invalid/v1".to_owned(),
        auth_style: AuthStyle::Bearer,
        default_model: None,
    }];

    let findings = run_only(config, CONSTRUCT).await;
    let result = finding(&findings, CONSTRUCT);

    assert_eq!(result.severity, FindingSeverity::Error);
    assert_eq!(result.config_path.as_deref(), Some("chat_provider"));
    assert!(result.message.contains("model is missing"));
    assert!(!result.message.contains("private"));
}

struct RegisterManagedModel;

impl Drop for RegisterManagedModel {
    fn drop(&mut self) {
        crate::config::schema::cloud_providers::register_hosted_cloud_provider_policy(
            Arc::new(|_| false),
            Arc::new(|_| false),
        );
        crate::inference::registry::unregister_inspection_chat_model(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        );
        crate::inference::registry::unregister_chat_model(
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        );
    }
}

#[tokio::test]
async fn embedding_only_configuration_does_not_construct_a_chat_model_for_embeddings() {
    use crate::inference::registry::{register_inspection_chat_model, InspectionChatModelFactory};

    let _inference_guard = crate::inference::inference_test_guard_async().await;

    let roles = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_roles = Arc::clone(&roles);
    let factory: InspectionChatModelFactory = Arc::new(move |request| {
        let seen_roles = Arc::clone(&seen_roles);
        Box::pin(async move {
            seen_roles.lock().unwrap().push(request.role.to_owned());
            anyhow::bail!("test construction marker")
        })
    });
    register_inspection_chat_model(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        factory,
    );
    let _registered_model = RegisterManagedModel;

    let mut config = Config::default();
    config.embeddings_provider = Some("custom".to_owned());
    config.primary_cloud = Some("openhuman".to_owned());
    let findings = run_only(config, CONSTRUCT).await;

    assert!(
        !roles
            .lock()
            .unwrap()
            .iter()
            .any(|role| role == "embeddings"),
        "embedding-only configuration must use embedding validation, never chat construction"
    );
    assert!(
        !findings.is_empty(),
        "the configured chat roles should still be inspected through their fake factory"
    );
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

#[tokio::test]
async fn invalid_role_and_route_temperatures_report_their_config_paths() {
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4o@-0.1".to_owned());
    config.model_routes = vec![crate::config::ModelRouteConfig {
        hint: "reasoning".to_owned(),
        model: "openai:o3@2.5".to_owned(),
    }];
    let findings = run_only(config, TEMPERATURE).await;
    let paths = findings
        .iter()
        .filter(|result| result.check_id == TEMPERATURE)
        .filter_map(|result| result.config_path.as_deref())
        .collect::<Vec<_>>();

    for expected in ["chat_provider", "model_routes[0].model"] {
        assert!(
            paths.contains(&expected),
            "missing {expected} temperature finding: {findings:#?}"
        );
    }
}
