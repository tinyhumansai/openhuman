use super::*;
use crate::openhuman::media::image::ImageGenerationOutputFormat;

#[test]
fn prompt_guidance_is_empty_when_no_image_tools_are_enabled() {
    let rendered =
        render_image_prompt_guidance(&ImageToolConfig::default(), &ImagePromptOptions::default());

    assert!(rendered.is_empty());
}

#[test]
fn prompt_guidance_renders_image_generation_and_view_rules() {
    let config = ImageToolConfig {
        image_generation_enabled: true,
        image_view_enabled: true,
        image_generation_output_format: ImageGenerationOutputFormat::Png,
        local_image_reads_allowed: true,
        generated_image_writes_allowed: true,
    };

    let rendered = render_image_prompt_guidance(&config, &ImagePromptOptions::default());

    assert!(rendered.contains("## Image Tools"));
    assert!(rendered.contains("`view_image`"));
    assert!(rendered.contains("`image_generation`"));
    assert!(rendered.contains("saved artifact path"));
}

#[test]
fn prompt_guidance_can_omit_optional_rule_text() {
    let config = ImageToolConfig {
        image_generation_enabled: true,
        image_view_enabled: true,
        image_generation_output_format: ImageGenerationOutputFormat::Png,
        local_image_reads_allowed: true,
        generated_image_writes_allowed: true,
    };
    let options = ImagePromptOptions {
        include_final_answer_rules: false,
        include_local_file_boundaries: false,
    };

    let rendered = render_image_prompt_guidance(&config, &options);

    assert!(rendered.contains("`view_image`"));
    assert!(rendered.contains("`image_generation`"));
    assert!(!rendered.contains("approved workspace"));
    assert!(!rendered.contains("saved artifact path"));
}

#[test]
fn prompt_guidance_respects_policy_gates() {
    let config = ImageToolConfig {
        image_generation_enabled: true,
        image_view_enabled: true,
        image_generation_output_format: ImageGenerationOutputFormat::Png,
        local_image_reads_allowed: false,
        generated_image_writes_allowed: false,
    };

    let rendered = render_image_prompt_guidance(&config, &ImagePromptOptions::default());

    assert!(rendered.is_empty());
}

#[test]
fn prompt_guidance_renders_single_available_tool() {
    let generation_only = ImageToolConfig {
        image_generation_enabled: true,
        image_view_enabled: false,
        image_generation_output_format: ImageGenerationOutputFormat::Png,
        local_image_reads_allowed: true,
        generated_image_writes_allowed: true,
    };
    let view_only = ImageToolConfig {
        image_generation_enabled: false,
        image_view_enabled: true,
        image_generation_output_format: ImageGenerationOutputFormat::Png,
        local_image_reads_allowed: true,
        generated_image_writes_allowed: true,
    };

    let generation_rendered =
        render_image_prompt_guidance(&generation_only, &ImagePromptOptions::default());
    let view_rendered = render_image_prompt_guidance(&view_only, &ImagePromptOptions::default());

    assert!(generation_rendered.contains("`image_generation`"));
    assert!(!generation_rendered.contains("`view_image`"));
    assert!(view_rendered.contains("`view_image`"));
    assert!(!view_rendered.contains("`image_generation`"));
}
