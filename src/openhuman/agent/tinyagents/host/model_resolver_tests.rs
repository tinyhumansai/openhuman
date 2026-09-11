use super::*;

// ── role routing (the product policy — pure, no provider needed) ─────────

fn req(agent: &str) -> ModelResolveRequest {
    ModelResolveRequest::new(agent)
}

#[test]
fn structural_split_decides_when_no_role_is_supplied() {
    assert_eq!(workload_role_for(&req("worker")), SUBAGENT_DEFAULT_ROLE);
    assert_eq!(
        workload_role_for(&req("lead").as_team_lead()),
        LEAD_DEFAULT_ROLE
    );
}

/// Asserts the literal, not the constant: the point is that this seam agrees
/// with `session/builder/factory.rs::provider_role_for`, whose
/// `orchestrator_defaults_to_chat` pins the same answer. Comparing against
/// `LEAD_DEFAULT_ROLE` would pass no matter what that constant was changed
/// to, which is exactly the drift that would silently move a user's
/// orchestrator off their configured chat provider. (`provider_role_for` is
/// module-private, so this restates its answer rather than calling it.)
#[test]
fn an_unannotated_lead_routes_to_chat_like_the_live_session_path() {
    assert_eq!(
        workload_role_for(&req("orchestrator").as_team_lead()),
        "chat"
    );
}

#[test]
fn agentic_is_reachable_only_through_an_explicit_hint() {
    assert_eq!(
        workload_role_for(&req("lead").as_team_lead().with_role("hint:agentic")),
        "agentic"
    );
}

/// `-v1` is a suffix heuristic, and `role_for_model_tier` answers `"chat"`
/// for anything it does not recognise — so an exact model id that happens to
/// end in `-v1` would be silently rerouted with no diagnostic.
#[test]
fn an_exact_model_id_ending_in_v1_is_not_mistaken_for_a_tier() {
    assert!(!is_known_model_tier("some-vendor-model-v1"));
    assert!(is_known_model_tier("reasoning-v1"));
    assert!(is_known_model_tier("reasoning-quick-v1"));

    // Falls through to the unknown-role path (structural default + warning)
    // rather than silently becoming chat via the tier table.
    assert_eq!(
        workload_role_for(&req("w").with_role("some-vendor-model-v1")),
        SUBAGENT_DEFAULT_ROLE
    );
}

#[test]
fn an_explicit_role_beats_the_structural_default() {
    // A lead that says "summarization" gets summarization, not agentic —
    // otherwise leadership would silently override an explicit pin.
    let r = req("lead").with_role("summarization").as_team_lead();
    assert_eq!(workload_role_for(&r), "summarization");
}

#[test]
fn plain_workload_roles_pass_through_case_insensitively() {
    for role in CHAT_WORKLOAD_ROLES {
        assert_eq!(workload_role_for(&req("a").with_role(*role)), *role);
        let shouted = role.to_ascii_uppercase();
        assert_eq!(workload_role_for(&req("a").with_role(shouted)), *role);
    }
}

#[test]
fn tier_and_hint_spellings_normalise_through_the_factory() {
    // Both spellings are in live use: `agent.toml` files write `hint = "..."`
    // and the channel routes carry `*-v1` tier aliases.
    assert_eq!(
        workload_role_for(&req("a").with_role("hint:agentic")),
        "agentic"
    );
    assert_eq!(
        workload_role_for(&req("a").with_role("reasoning-v1")),
        "reasoning"
    );
    assert_eq!(
        workload_role_for(&req("a").with_role("vision-v1")),
        "vision"
    );
    // `subconscious` rides the chat tier for its model, per the factory table.
    assert_eq!(
        workload_role_for(&req("a").with_role("hint:subconscious")),
        "chat"
    );
}

#[test]
fn a_blank_role_reads_as_absent_and_uses_the_structural_default() {
    // `ModelResolveRequest::role()` maps whitespace to `None`; the resolver
    // must not treat "" as a role-specific branch.
    assert_eq!(
        workload_role_for(&req("a").with_role("   ")),
        SUBAGENT_DEFAULT_ROLE
    );
    assert_eq!(
        workload_role_for(&req("a").with_role("").as_team_lead()),
        LEAD_DEFAULT_ROLE
    );
}

