//! OpenHuman configuration boundary around portable install URL guards.

pub const MAX_INSTALL_URL_LEN: usize = tinyskills::MAX_INSTALL_URL_LEN;
pub(crate) const ALLOW_LOCAL_HTTP_ENV: &str = "OPENHUMAN_SKILL_INSTALL_ALLOW_LOCAL_HTTP";

pub(crate) fn normalize_install_url(raw: &str) -> Result<String, String> {
    tinyskills::normalize_registry_document_url(raw).map_err(|error| error.to_string())
}

pub fn validate_install_url(raw: &str) -> Result<(), String> {
    validate_install_url_with_config(raw, read_allow_local_http_env())
}

pub(crate) fn validate_install_url_with_config(
    raw: &str,
    allow_local_http: bool,
) -> Result<(), String> {
    tinyskills::validate_install_url(raw, allow_local_http).map_err(|error| error.to_string())
}

pub(crate) fn allow_local_http(raw: Option<String>) -> bool {
    raw.as_deref() == Some("1")
}

pub(super) fn read_allow_local_http_env() -> bool {
    allow_local_http(std::env::var(ALLOW_LOCAL_HTTP_ENV).ok())
}

pub async fn validate_resolved_host(raw_url: &str) -> Result<(), String> {
    tinyskills::validate_resolved_host(raw_url)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}
