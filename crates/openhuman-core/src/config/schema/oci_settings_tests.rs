use super::*;
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

#[test]
fn defaults_are_api_key_over_openai() {
    let settings = OciProviderSettings::default();
    assert_eq!(settings.auth_mode, OciAuthMode::ApiKey);
    assert_eq!(settings.transport, OciTransportKind::Openai);
    assert!(settings.auth_mode.needs_stored_key());
    assert!(!settings.transport.is_native());
}

#[test]
fn wire_names_round_trip_for_every_variant() {
    for mode in [
        OciAuthMode::ApiKey,
        OciAuthMode::ConfigProfile,
        OciAuthMode::SessionToken,
        OciAuthMode::ResourcePrincipal,
        OciAuthMode::InstancePrincipal,
        OciAuthMode::OkeWorkloadIdentity,
    ] {
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, format!("\"{}\"", mode.as_str()));
        assert_eq!(serde_json::from_str::<OciAuthMode>(&json).unwrap(), mode);
    }
    for transport in [
        OciTransportKind::Openai,
        OciTransportKind::Actions,
        OciTransportKind::NativeGeneric,
        OciTransportKind::NativeCohere,
    ] {
        let json = serde_json::to_string(&transport).unwrap();
        assert_eq!(json, format!("\"{}\"", transport.as_str()));
        assert_eq!(
            serde_json::from_str::<OciTransportKind>(&json).unwrap(),
            transport
        );
    }
}

#[test]
fn only_profile_modes_read_the_config_file() {
    assert!(OciAuthMode::ConfigProfile.uses_config_file());
    assert!(OciAuthMode::SessionToken.uses_config_file());
    assert!(!OciAuthMode::ApiKey.uses_config_file());
    assert!(!OciAuthMode::ResourcePrincipal.uses_config_file());
}

#[test]
fn provider_entry_round_trips_with_oci_settings() {
    let entry = CloudProviderCreds {
        id: "p_oci_1".into(),
        slug: "oci".into(),
        label: "OCI Generative AI".into(),
        endpoint: String::new(),
        auth_style: AuthStyle::Oci,
        oci: Some(OciProviderSettings {
            auth_mode: OciAuthMode::SessionToken,
            profile: Some("SESSION".into()),
            region: Some("us-chicago-1".into()),
            compartment_id: Some("ocid1.compartment.oc1..example".into()),
            transport: OciTransportKind::NativeGeneric,
            ..Default::default()
        }),
        ..Default::default()
    };
    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["auth_style"], "oci");
    assert_eq!(json["oci"]["auth_mode"], "session_token");
    assert_eq!(json["oci"]["transport"], "native_generic");
    assert!(json["oci"].get("project_id").is_none());
    let back: CloudProviderCreds = serde_json::from_value(json).unwrap();
    assert_eq!(back, entry);
}

#[test]
fn legacy_entries_without_oci_deserialize_to_none() {
    let entry: CloudProviderCreds =
        serde_json::from_str(r#"{"id":"a","slug":"openai","endpoint":"https://x/v1"}"#).unwrap();
    assert!(entry.oci.is_none());
    assert!(serde_json::to_value(&entry).unwrap().get("oci").is_none());
}
