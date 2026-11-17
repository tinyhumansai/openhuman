use super::{ConfigProvenance, DotenvLoadProvenance, ProvenanceSource};
use std::path::PathBuf;

#[test]
fn environment_override_reports_source_and_whether_the_file_also_set_it() {
    let document: toml::Value = toml::from_str(r#"default_model = "file-model""#).unwrap();
    let mut provenance = ConfigProvenance::default();
    provenance.record_file_keys(&document);
    provenance.record_env_override(
        "default_model",
        "OPENHUMAN_DEFAULT_MODEL",
        ProvenanceSource::Env,
    );

    let entry = provenance.field("default_model");
    assert_eq!(entry.source, ProvenanceSource::Env);
    assert_eq!(entry.env_var.as_deref(), Some("OPENHUMAN_DEFAULT_MODEL"));
    assert!(entry.file_also_set);
    assert!(!format!("{provenance:?}").contains("file-model"));
}

#[test]
fn unoverridden_values_report_file_or_default_without_values() {
    let document: toml::Value = toml::from_str(
        r#"
default_model = "file-model"
[agent]
tool_dispatcher = "native"
"#,
    )
    .unwrap();
    let provenance = ConfigProvenance::from_file_document(&document);

    assert_eq!(
        provenance.field("default_model").source,
        ProvenanceSource::File
    );
    assert_eq!(
        provenance.field("agent.tool_dispatcher").source,
        ProvenanceSource::File
    );
    assert_eq!(
        provenance.field("output_language").source,
        ProvenanceSource::Default
    );
    assert!(!format!("{provenance:?}").contains("file-model"));
    assert!(!format!("{provenance:?}").contains("native"));
}

#[test]
fn explicit_subtree_lookup_matches_exact_and_descendant_paths_only() {
    let document: toml::Value =
        toml::from_str("[memory]\nengine = \"tinyhumans\"\n[search]\nenabled = true\n").unwrap();
    let provenance = ConfigProvenance::from_file_document(&document);

    assert!(provenance.has_explicit_under("memory"));
    assert!(provenance.has_explicit_under("memory.engine"));
    assert!(provenance.has_explicit_under("search.enabled"));
    assert!(!provenance.has_explicit_under("mem"));
    assert!(!provenance.has_explicit_under("memory_config"));
}

#[test]
fn every_serialized_config_field_has_file_or_default_source_when_no_env_is_applied() {
    let config = super::config_from_toml_str(
        r#"
default_model = "file-model"
[agent]
tool_dispatcher = "native"
"#,
    )
    .unwrap();
    let serialized = serde_json::to_value(&config).unwrap();
    assert_config_sources(&serialized, "", config.load_provenance());
}

fn assert_config_sources(value: &serde_json::Value, path: &str, provenance: &ConfigProvenance) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                assert_config_sources(child, &child_path, provenance);
            }
        }
        serde_json::Value::Array(items) if items.iter().any(serde_json::Value::is_object) => {
            for item in items {
                assert_config_sources(item, &format!("{path}[]"), provenance);
            }
        }
        _ => {
            let source = provenance.field(path).source;
            assert!(
                matches!(source, ProvenanceSource::File | ProvenanceSource::Default),
                "{path} reported unexpected source {source:?}"
            );
        }
    }
}

#[test]
fn dotenv_record_keeps_the_loaded_path_and_only_names_of_shadowed_variables() {
    let record = DotenvLoadProvenance::from_variables(
        PathBuf::from("/workspace/.env"),
        ["DOTENV_ONLY", "PROCESS_WINS"],
        ["PROCESS_WINS"],
    );

    assert_eq!(record.path, PathBuf::from("/workspace/.env"));
    assert_eq!(record.skipped_vars, ["PROCESS_WINS"]);
    assert_eq!(record.loaded_vars, ["DOTENV_ONLY"]);
    assert!(!format!("{record:?}").contains("secret"));
}
