use serde_json::Value;

const DEFAULT_SOURCE_TYPE: &str = "mcp";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct McpSession {
    client_source_type: Option<String>,
}

impl McpSession {
    pub(crate) fn observe_initialize_params(&mut self, params: &Value) {
        if let Some(client_source_type) = self.client_source_type.as_deref() {
            log::trace!(
                "[mcp_server] initialize provenance already captured client_source_type={} param_keys={:?}",
                client_source_type,
                object_keys(params)
            );
            return;
        }

        let Some(normalized_name) = params
            .as_object()
            .and_then(|obj| obj.get("clientInfo"))
            .and_then(Value::as_object)
            .and_then(|client_info| client_info.get("name"))
            .and_then(Value::as_str)
            .and_then(Self::normalize_client_name)
        else {
            log::trace!(
                "[mcp_server] initialize provenance fallback locked client_source_type={} param_keys={:?}",
                DEFAULT_SOURCE_TYPE,
                object_keys(params)
            );
            self.client_source_type = Some(DEFAULT_SOURCE_TYPE.to_string());
            return;
        };

        let client_source_type = format!("{DEFAULT_SOURCE_TYPE}:{normalized_name}");
        log::debug!(
            "[mcp_server] initialize provenance captured base_source_type={} normalized_client_name={} client_source_type={}",
            DEFAULT_SOURCE_TYPE,
            normalized_name,
            client_source_type
        );
        self.client_source_type = Some(client_source_type);
    }

    pub(crate) fn source_type(&self) -> &str {
        self.client_source_type
            .as_deref()
            .unwrap_or(DEFAULT_SOURCE_TYPE)
    }

    pub(crate) fn normalize_client_name(raw: &str) -> Option<String> {
        let mut normalized = String::new();
        let mut previous_was_separator = false;

        for ch in raw.trim().chars() {
            if ch.is_ascii_alphanumeric() {
                normalized.push(ch.to_ascii_lowercase());
                previous_was_separator = false;
            } else if !normalized.is_empty() && !previous_was_separator {
                normalized.push('-');
                previous_was_separator = true;
            }
        }

        while normalized.ends_with('-') {
            normalized.pop();
        }

        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }
}

fn object_keys(value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}
