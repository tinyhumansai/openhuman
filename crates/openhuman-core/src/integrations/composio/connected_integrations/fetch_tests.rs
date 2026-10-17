use super::*;
use tinyconnectors_bus::vocabulary::ToolScope;
use tinyconnectors_bus::{
    ComposioConnectedIntegration, ComposioConnectedIntegrationTool,
    ComposioConnectedIntegrationsResponse, ComposioGatedIntegrationTool,
    ComposioIntegrationConnection,
};

fn overview() -> ComposioConnectedIntegrationsResponse {
    ComposioConnectedIntegrationsResponse {
        integrations: vec![
            ComposioConnectedIntegration {
                toolkit: "gmail".into(),
                description: "mail".into(),
                tools: vec![ComposioConnectedIntegrationTool {
                    name: "GMAIL_FETCH_EMAILS".into(),
                    description: "fetch".into(),
                    parameters: Some(serde_json::json!({"type": "object"})),
                }],
                gated_tools: vec![ComposioGatedIntegrationTool {
                    name: "GMAIL_DELETE_MESSAGE".into(),
                    description: "delete".into(),
                    required_scope: ToolScope::Admin,
                }],
                connected: true,
                connections: vec![ComposioIntegrationConnection {
                    connection_id: "c1".into(),
                    label: Some("a@b.c".into()),
                    is_default: true,
                }],
                non_active_status: None,
            },
            ComposioConnectedIntegration {
                toolkit: "notion".into(),
                description: "notes".into(),
                tools: vec![],
                gated_tools: vec![],
                connected: false,
                connections: vec![],
                non_active_status: Some("EXPIRED".into()),
            },
        ],
    }
}

#[test]
fn the_module_overview_maps_to_the_prompt_types() {
    let out = to_prompt_integrations(overview());
    assert_eq!(out.len(), 2);
    let gmail = &out[0];
    assert!(gmail.connected);
    assert_eq!(gmail.tools[0].name, "GMAIL_FETCH_EMAILS");
    assert_eq!(gmail.connections[0].connection_id, "c1");
    assert!(gmail.connections[0].is_default);
    let gated = &gmail.gated_tools[0];
    assert_eq!(gated.required_scope, "admin");
    assert_eq!(
        gated.unlock_paths,
        vec!["the user enables it themselves in Connections → gmail → admin".to_string()]
    );
    assert_eq!(out[1].non_active_status.as_deref(), Some("EXPIRED"));
    assert!(out[1].tools.is_empty());
}
