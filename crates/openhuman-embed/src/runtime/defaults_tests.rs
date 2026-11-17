use super::*;
#[test]
fn sampling_overlays_preserve_unspecified_fields() {
    let runtime = ModelDefaults {
        temperature: Some(0.2),
        top_p: Some(0.8),
        max_tokens: Some(1000),
        max_iterations: Some(4),
    };
    let spec = ModelDefaults {
        temperature: Some(0.7),
        max_tokens: Some(2000),
        ..Default::default()
    };
    let resolved = runtime.overlay(&spec);
    assert_eq!(resolved.temperature, Some(0.7));
    assert_eq!(resolved.max_tokens, Some(2000));
    assert_eq!(resolved.top_p, Some(0.8));
    assert_eq!(resolved.max_iterations, Some(4));
    assert_eq!(runtime.temperature, Some(0.2));
}
#[test]
fn invalid_sampling_parameters_are_rejected() {
    for model in [
        ModelDefaults {
            temperature: Some(f64::NAN),
            ..Default::default()
        },
        ModelDefaults {
            top_p: Some(1.1),
            ..Default::default()
        },
        ModelDefaults {
            max_tokens: Some(0),
            ..Default::default()
        },
        ModelDefaults {
            max_iterations: Some(0),
            ..Default::default()
        },
    ] {
        assert!(model.validate().is_err());
    }
}
