use super::*;

#[tokio::test]
async fn selected_profile_reports_preserve_parent_check_sources_without_using_parent_state() {
    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _workspace_override = WorkspaceOverrideGuard::clear();
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let account_a = ProfileLayout::new(root, "account-a");
    let account_b = ProfileLayout::new(root, "account-b");
    tokio::fs::create_dir_all(&account_a.dir).await.unwrap();
    tokio::fs::create_dir_all(&account_b.dir).await.unwrap();
    tokio::fs::write(
        &account_a.config_path,
        "api_url = \"https://account-a.example\"\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        &account_b.config_path,
        "api_url = \"https://account-b.example\"\n[components]\nmemory = false\n",
    )
    .await
    .unwrap();

    let mut account_a_config = Config {
        config_path: account_a.config_path.clone(),
        workspace_dir: account_a.workspace_dir.clone(),
        api_url: Some("https://account-a.example".into()),
        ..Config::default()
    };
    account_a_config.components.memory = true;
    let parent = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::kernel(),
        account_a_config.clone(),
    )
    .derive_with(
        crate::core::runtime::ContextOverlay::new(
            account_a_config,
            crate::core::runtime::DomainSet::kernel(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .without_user_skill_roots(),
    );

    let observed = Arc::new(Mutex::new(Vec::new()));
    let parent_state = parent.agent_state().slot::<InspectionStateSentinel>();
    crate::doctor::register_check_source_in(
        parent.extensions(),
        SelectedProfileContextSource {
            observed: observed.clone(),
            parent_state: Some(parent_state),
        },
    );
    let before = snapshot_tree(root);
    let explicit_outcome = crate::core::runtime::CoreContext::scope(parent.clone(), async {
        crate::doctor::ops::doctor_report_for_selected_profile(
            root,
            "account-b",
            selected_profile_context_options(),
        )
        .await
    })
    .await
    .unwrap();
    let selection = crate::core::runtime::CoreContext::scope(parent.clone(), async {
        resolve_desktop_profile_for_doctor(root, Some("account-b")).await
    })
    .await
    .unwrap();
    let selection_outcome = crate::core::runtime::CoreContext::scope(parent.clone(), async {
        crate::doctor::ops::doctor_report_for_desktop_selection(
            selection,
            selected_profile_context_options(),
        )
        .await
    })
    .await
    .unwrap();

    for report in [explicit_outcome.value, selection_outcome.value] {
        assert!(!report
            .findings
            .iter()
            .any(|finding| finding.check_id == "doctor/selection"));
        assert!(report
            .checks_run
            .iter()
            .any(|id| id == "hosted/selected-profile-context"));
        assert_eq!(report.profile.as_deref(), Some("account-b"));
    }
    let observations = observed.lock().unwrap();
    assert_eq!(observations.len(), 2);
    for observation in observations.iter() {
        assert_eq!(observation.doctor_workspace, account_b.workspace_dir);
        assert_eq!(observation.doctor_config_path, account_b.config_path);
        assert_eq!(
            observation.doctor_api_url.as_deref(),
            Some("https://account-b.example")
        );
        assert_eq!(observation.ambient_workspace, account_b.workspace_dir);
        assert_eq!(observation.domains, {
            let mut domains = crate::core::runtime::DomainSet::kernel();
            domains.memory = false;
            domains
        });
        assert_eq!(
            observation.tool_groups,
            crate::tools::toolpacks::ToolGroups::none()
        );
        assert!(!observation.user_skill_roots);
        assert!(!observation.components.memory);
        assert!(!observation.ambient_components.memory);
        assert_eq!(observation.tenant_profile, None);
        assert!(!observation.state_shared_with_parent);
    }
    assert_eq!(before, snapshot_tree(root));
}

#[tokio::test]
async fn selected_profile_is_ambient_for_process_sources_without_a_parent_context() {
    const CHILD_MARKER: &str = "OPENHUMAN_DOCTOR_NO_PARENT_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "doctor::profile::tests::inspection::selected_profile_is_ambient_for_process_sources_without_a_parent_context",
                "--nocapture",
            ])
            .env(CHILD_MARKER, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let _env_guard = crate::config::TEST_ENV_LOCK.lock().await;
    let _workspace_override = WorkspaceOverrideGuard::clear();
    assert!(crate::core::runtime::CoreContext::current().is_none());

    let temp = TempDir::new().unwrap();
    let layout = ProfileLayout::new(temp.path(), "account-b");
    tokio::fs::create_dir_all(&layout.dir).await.unwrap();
    tokio::fs::write(
        &layout.config_path,
        "api_url = \"https://account-b.example\"\n[components]\nmemory = false\n",
    )
    .await
    .unwrap();

    let observed = Arc::new(Mutex::new(Vec::new()));
    let source_id = "doctor-selected-profile-process-context";
    crate::doctor::register_check_source(ProcessSelectedProfileContextSource(observed.clone()));
    let outcome = crate::doctor::ops::doctor_report_for_selected_profile(
        temp.path(),
        "account-b",
        DoctorOptions {
            tier: DoctorTier::Static,
            only: vec!["hosted/process-selected-profile-context".into()],
            ..DoctorOptions::default()
        },
    )
    .await;
    crate::doctor::registry::unregister_check_source(source_id);
    let report = outcome.unwrap().value;

    assert!(
        report
            .checks_run
            .iter()
            .any(|id| id == "hosted/process-selected-profile-context"),
        "checks_run={:?}, findings={:?}",
        report.checks_run,
        report.findings
    );
    let observations = observed.lock().unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(
        observations[0].ambient_workspace.as_deref(),
        Some(layout.workspace_dir.as_path())
    );
    assert_eq!(observations[0].tenant_profile, None);
    assert_eq!(observations[0].ambient_memory_component, Some(false));
}

