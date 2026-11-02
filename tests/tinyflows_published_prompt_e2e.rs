//! Normal-host prompt composition against the checksum-verified TinyFlows v0.9.4 module.

#[path = "support/env_guard.rs"]
mod env_guard;
#[path = "support/published_tinyflows.rs"]
mod published_tinyflows;
#[path = "support/tinyhumans_boot.rs"]
mod tinyhumans_boot;

use env_guard::EnvVarGuard;
use std::path::Path;
use std::process::Command;
#[cfg(feature = "skills")]
use std::time::Duration;

fn dump_published_prompt(
    agent_id: &str,
    home: &Path,
    workspace: &Path,
    module_search_path: &Path,
) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args([
            "agent",
            "dump-prompt",
            "--agent",
            agent_id,
            "--workspace",
            workspace.to_str().expect("workspace path is UTF-8"),
            "--json",
            "--with-tools",
        ])
        .env("HOME", home)
        .env("OPENHUMAN_WORKSPACE", workspace)
        .env("OPENHUMAN_MODULE_PATH", module_search_path)
        .env_remove("BACKEND_URL")
        .env_remove("VITE_BACKEND_URL")
        .env_remove("OPENHUMAN_BACKEND_API_KEY")
        .env_remove("OPENHUMAN_BACKEND_SESSION_TOKEN")
        .env_remove("OPENHUMAN_STORAGE_URL")
        .env_remove("OPENHUMAN_MODULE_SOURCE")
        .env_remove("OPENHUMAN_MODULE_SOURCE_DIR")
        .env_remove("OPENHUMAN_MODULE_SOURCE_TINYFLOWS")
        .output()
        .expect("run the public prompt-dump command");
    assert!(
        output.status.success(),
        "prompt dump for {agent_id} failed ({}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("decode public prompt-dump JSON")
}

#[cfg(feature = "skills")]
async fn prewarm_flow_authoring_in_parent(
    ctx: &openhuman_core::core::runtime::DispatchCtx,
    config: &openhuman_core::config::Config,
) {
    // Use the same public session composition that schedules optional
    // flow-authoring prewarm during normal host construction. The prompt-dump
    // child cannot populate this process's module runtime or resource snapshot.
    let mut session = openhuman_core::agent::OpenHumanSessionHost::from_config_for_agent(
        ctx,
        config,
        "orchestrator",
    )
    .await
    .expect("build a normal host session to schedule resource prewarm");
    session.refresh_prompt_surface().await;

    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            let discovered = openhuman_core::skills::load_workflow_metadata_for(ctx, config);
            if discovered
                .iter()
                .any(|skill| skill.id() == "flow-authoring")
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the normal host prewarm should expose the published builtin");
}

