use super::*;

#[test]
fn view_image_schema_requires_path_and_marks_model_visible_output() {
    let spec = image_view_spec();

    assert_eq!(spec.name, "view_image");
    assert!(spec.description.contains("model-visible image context"));
    assert_eq!(spec.permission, ImagePermission::ReadOnly);
    assert!(spec.model_visible_image_output);
    assert!(!spec.writes_files);
    assert_eq!(spec.parameters["required"], serde_json::json!(["path"]));
    assert_eq!(spec.parameters["properties"]["detail"]["default"], "auto");
}

#[test]
fn detail_names_match_prompt_contract() {
    assert_eq!(ImageDetail::Auto.as_str(), "auto");
    assert_eq!(ImageDetail::High.as_str(), "high");
    assert_eq!(ImageDetail::Original.as_str(), "original");

    assert_eq!(
        serde_json::to_value(ImageDetail::Original).unwrap(),
        serde_json::json!("original")
    );
    assert_eq!(
        serde_json::from_value::<ImageDetail>(serde_json::json!("high")).unwrap(),
        ImageDetail::High
    );
}

#[test]
fn view_image_schema_lists_supported_detail_levels() {
    let spec = image_view_spec();

    assert_eq!(
        spec.parameters["properties"]["detail"]["enum"],
        serde_json::json!(["auto", "high", "original"])
    );
    assert_eq!(spec.parameters["properties"]["path"]["type"], "string");
}