struct ProcessSelectedProfileContextSource(Arc<Mutex<Vec<ProcessSelectedProfileObservation>>>);

struct ProcessSelectedProfileObservation {
    ambient_workspace: Option<std::path::PathBuf>,
    tenant_profile: Option<String>,
    ambient_memory_component: Option<bool>,
}

struct ProcessSelectedProfileContextCheck(Arc<Mutex<Vec<ProcessSelectedProfileObservation>>>);

#[async_trait::async_trait]
impl DoctorCheck for ProcessSelectedProfileContextCheck {
    fn id(&self) -> &str {
        "hosted/process-selected-profile-context"
    }

    fn category(&self) -> &str {
        "hosted"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, _ctx: &DoctorCtx) -> Vec<Finding> {
        let ambient_workspace = crate::core::runtime::CoreContext::current_embedder_config()
            .map(|config| config.workspace_dir);
        let tenant_profile = crate::core::runtime::current_tenant()
            .ok()
            .and_then(|tenant| tenant.profile);
        let ambient_memory_component =
            crate::core::runtime::CoreContext::current().map(|context| context.components().memory);
        self.0
            .lock()
            .unwrap()
            .push(ProcessSelectedProfileObservation {
                ambient_workspace,
                tenant_profile,
                ambient_memory_component,
            });
        vec![Finding::pass(
            self.id(),
            "The selected profile is ambient to host checks.",
        )]
    }
}

impl DoctorCheckSource for ProcessSelectedProfileContextSource {
    fn id(&self) -> &str {
        "doctor-selected-profile-process-context"
    }

    fn checks(&self) -> Vec<Arc<dyn DoctorCheck>> {
        vec![Arc::new(ProcessSelectedProfileContextCheck(self.0.clone()))]
    }
}

fn selected_profile_context_options() -> DoctorOptions {
    DoctorOptions {
        tier: DoctorTier::Static,
        only: vec!["hosted/selected-profile-context".into()],
        ..DoctorOptions::default()
    }
}

struct SelectedProfileContextObservation {
    doctor_workspace: std::path::PathBuf,
    doctor_config_path: std::path::PathBuf,
    doctor_api_url: Option<String>,
    ambient_workspace: std::path::PathBuf,
    domains: crate::core::runtime::DomainSet,
    tool_groups: crate::tools::toolpacks::ToolGroups,
    user_skill_roots: bool,
    components: crate::core::runtime::ComponentSet,
    ambient_components: crate::core::runtime::ComponentSet,
    tenant_profile: Option<String>,
    state_shared_with_parent: bool,
}

#[derive(Default)]
struct InspectionStateSentinel;

struct SelectedProfileContextCheck {
    observed: Arc<Mutex<Vec<SelectedProfileContextObservation>>>,
    parent_state: Option<Arc<InspectionStateSentinel>>,
}

#[async_trait::async_trait]
impl DoctorCheck for SelectedProfileContextCheck {
    fn id(&self) -> &str {
        "hosted/selected-profile-context"
    }

    fn category(&self) -> &str {
        "hosted"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let current = crate::core::runtime::CoreContext::current().unwrap();
        let ambient = crate::core::runtime::CoreContext::current_embedder_config().unwrap();
        let context_state = current.agent_state().slot::<InspectionStateSentinel>();
        let tenant_profile = crate::core::runtime::current_tenant()
            .ok()
            .and_then(|tenant| tenant.profile);
        self.observed
            .lock()
            .unwrap()
            .push(SelectedProfileContextObservation {
                doctor_workspace: ctx.config().workspace_dir.clone(),
                doctor_config_path: ctx.config().config_path.clone(),
                doctor_api_url: ctx.config().api_url.clone(),
                ambient_workspace: ambient.workspace_dir,
                domains: current.domains(),
                tool_groups: current.tool_groups(),
                user_skill_roots: current.user_skill_roots(),
                components: ctx.components(),
                ambient_components: current.components(),
                tenant_profile,
                state_shared_with_parent: self
                    .parent_state
                    .as_ref()
                    .is_some_and(|parent| Arc::ptr_eq(parent, &context_state)),
            });
        vec![Finding::pass(
            self.id(),
            "The selected profile context and host check source are available.",
        )]
    }
}

struct SelectedProfileContextSource {
    observed: Arc<Mutex<Vec<SelectedProfileContextObservation>>>,
    parent_state: Option<Arc<InspectionStateSentinel>>,
}

impl DoctorCheckSource for SelectedProfileContextSource {
    fn id(&self) -> &str {
        "doctor-selected-profile-context"
    }

    fn checks(&self) -> Vec<Arc<dyn DoctorCheck>> {
        vec![Arc::new(SelectedProfileContextCheck {
            observed: self.observed.clone(),
            parent_state: self.parent_state.clone(),
        })]
    }
}
