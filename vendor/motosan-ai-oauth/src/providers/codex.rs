use crate::{OAuthConfig, StateStrategy, TokenBodyFormat};

pub fn codex() -> OAuthConfig {
    OAuthConfig {
        client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
        client_secret: None,
        auth_url: "https://auth.openai.com/oauth/authorize",
        token_url: "https://auth.openai.com/oauth/token",
        scopes: &["openid", "profile", "email", "offline_access"],
        redirect_port: Some(1455),
        callback_path: "/auth/callback",
        redirect_uri_host: "127.0.0.1",
        token_body: TokenBodyFormat::Form,
        extra_auth_params: &[("access_type", "offline")],
        state_strategy: StateStrategy::Random,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_config_has_correct_client_id() {
        let c = codex();
        assert_eq!(c.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
    }

    #[test]
    fn codex_config_has_no_client_secret() {
        let c = codex();
        assert!(c.client_secret.is_none());
    }

    #[test]
    fn codex_config_redirect_port_is_1455() {
        let c = codex();
        assert_eq!(c.redirect_port, Some(1455));
    }

    #[test]
    fn codex_config_auth_url_is_openai() {
        let c = codex();
        assert!(c.auth_url.contains("auth.openai.com"));
    }
}
