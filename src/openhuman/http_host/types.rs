use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct HostedDirAuth {
    pub enabled: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostedDirServerInfo {
    pub server_id: String,
    pub server_name: Option<String>,
    pub directory: String,
    pub bind_host: String,
    pub port: u16,
    pub base_url: String,
    pub local_url: String,
    pub auth: HostedDirAuth,
}

#[derive(Debug, Deserialize)]
pub struct StartHostedDirParams {
    pub directory: String,
    pub port: u16,
    #[serde(default = "default_bind_host")]
    pub bind_host: String,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub disable_auth: bool,
    #[serde(default)]
    pub username: Option<String>,
}

fn default_bind_host() -> String {
    "127.0.0.1".to_string()
}

#[derive(Debug, Deserialize)]
pub struct HostedDirLookupParams {
    pub server_id: String,
}

#[derive(Debug, Serialize)]
pub struct HostedDirListResult {
    pub servers: Vec<HostedDirServerInfo>,
}

#[derive(Debug, Serialize)]
pub struct HostedDirGetResult {
    pub server: HostedDirServerInfo,
}

#[derive(Debug, Serialize)]
pub struct HostedDirStartResult {
    pub server: HostedDirServerInfo,
}

#[derive(Debug, Serialize)]
pub struct HostedDirStopResult {
    pub stopped: bool,
    pub server: HostedDirServerInfo,
}
