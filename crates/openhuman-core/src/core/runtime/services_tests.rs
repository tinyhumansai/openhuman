use super::*;

/// desktop() must enable every bootstrap job.
#[test]
fn desktop_plan_enables_every_job() {
    let plan = bootstrap_job_plan(&ServiceSet::desktop());
    assert_eq!(
        plan,
        BootstrapJobPlan {
            composio_integration_sync: true,
            memory_jobs: true,
            task_source_pollers: true,
            module_preload: true,
        }
    );
}

/// none() / headless_api() run no bootstrap job at all.
#[test]
fn job_free_presets_enable_nothing() {
    let empty = BootstrapJobPlan {
        composio_integration_sync: false,
        memory_jobs: false,
        task_source_pollers: false,
        module_preload: false,
    };
    assert_eq!(bootstrap_job_plan(&ServiceSet::none()), empty);
    assert_eq!(bootstrap_job_plan(&ServiceSet::headless_api()), empty);
}

/// From none(), flipping exactly one concern flag enables exactly its job.
#[test]
fn each_concern_flag_enables_exactly_its_job() {
    let mut integrations = ServiceSet::none();
    integrations.integrations = true;
    let plan = bootstrap_job_plan(&integrations);
    assert!(plan.composio_integration_sync);
    assert!(!plan.memory_jobs);
    assert!(!plan.task_source_pollers);

    let mut memory_sync = ServiceSet::none();
    memory_sync.memory_sync = true;
    let plan = bootstrap_job_plan(&memory_sync);
    assert!(plan.memory_jobs);
    assert!(!plan.composio_integration_sync);
    assert!(!plan.task_source_pollers);
}

/// From desktop(), disabling exactly one concern flag disables only its job.
#[test]
fn disabling_one_concern_disables_only_its_job() {
    let mut services = ServiceSet::desktop();
    services.integrations = false;
    let plan = bootstrap_job_plan(&services);
    assert!(!plan.composio_integration_sync);
    assert!(plan.memory_jobs);
    assert!(plan.task_source_pollers);

    let mut services = ServiceSet::desktop();
    services.memory_sync = false;
    let plan = bootstrap_job_plan(&services);
    assert!(!plan.memory_jobs);
    assert!(plan.composio_integration_sync);
}

/// `channels` gates NO bootstrap job.
#[test]
fn channels_flag_gates_no_bootstrap_job() {
    let mut channels_only = ServiceSet::none();
    channels_only.channels = true;
    assert_eq!(
        bootstrap_job_plan(&channels_only),
        bootstrap_job_plan(&ServiceSet::none()),
        "channels alone must enable no bootstrap job"
    );

    let mut channels_off = ServiceSet::desktop();
    channels_off.channels = false;
    assert_eq!(
        bootstrap_job_plan(&channels_off),
        bootstrap_job_plan(&ServiceSet::desktop()),
        "dropping channels must not drop any bootstrap job"
    );
}