#[tokio::test]
async fn published_module_prompts_render_through_the_normal_host_path() {
    let _env_lock = env_guard::env_lock_async().await;
    let temp = tempfile::tempdir().expect("isolated home and workspace");
    let _home_guard = EnvVarGuard::set_to_path("HOME", temp.path());
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_key_guard = EnvVarGuard::unset("OPENHUMAN_BACKEND_API_KEY");
    let _session_guard = EnvVarGuard::unset("OPENHUMAN_BACKEND_SESSION_TOKEN");
    let _storage_guard = EnvVarGuard::unset("OPENHUMAN_STORAGE_URL");
    let _source_guard = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE");
    let _source_dir_guard = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE_DIR");
    let _tinyflows_source_guard = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE_TINYFLOWS");
    let (library, _) = published_tinyflows::staged_v0_9_4_module();
    let module_search_path = library
        .parent()
        .expect("staged module artifact has a search directory");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace directory");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    let _module_path_guard = EnvVarGuard::set_to_path("OPENHUMAN_MODULE_PATH", module_search_path);
    let openhuman_home = temp.path().join(".openhuman");
    let local_config_dir = openhuman_home.join("users/local");
    std::fs::create_dir_all(&local_config_dir).expect("create isolated local config directory");
    let config_path = openhuman_home.join("config.toml");
    let local_config_path = local_config_dir.join("config.toml");
    let quoted_library = serde_json::to_string(
        library
            .to_str()
            .expect("published module path is valid UTF-8"),
    )
    .expect("TOML-compatible quoted path");
    let enabled_config = format!(
        "api_url = \"http://127.0.0.1:1\"\n\n[components]\ntinyflows = true\n\n[modules]\nenabled = true\n\n[[modules.overrides]]\nid = \"tinyflows\"\npath = {quoted_library}\n"
    );
    for path in [&config_path, &local_config_path] {
        std::fs::write(path, &enabled_config).expect("write enabled module override config");
    }

    // `agent dump-prompt` reads the install it is pointed at and builds the real
    // session agent, which needs a stored backend credential (`SESSION_EXPIRED`
    // otherwise). Install a local-only key the way a host does, through
    // `auth_set_credential`, under a per-run master key the child processes
    // inherit. `api_url` is unroutable, so nothing leaves the machine.
    let _master_key_guard =
        EnvVarGuard::set("OPENHUMAN_KEYRING_MASTER_KEY", "5a".repeat(32).as_str());
    let _keyring_backend_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let credential = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args([
            "call",
            "--method",
            "openhuman.auth_set_credential",
            "--params",
            r#"{"token":"th_test_local_only","kind":"api-key"}"#,
        ])
        .env("HOME", temp.path())
        .env("OPENHUMAN_WORKSPACE", &workspace)
        .env("OPENHUMAN_MODULE_PATH", module_search_path)
        .env_remove("BACKEND_URL")
        .env_remove("VITE_BACKEND_URL")
        .env_remove("OPENHUMAN_BACKEND_API_KEY")
        .env_remove("OPENHUMAN_BACKEND_SESSION_TOKEN")
        .output()
        .expect("install the local test credential");
    assert!(
        credential.status.success(),
        "installing the local test credential failed: {}",
        String::from_utf8_lossy(&credential.stderr)
    );

    for (agent_id, required, invariant) in [
        ("workflow_builder", "propose_workflow", "propose-only"),
        ("flow_discovery", "suggest_workflows", "read-only"),
    ] {
        let prompt = dump_published_prompt(agent_id, temp.path(), &workspace, module_search_path);
        let text = prompt["system_prompt"]
            .as_str()
            .expect("public prompt dump returns system_prompt text");
        let lower = text.to_lowercase();
        assert!(
            text.contains(required),
            "{agent_id} prompt missing {required}"
        );
        assert!(
            lower.contains(invariant),
            "{agent_id} prompt missing {invariant}"
        );
        assert!(text.contains("## Workspace"));
    }

    #[cfg(feature = "skills")]
    {
        // This is the actual published module path: the host must materialize the
        // bus-owned skill, then the normal TinySkills discovery and resource reader
        // must expose that Builtin under the separate dynamic root.
        let config =
            openhuman_core::config::Config::load_from_config_path(&config_path, &workspace)
                .await
                .expect("load the same scoped Config used by the prompt dump");
        // The in-process host session resolves its credential from this config's
        // own state directory, not the one the child processes used.
        openhuman_core::security::credentials::api_key::store_api_key(
            &config,
            "th_test_local_only",
        )
        .expect("store a local test credential for the in-process host session");
        let ctx = openhuman_core::core::runtime::DispatchCtx::host_default();
        tinyhumans_boot::boot();
        prewarm_flow_authoring_in_parent(&ctx, &config).await;
        assert!(openhuman_core::modules::runtime::is_module_serving(
            "tinyflows"
        ));
        let discovered = openhuman_core::skills::load_workflow_metadata_for(&ctx, &config);
        let flow = discovered
            .iter()
            .find(|skill| skill.id() == "flow-authoring")
            .expect("the published module's flow-authoring skill is discoverable");
        assert_eq!(flow.scope.precedence(), 0, "flow-authoring is a Builtin");
        let flow_dir = flow
            .location
            .as_deref()
            .and_then(std::path::Path::parent)
            .expect("discovered flow-authoring location");
        assert!(
            flow_dir.starts_with(workspace.join(".openhuman").join("builtin-flow-skills")),
            "the dynamic builtin uses the separate product root: {}",
            flow_dir.display()
        );
        assert!(
            !flow_dir.starts_with(workspace.join(".openhuman").join("builtin-skills")),
            "TinyFlows resources must not share the static desktop-control root"
        );
        for resource in &flow.resources {
            let body = openhuman_core::skills::read_workflow_resource_for(
                &ctx,
                &config,
                "flow-authoring",
                resource,
            )
            .unwrap_or_else(|error| {
                panic!("reading published flow-authoring resource {resource:?}: {error}")
            });
            assert!(!body.is_empty(), "published resource {resource:?} is empty");
        }

        let disabled_config_path = temp.path().join("disabled-config.toml");
        std::fs::write(
        &disabled_config_path,
        "api_url = \"http://127.0.0.1:1\"\n\n[components]\nskills = true\nflows = true\ntinyflows = true\n\n[modules]\nenabled = false\n",
    )
    .expect("disabled components config");
        let disabled_config = openhuman_core::config::Config::load_from_config_path(
            &disabled_config_path,
            &workspace,
        )
        .await
        .expect("load disabled profile config");
        let disabled = openhuman_core::skills::load_workflow_metadata_for(&ctx, &disabled_config);
        assert!(
            !disabled.iter().any(|skill| skill.id() == "flow-authoring"),
            "a profile with modules disabled cannot see the shared workspace builtin"
        );
        let disabled_text =
            std::fs::read_to_string(&disabled_config_path).expect("read disabled module config");
        for path in [&config_path, &local_config_path] {
            std::fs::write(path, &disabled_text).expect("disable module in active CLI config");
        }
        let ordinary_prompt =
            dump_published_prompt("orchestrator", temp.path(), &workspace, module_search_path);
        assert!(ordinary_prompt["system_prompt"]
            .as_str()
            .expect("ordinary prompt text")
            .contains("## Workspace"));

        // Re-enable the same profile after the disabled view and ensure the
        // async host preflight restores the module-backed catalogue.
        for path in [&config_path, &local_config_path] {
            std::fs::write(path, &enabled_config).expect("re-enable module in CLI config");
        }
        dump_published_prompt(
            "workflow_builder",
            temp.path(),
            &workspace,
            module_search_path,
        );
        prewarm_flow_authoring_in_parent(&ctx, &config).await;
        let reenabled = openhuman_core::skills::load_workflow_metadata_for(&ctx, &config);
        assert!(
            reenabled.iter().any(|skill| skill.id() == "flow-authoring"),
            "re-enabling TinyFlows restores the module-backed builtin"
        );

        // A live disk edit must invalidate both the cached metadata and the
        // resource-read path; cached Builtin metadata cannot bless changed bytes.
        let resource = flow.resources.first().expect("published reference files");
        std::fs::write(flow_dir.join(resource), "tampered resource").expect("tamper resource");
        let after_tamper = openhuman_core::skills::load_workflow_metadata_for(&ctx, &config);
        assert!(
            !after_tamper
                .iter()
                .any(|skill| skill.id() == "flow-authoring"),
            "a cache hit must not advertise tampered module resources"
        );
        assert!(
            openhuman_core::skills::read_workflow_resource_for(
                &ctx,
                &config,
                "flow-authoring",
                resource,
            )
            .is_err(),
            "the resource reader must refuse tampered module bytes"
        );
    }
}