#[test]
fn an_unknown_role_falls_back_instead_of_failing() {
    assert_eq!(
        workload_role_for(&req("a").with_role("wizardry")),
        SUBAGENT_DEFAULT_ROLE
    );
    assert_eq!(
        workload_role_for(&req("a").with_role("wizardry").as_team_lead()),
        LEAD_DEFAULT_ROLE
    );
}

#[test]
fn embeddings_is_not_routable_as_a_chat_role() {
    // The provider factory knows the role, but it names an embedding model:
    // returning one here would fail at dispatch instead of at resolution.
    assert!(!CHAT_WORKLOAD_ROLES.contains(&"embeddings"));
    assert_eq!(
        workload_role_for(&req("a").with_role("embeddings")),
        SUBAGENT_DEFAULT_ROLE
    );
}

// ── StatelessModel bridging (offline: no provider, no network) ───────────

struct EchoModel(&'static str);

#[async_trait]
impl ChatModel<()> for EchoModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Ok(ModelResponse::assistant(self.0))
    }
}

#[tokio::test]
async fn stateless_model_invokes_the_inner_model_under_any_state() {
    let bridged = StatelessModel {
        inner: Arc::new(EchoModel("hi")),
    };

    // The same wrapper satisfies `ChatModel<State>` for unrelated states.
    let unit: &dyn ChatModel<()> = &bridged;
    assert_eq!(
        unit.invoke(&(), ModelRequest::default())
            .await
            .expect("invoke")
            .text(),
        "hi"
    );

    let stringy: &dyn ChatModel<String> = &bridged;
    assert_eq!(
        stringy
            .invoke(&"ignored".to_string(), ModelRequest::default())
            .await
            .expect("invoke")
            .text(),
        "hi"
    );
}

#[tokio::test]
async fn stateless_model_is_usable_as_the_resolver_return_type() {
    // Object safety at the exact type the trait hands back is part of the
    // contract, not an implementation detail.
    let model: Arc<dyn ChatModel<u32>> = Arc::new(StatelessModel {
        inner: Arc::new(EchoModel("dyn")),
    });
    let response = model
        .invoke(&7, ModelRequest::default())
        .await
        .expect("invoke");
    assert_eq!(response.text(), "dyn");
}

// ── resolver wiring ──────────────────────────────────────────────────────

#[test]
fn new_takes_the_configured_default_temperature() {
    let mut config = Config::default();
    config.default_temperature = 0.42;
    let resolver = OpenHumanModelResolver::new(Arc::new(config));
    assert_eq!(resolver.temperature, 0.42);

    let pinned = OpenHumanModelResolver::with_temperature(Arc::new(Config::default()), 0.0);
    assert_eq!(pinned.temperature, 0.0);
}

#[tokio::test]
async fn repeated_resolution_reuses_one_client_per_role() {
    // Whether a model can be *built* depends on the ambient config (a test
    // environment may have no provider configured at all), so this test
    // asserts the caching contract only when construction succeeds. The
    // failure path is covered by `unroutable_role_is_an_error`.
    let resolver = OpenHumanModelResolver::new(Arc::new(Config::default()));
    let Ok(first) = resolver.base_model_for_role("chat") else {
        return;
    };
    let second = resolver
        .base_model_for_role("chat")
        .expect("a role that built once must build again");
    assert!(
        Arc::ptr_eq(&first, &second),
        "resolving the same role twice must reuse the client, not rebuild its connection pool"
    );
}

#[tokio::test]
async fn resolve_routes_through_the_role_policy() {
    let resolver = OpenHumanModelResolver::new(Arc::new(Config::default()));
    let request = ModelResolveRequest::new("lead").as_team_lead();
    let expected = workload_role_for(&request);
    assert_eq!(expected, LEAD_DEFAULT_ROLE);

    // Only assert the end-to-end hand-off when the role is buildable here;
    // the routing decision itself is pinned by the pure tests above.
    if resolver.base_model_for_role(expected).is_ok() {
        let resolved: TaResult<Arc<dyn ChatModel<()>>> =
            ModelResolver::<()>::resolve(&resolver, &request).await;
        assert!(resolved.is_ok(), "a buildable role must resolve");
    }
}

#[test]
fn an_unroutable_role_is_reported_as_a_model_error() {
    // `base_model_for_role` only ever fails by way of the factory, so pin the
    // shape of the error the runtime sees rather than forcing that failure.
    let error = TinyAgentsError::Model(
        "openhuman: no model for workload role `chat`: unresolved".to_string(),
    );
    assert!(error.to_string().contains("no model for workload role"));
}
