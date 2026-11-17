use super::*;

use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use crate::tools::toolpacks::ToolGroups;

const WORKSPACE_COUNT: usize = 8;
const FD_TOLERANCE: usize = 4;

fn context(config: &Config, profile: &str) -> DispatchCtx {
    let root = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    DispatchCtx::new(
        root.derive_with(
            ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::default())
                .profile(profile),
        ),
    )
}

fn open_fd_count() -> usize {
    std::fs::read_dir("/dev/fd")
        .expect("Unix exposes the current process descriptor directory")
        .count()
}

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn workspace_fd_lifetime_is_separate_from_catalog_callback_lifetime() {
    if crate::util::module_test_process::run(
        module_path!(),
        "workspace_fd_lifetime_is_separate_from_catalog_callback_lifetime",
    ) {
        return;
    }

    // Warm the process-wide bus before measuring. Serving additional callback
    // objects should only add entries to that shared bus, not open a backend.
    crate::modules::runtime::runtime()
        .await
        .expect("process module runtime starts");
    crate::storage::connect_on_storage_runtime(|| async {})
        .await
        .expect("process storage runtime starts");
    let before = open_fd_count();

    let mut workspaces = Vec::with_capacity(WORKSPACE_COUNT);
    let mut registrations = Vec::with_capacity(WORKSPACE_COUNT);
    for index in 0..WORKSPACE_COUNT {
        let workspace = tempfile::tempdir().expect("isolated workspace");
        let config = Config {
            workspace_dir: workspace.path().to_path_buf(),
            ..Config::default()
        };
        crate::storage::local::use_persistent_default_for_test(workspace.path());
        let context = context(&config, &format!("fd-probe-{index}"));
        registrations.push(
            register(&context, &config)
                .await
                .expect("scope callback registers without opening a workspace backend"),
        );
        workspaces.push(workspace);
    }
    let after_registrations = open_fd_count();
    assert!(
        after_registrations <= before + FD_TOLERANCE,
        "idle callback registration opened OS descriptors: before={before}, after={after_registrations}"
    );

    // The existing default storage cache intentionally keeps a backend per
    // workspace. Exercise that separate owner, then use its existing release
    // API and retain TempDirs until every cached backend has been dropped.
    for workspace in &workspaces {
        let backend = crate::storage::local::backend_for(workspace.path())
            .expect("open the temporary workspace default backend");
        assert!(workspace.path().join("storage").exists());
        drop(backend);
    }
    let after_backend_opens = open_fd_count();
    assert!(
        after_backend_opens >= after_registrations + WORKSPACE_COUNT,
        "opening one SQLite database per workspace should retain file descriptors: registrations={after_registrations}, backends={after_backend_opens}"
    );

    for workspace in &workspaces {
        crate::storage::local::forget(workspace.path());
    }
    drop(registrations);
    let after_release = open_fd_count();
    eprintln!(
        "workspace_fd_lifetime before={before} after_registrations={after_registrations} after_backend_opens={after_backend_opens} after_release={after_release} workspaces={WORKSPACE_COUNT}"
    );
    assert!(
        after_release <= after_registrations + FD_TOLERANCE,
        "forgetting cached workspace backends should return to the callback-only descriptor count: registrations={after_registrations}, released={after_release}"
    );
}
