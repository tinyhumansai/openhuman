use super::*;

#[test]
fn image_generation_schema_declares_output_path_and_format() {
    let spec = image_generation_spec(ImageGenerationOutputFormat::Webp);

    assert_eq!(spec.name, "image_generation");
    assert!(spec.description.contains("Generate or edit raster images"));
    assert_eq!(spec.permission, ImagePermission::Write);
    assert!(!spec.model_visible_image_output);
    assert!(spec.writes_files);
    assert_eq!(spec.parameters["required"], serde_json::json!(["prompt"]));
    assert!(spec.parameters["properties"].get("output_path").is_some());
    assert_eq!(
        spec.parameters["properties"]["output_format"]["default"],
        "webp"
    );
}

#[test]
fn output_format_serializes_as_snake_case() {
    assert_eq!(ImageGenerationOutputFormat::Png.as_str(), "png");
    assert_eq!(ImageGenerationOutputFormat::Webp.as_str(), "webp");
    assert_eq!(ImageGenerationOutputFormat::Jpeg.as_str(), "jpeg");

    assert_eq!(
        serde_json::to_value(ImageGenerationOutputFormat::Webp).unwrap(),
        serde_json::json!("webp")
    );
    assert_eq!(
        serde_json::from_value::<ImageGenerationOutputFormat>(serde_json::json!("jpeg")).unwrap(),
        ImageGenerationOutputFormat::Jpeg
    );
}

#[test]
fn image_generation_schema_keeps_edit_and_size_inputs_optional() {
    let spec = image_generation_spec(ImageGenerationOutputFormat::Png);
    let properties = spec.parameters["properties"].as_object().unwrap();

    assert_eq!(
        properties.keys().cloned().collect::<Vec<_>>(),
        vec![
            "input_image_path".to_string(),
            "output_format".to_string(),
            "output_path".to_string(),
            "prompt".to_string(),
            "size".to_string()
        ]
    );
    assert_eq!(
        properties["output_format"]["enum"],
        serde_json::json!(["png", "webp", "jpeg"])
    );
}