#[tokio::test]
async fn published_module_public_import_uses_authoring_bus_in_a_cold_process() {
    const CHILD_ENV: &str = "OPENHUMAN_TINYFLOWS_IMPORT_CHILD";
    if std::env::var_os(CHILD_ENV).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "published_module_public_import_uses_authoring_bus_in_a_cold_process",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run isolated published-module authoring test");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        print!("{stdout}");
        eprint!("{stderr}");
        let child_output = format!("{stdout}{stderr}");
        assert!(
            output.status.success(),
            "published authoring child failed:\n{child_output}"
        );
        assert!(
            child_output.contains("running 1 test")
                && child_output.contains(
                    "published_module_public_import_uses_authoring_bus_in_a_cold_process ... ok"
                )
                && child_output.contains("1 passed; 0 failed"),
            "child must prove exactly one public-import test ran:\n{child_output}"
        );
        return;
    }

    let _env_lock = env_guard::env_lock_async().await;
    let temp = tempfile::tempdir().expect("isolated home and workspace");
    let _home_guard = EnvVarGuard::set_to_path("HOME", temp.path());
    let _backend_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _api_key_guard = EnvVarGuard::unset("OPENHUMAN_BACKEND_API_KEY");
    let _session_guard = EnvVarGuard::unset("OPENHUMAN_BACKEND_SESSION_TOKEN");
    let _storage_guard = EnvVarGuard::unset("OPENHUMAN_STORAGE_URL");
    let _source_guard = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE");
    let _source_dir_guard = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE_DIR");
    let _tinyflows_source_guard = EnvVarGuard::unset("OPENHUMAN_MODULE_SOURCE_TINYFLOWS");
    let (library, digest) = published_tinyflows::staged_v0_9_4_module();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace directory");
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    let quoted_library = serde_json::to_string(library.to_str().expect("module path UTF-8"))
        .expect("quoted module path");
    let config_path = temp.path().join("config.toml");
    std::fs::write(
        &config_path,
        format!(
            "api_url = \"http://127.0.0.1:1\"\n\n[components]\ntinyflows = true\n\n[modules]\nenabled = true\n\n[[modules.overrides]]\nid = \"tinyflows\"\npath = {quoted_library}\n"
        ),
    )
    .expect("module override config");
    let config = openhuman_core::config::Config::load_from_config_path(&config_path, &workspace)
        .await
        .expect("load isolated config");
    assert_eq!(
        openhuman_core::modules::state_of("tinyflows"),
        openhuman_core::modules::ModuleState::Available,
        "cold process must begin with TinyFlows unresolved"
    );

    let imported = openhuman_core::flows::ops::flows_import(
        &openhuman_core::core::runtime::DispatchCtx::unbound(),
        &config,
        serde_json::json!({
            "name": "published import",
            "nodes": [{"id":"trigger","kind":"trigger","name":"Trigger"}],
            "edges": []
        }),
        Some("native".into()),
    )
    .await
    .expect("public import must call the published Authoring.ImportGraph member");
    assert_eq!(imported.value.graph["name"], "published import");
    assert_eq!(
        openhuman_core::modules::state_of("tinyflows"),
        openhuman_core::modules::ModuleState::Ready,
        "public import must load the checksum-verified published module (sha256 {digest})"
    );

    // Catalog.ApprovalManifest owns migration of graph JSON too. This legacy
    // null schema version was accepted by the previous public operation and
    // must still reach the invocation-scoped host classifier over TinyBus.
    let ctx = openhuman_core::core::runtime::DispatchCtx::unbound();
    let legacy_graph = serde_json::json!({
        "schema_version": null,
        "name": "legacy approval manifest",
        "nodes": [
            {"id":"trigger","kind":"trigger","name":"Trigger"},
            {"id":"request","kind":"http_request","name":"Request","config":{"url":"https://example.test","method":"GET"}}
        ],
        "edges": [{"from_node":"trigger","to_node":"request"}]
    });
    let manifest = openhuman_core::flows::ops::flows_approval_manifest(
        &ctx,
        &config,
        None,
        Some(legacy_graph.clone()),
    )
    .await
    .expect("published Catalog.ApprovalManifest must migrate and classify the raw graph");
    assert!(manifest.value["entries"].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry["tool_name"] == "flows_http_request" && entry["kind"] == "approvable"
        })
    }));

    let mut disabled = config.clone();
    disabled.components.tinyflows = false;
    let error = openhuman_core::flows::ops::flows_approval_manifest(
        &ctx,
        &disabled,
        None,
        Some(legacy_graph),
    )
    .await
    .expect_err(
        "a failed Catalog operation must propagate instead of yielding an empty grant list",
    );
    assert!(error.contains("COMPONENT_DISABLED"), "{error}");
}
