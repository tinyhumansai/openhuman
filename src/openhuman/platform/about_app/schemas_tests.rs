use super::*;

#[test]
fn schema_names_are_stable() {
    let list = about_app_schemas("about_app_list");
    assert_eq!(list.namespace, "about_app");
    assert_eq!(list.function, "list");

    let lookup = about_app_schemas("about_app_lookup");
    assert_eq!(lookup.namespace, "about_app");
    assert_eq!(lookup.function, "lookup");

    let search = about_app_schemas("about_app_search");
    assert_eq!(search.namespace, "about_app");
    assert_eq!(search.function, "search");
}

#[test]
fn controller_lists_match_lengths() {
    assert_eq!(
        all_about_app_controller_schemas().len(),
        all_about_app_registered_controllers().len()
    );
}
