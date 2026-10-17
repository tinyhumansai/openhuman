use super::*;
use crate::config::schema::ModuleOverride;

fn config(mode: ModuleSourceMode) -> Config {
    let mut config = Config::default();
    config.modules.source = mode;
    config
}

#[test]
fn release_is_the_default_and_serves_the_pinned_release() {
    let plan = plan_with(&Config::default(), "tinydocs", true, Some("/b".into()));
    assert_eq!(plan.requested, ModuleSourceMode::Release);
    assert_eq!(plan.served, ModuleSource::Release);
    assert_eq!(plan.artifact, None);
    assert_eq!(plan.fallback, None);
}

#[test]
fn linked_is_served_when_a_factory_is_registered() {
    let plan = plan_with(&config(ModuleSourceMode::Linked), "tinydocs", true, None);
    assert_eq!(plan.served, ModuleSource::Linked);
    assert_eq!(plan.fallback, None);
}

#[test]
fn linked_without_a_factory_falls_back_to_release_with_a_reason() {
    let plan = plan_with(&config(ModuleSourceMode::Linked), "tinydocs", false, None);
    assert_eq!(plan.requested, ModuleSourceMode::Linked);
    assert_eq!(plan.served, ModuleSource::Release);
    assert!(plan.fallback.is_some());
}

#[test]
fn source_uses_the_built_artifact_when_present() {
    let plan = plan_with(
        &config(ModuleSourceMode::Source),
        "tinydocs",
        false,
        Some("/build/libtinydocs.so".into()),
    );
    assert_eq!(plan.served, ModuleSource::Source);
    assert_eq!(plan.artifact, Some("/build/libtinydocs.so".into()));
}

#[test]
fn source_without_an_artifact_falls_back_to_release_with_a_reason() {
    let plan = plan_with(&config(ModuleSourceMode::Source), "tinydocs", false, None);
    assert_eq!(plan.served, ModuleSource::Release);
    assert!(plan.fallback.is_some());
}

#[test]
fn a_fallback_reason_never_carries_a_path() {
    let plan = plan_with(&config(ModuleSourceMode::Source), "tinydocs", false, None);
    assert!(!plan.fallback.unwrap().contains('/'));
}

#[test]
fn a_per_module_override_beats_the_global_mode() {
    let mut config = config(ModuleSourceMode::Linked);
    config.modules.overrides.push(ModuleOverride::with_source(
        "tinydocs",
        ModuleSourceMode::Release,
    ));
    let plan = plan_with(&config, "tinydocs", true, None);
    assert_eq!(plan.served, ModuleSource::Release);
    assert_eq!(plan.fallback, None);
    // Another module still follows the global mode.
    assert_eq!(
        plan_with(&config, "tinysearch", true, None).served,
        ModuleSource::Linked
    );
}

#[test]
fn a_path_override_still_loads_the_local_artifact() {
    let mut config = Config::default();
    config
        .modules
        .overrides
        .push(ModuleOverride::with_path("tinydocs", "/dev/libtinydocs.so"));
    let plan = plan_with(&config, "tinydocs", false, None);
    assert_eq!(plan.served, ModuleSource::LocalFile);
    assert_eq!(plan.artifact, Some("/dev/libtinydocs.so".into()));
}

#[test]
fn an_override_with_both_path_and_source_is_ignored() {
    let mut config = Config::default();
    config.modules.overrides.push(ModuleOverride {
        id: "tinydocs".into(),
        path: Some("/dev/libtinydocs.so".into()),
        source: Some(ModuleSourceMode::Linked),
    });
    let plan = plan_with(&config, "tinydocs", true, None);
    assert_eq!(plan.served, ModuleSource::Release);
    assert_eq!(plan.artifact, None);
}

#[test]
fn artifact_in_finds_the_built_library_by_registry_file_name() {
    let dir = tempfile::tempdir().unwrap();
    let record = crate::modules::registry::find("tinymcp").unwrap();
    assert_eq!(artifact_in(dir.path(), "tinymcp"), None);
    let module_dir = dir.path().join("tinymcp");
    std::fs::create_dir_all(&module_dir).unwrap();
    let file = module_dir.join(record.source.artifact_filename());
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(artifact_in(dir.path(), "tinymcp"), Some(file));
}

#[test]
fn artifact_in_has_nothing_for_an_unknown_id() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("nope")).unwrap();
    assert_eq!(artifact_in(dir.path(), "nope"), None);
}

#[test]
fn local_artifact_lookup_finds_the_published_module_filename() {
    let dir = tempfile::tempdir().unwrap();
    let released_name = if cfg!(target_os = "windows") {
        "tinymcp_module.dll"
    } else if cfg!(target_os = "macos") {
        "libtinymcp_module.dylib"
    } else {
        "libtinymcp_module.so"
    };
    let released = dir.path().join(released_name);
    std::fs::write(&released, b"module").unwrap();

    assert_eq!(module_artifact_in(dir.path(), "tinymcp"), Some(released));
}
