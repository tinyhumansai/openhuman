use super::*;
use crate::config::migrations::{run_pending, CURRENT_SCHEMA_VERSION};
use std::fs;
use tempfile::TempDir;

#[tokio::test]
async fn run_pending_v3_to_v4_preserves_custom_max_actions() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    simulate_v3_autonomy(&mut config);
    // User has deliberately configured a specific ceiling.
    config.security.autonomy.max_actions_per_hour = 50;

    run_pending(&mut config).await;

    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        config.security.autonomy.max_actions_per_hour, 50,
        "user-customised max_actions_per_hour must not be overwritten"
    );
}

/// The coverage gap that let the allowlist-widening ship unnoticed.
///
/// Existing tests start at v3 (a real migration) or at a fresh install (already
/// stamped `CURRENT_SCHEMA_VERSION`). Neither covers the case a user actually
/// hits: a **hand-written** `config.toml` with a deliberately narrow
/// `allowed_commands` and no `schema_version` field, which `#[serde(default)]`
/// loads as version `0`.
///
/// This test documents what happens today rather than what should. It is not an
/// endorsement: an additive merge structurally cannot tell "absent because the
/// user removed it" from "absent because it post-dates this config", so
/// deciding whether a v0 config with an explicit allowlist should be migrated at
/// all is a product ruling, not a refactor. Pinning current behaviour means that
/// ruling, when it comes, has to walk past this assertion.
#[tokio::test]
async fn run_pending_widens_a_hand_written_v0_allowlist_and_that_is_visible() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    // A hand-written file: no `schema_version` key at all -> serde default 0.
    config.schema_version = 0;
    config.security.autonomy.allowed_commands = vec!["ls".to_string(), "cat".to_string()];

    run_pending(&mut config).await;

    assert_eq!(
        config.schema_version, CURRENT_SCHEMA_VERSION,
        "the whole 0 -> current chain runs against a config that was never at v3"
    );

    // The user's own entries survive — the merge is additive.
    for kept in &["ls", "cat"] {
        assert!(
            config
                .security
                .autonomy
                .allowed_commands
                .iter()
                .any(|c| c == kept),
            "{kept} was configured explicitly and must survive"
        );
    }

    // …and so do 28 commands the user never asked for, five of which mutate the
    // filesystem. This is the security-relevant half.
    for added in &["mkdir", "touch", "cp", "mv", "ln"] {
        assert!(
            config
                .security
                .autonomy
                .allowed_commands
                .iter()
                .any(|c| c == added),
            "current behaviour: the v3->v4 migration adds {added} to a hand-written \
             allowlist that never contained it"
        );
    }
    assert!(
        config.security.autonomy.allowed_commands.len() > 2,
        "the configured list of 2 is widened, not preserved: {:?}",
        config.security.autonomy.allowed_commands
    );
}

/// Setting `schema_version` explicitly is the documented opt-out, and it works.
#[tokio::test]
async fn an_explicit_schema_version_stops_the_allowlist_being_widened() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = CURRENT_SCHEMA_VERSION;
    config.security.autonomy.allowed_commands = vec!["ls".to_string(), "cat".to_string()];

    run_pending(&mut config).await;

    assert_eq!(
        config.security.autonomy.allowed_commands,
        vec!["ls".to_string(), "cat".to_string()],
        "with the gate satisfied no migration runs, so the curated list is untouched"
    );
}

/// The empty-allowlist case the first revision of the WARN guard silenced.
///
/// `allowed_commands = []` is a deny-all shell policy — the strongest curated
/// choice available — and it is precisely the config that must not be widened
/// quietly. Pinned separately from the two-entry case because an earlier guard
/// treated "empty" as "no opinion" and skipped the warning for exactly these
/// users.
#[tokio::test]
async fn a_hand_written_empty_allowlist_is_also_widened() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 0;
    config.security.autonomy.allowed_commands = Vec::new();

    run_pending(&mut config).await;

    assert!(
        !config.security.autonomy.allowed_commands.is_empty(),
        "current behaviour: a deny-all list is widened by the v3->v4 migration"
    );
    for added in &["mkdir", "touch", "cp", "mv", "ln"] {
        assert!(
            config
                .security
                .autonomy
                .allowed_commands
                .iter()
                .any(|c| c == added),
            "an explicitly empty allowlist still gains {added}"
        );
    }
}

/// The stuck-workspace case the 10 -> 11 step exists for.
///
/// `unify_ai_provider_settings` is the only code that seeds the managed
/// `openhuman` entry, and its step is guarded on `schema_version == 1`. A
/// workspace that reached a later version without the entry could therefore
/// never acquire one: observed on every workspace on a developer machine (two
/// production users and one staging), all at `schema_version = 10` with
/// `cloud_providers = []`.
///
/// The user-visible consequence is that `inference_list_models("openhuman")`
/// fails its provider lookup before making any HTTP request, so the model
/// picker's managed pane renders "Could not load models from this provider."
#[tokio::test]
async fn a_v10_workspace_with_no_cloud_providers_is_reseeded() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 10;
    config.cloud_providers = Vec::new();

    run_pending(&mut config).await;

    assert!(
        config.cloud_providers.iter().any(|e| e.slug == "openhuman"),
        "a v10 workspace with an empty provider list must gain the managed entry, \
         otherwise the model picker can never load the managed catalog"
    );
    assert_eq!(
        config.schema_version, CURRENT_SCHEMA_VERSION,
        "the re-seed step must advance the schema version"
    );
}

/// The re-seed must not touch a workspace that already has providers.
///
/// `seed_cloud_providers` early-returns on a non-empty list, so this is the
/// guard that keeps the new step a no-op for every healthy install — including
/// one whose only entry is a BYO provider the user configured themselves.
#[tokio::test]
async fn a_v10_workspace_that_already_has_providers_is_left_alone() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("workspace")).unwrap();

    let mut config = config_in(&tmp);
    config.schema_version = 10;
    config.cloud_providers = vec![crate::config::schema::cloud_providers::CloudProviderCreds {
        id: "prov_user_owned".to_string(),
        slug: "custom".to_string(),
        label: "My own endpoint".to_string(),
        endpoint: "https://llm.example.com/v1".to_string(),
        ..Default::default()
    }];

    run_pending(&mut config).await;

    assert_eq!(
        config.cloud_providers.len(),
        1,
        "an existing provider list must not be added to"
    );
    assert_eq!(config.cloud_providers[0].id, "prov_user_owned");
}
