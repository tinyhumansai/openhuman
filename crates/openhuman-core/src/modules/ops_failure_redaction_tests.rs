use super::*;

#[tokio::test]
async fn a_failing_linked_factory_is_reported_without_its_error_detail() {
    fn failing() -> tinybus::Result<tinybus::module::LinkedModule> {
        Err(tinybus::Error::failed("secret-token /home/user/path"))
    }
    let runtime = crate::modules::runtime::runtime()
        .await
        .expect("module host");
    let error = Resolver::load_linked(
        runtime,
        &registry::find("tinychannels").unwrap().release(),
        Some(failing),
        serde_json::json!({}),
    )
    .expect_err("the factory fails");
    assert!(
        error.contains("linked factory could not be started"),
        "{error}"
    );
    assert!(!error.contains("secret-token"), "{error}");
    assert!(!error.contains("/home"), "{error}");
}

#[test]
fn a_debug_build_warns_when_a_local_build_is_served_ahead_of_its_pin() {
    use crate::modules::source::pin_ahead_warning;
    use crate::modules::types::ModuleSource;

    for served in [ModuleSource::Linked, ModuleSource::Source] {
        let line = pin_ahead_warning("tinydocs", served, Some("tinysearch, tinydocs"), true)
            .expect("an ahead id served locally warns");
        assert!(line.contains("id=tinydocs"));
        assert!(line.contains("gitlink ahead of released pin"));
        assert!(line.contains("release-and-repin"));
    }
}

#[test]
fn the_ahead_warning_is_silent_for_released_modules_other_ids_and_release_builds() {
    use crate::modules::source::pin_ahead_warning;
    use crate::modules::types::ModuleSource;

    // Released and explicit-file modules are not a locally compiled pin.
    for served in [ModuleSource::Release, ModuleSource::LocalFile] {
        assert!(pin_ahead_warning("tinydocs", served, Some("tinydocs"), true).is_none());
    }
    // Not in the list, an id that merely contains the name, no list at all.
    assert!(pin_ahead_warning("tinydocs", ModuleSource::Source, Some("tinydocs2"), true).is_none());
    assert!(pin_ahead_warning("tinydocs", ModuleSource::Source, Some(""), true).is_none());
    assert!(pin_ahead_warning("tinydocs", ModuleSource::Source, None, true).is_none());
    // A release build never warns.
    assert!(pin_ahead_warning("tinydocs", ModuleSource::Source, Some("tinydocs"), false).is_none());
}
