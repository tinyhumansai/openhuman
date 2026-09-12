use super::*;

#[test]
fn image_specs_gate_each_tool_independently() {
    let config = ImageToolConfig {
        image_generation_enabled: true,
        image_view_enabled: true,
        local_image_reads_allowed: false,
        generated_image_writes_allowed: true,
        ..ImageToolConfig::default()
    };

    let specs = image_specs(&config);

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, IMAGE_GENERATION_TOOL_NAME);
    assert!(!is_image_tool_gated(IMAGE_GENERATION_TOOL_NAME, &config));
    assert!(is_image_tool_gated(IMAGE_VIEW_TOOL_NAME, &config));
}

#[test]
fn image_specs_hide_generation_when_writes_are_blocked() {
    let config = ImageToolConfig {
        image_generation_enabled: true,
        image_view_enabled: true,
        local_image_reads_allowed: true,
        generated_image_writes_allowed: false,
        ..ImageToolConfig::default()
    };

    let specs = image_specs(&config);

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, IMAGE_VIEW_TOOL_NAME);
    assert!(is_image_tool_gated(IMAGE_GENERATION_TOOL_NAME, &config));
    assert!(!is_image_tool_gated(IMAGE_VIEW_TOOL_NAME, &config));
}

#[test]
fn image_specs_are_empty_when_runtime_support_is_disabled() {
    let config = ImageToolConfig {
        local_image_reads_allowed: true,
        generated_image_writes_allowed: true,
        ..ImageToolConfig::default()
    };

    assert!(image_specs(&config).is_empty());
    assert!(is_image_tool_gated("unknown_tool", &config));
}

#[test]
fn image_e2e_contract_renders_specs_and_prompt_guidance() {
    let config = ImageToolConfig {
        image_generation_enabled: true,
        image_view_enabled: true,
        image_generation_output_format: ImageGenerationOutputFormat::Jpeg,
        local_image_reads_allowed: true,
        generated_image_writes_allowed: true,
    };

    let specs = image_specs(&config);
    let names = specs
        .iter()
        .map(|spec| spec.name.as_str())
        .collect::<Vec<_>>();
    let prompt = render_image_prompt_guidance(&config, &ImagePromptOptions::default());

    assert_eq!(
        names,
        vec![IMAGE_GENERATION_TOOL_NAME, IMAGE_VIEW_TOOL_NAME]
    );
    assert_eq!(
        specs[0].parameters["properties"]["output_format"]["default"],
        "jpeg"
    );
    assert_eq!(
        specs[1].parameters["properties"]["detail"]["default"],
        "auto"
    );
    assert!(prompt.contains("## Image Tools"));
    assert!(prompt.contains(IMAGE_GENERATION_TOOL_NAME));
    assert!(prompt.contains(IMAGE_VIEW_TOOL_NAME));
}

#[test]
fn known_image_tool_names_stay_stable() {
    assert_eq!(
        IMAGE_TOOL_NAMES,
        [IMAGE_GENERATION_TOOL_NAME, IMAGE_VIEW_TOOL_NAME]
    );
}

#[test]
fn image_spec_serializes_for_schema_catalogs() {
    let spec = image_generation_spec(ImageGenerationOutputFormat::Png);
    let encoded = serde_json::to_value(&spec).unwrap();

    assert_eq!(encoded["name"], IMAGE_GENERATION_TOOL_NAME);
    assert_eq!(encoded["permission"], "write");
    assert_eq!(encoded["writes_files"], true);
    assert_eq!(encoded["model_visible_image_output"], false);

    let decoded: ImageToolSpec = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, spec);
}

#[test]
fn image_config_default_is_closed_by_capability() {
    let config = ImageToolConfig::default();

    assert!(!config.image_generation_enabled);
    assert!(!config.image_view_enabled);
    assert_eq!(
        config.image_generation_output_format,
        ImageGenerationOutputFormat::Png
    );
    assert!(config.local_image_reads_allowed);
    assert!(config.generated_image_writes_allowed);
    assert!(image_specs(&config).is_empty());
}
