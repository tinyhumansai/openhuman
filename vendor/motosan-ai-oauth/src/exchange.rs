use serde::Deserialize;

use crate::{error::Error, unix_now, OAuthConfig, StateStrategy, Token};

#[derive(Deserialize)]
struct RawTokenResponse {
    access_token: String,
    /// Servers may omit `refresh_token` on a refresh grant (RFC 6749 §6).
    /// Callers must pass their existing refresh token as fallback.
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    expires_in: u64,
}

impl RawTokenResponse {
    fn into_token(self, fallback_refresh: Option<&str>) -> Token {
        Token {
            access_token: self.access_token,
            refresh_token: self
                .refresh_token
                .or_else(|| fallback_refresh.map(str::to_owned))
                .unwrap_or_default(),
            id_token: self.id_token,
            expires_in: self.expires_in,
            issued_at: unix_now(),
        }
    }
}

async fn post_token(
    token_url: &str,
    body_format: crate::TokenBodyFormat,
    params: Vec<(&str, &str)>,
    fallback_refresh: Option<&str>,
) -> Result<Token, Error> {
    let client = reqwest::Client::new();
    // No custom User-Agent override — reqwest's default is safer than a
    // "Mozilla/5.0..." string (which can look like a bot to anti-abuse
    // systems). Accept: application/json matches pi-ai's reference impl.
    let req = client.post(token_url).header("Accept", "application/json");

    let req = match body_format {
        crate::TokenBodyFormat::Form => req.form(&params),
        crate::TokenBodyFormat::Json => {
            let map: std::collections::HashMap<&str, &str> = params.iter().copied().collect();
            req.json(&map)
        }
    };

    let resp = req.send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::TokenExchange(format!("HTTP {status}: {body}")));
    }

    Ok(resp
        .json::<RawTokenResponse>()
        .await?
        .into_token(fallback_refresh))
}

pub async fn exchange_code(
    config: &OAuthConfig,
    code: &str,
    state: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Token, Error> {
    // Standard OAuth (RFC 6749 §4.1.3) does not require `state` on the
    // token endpoint, and some servers reject extra fields. Anthropic's
    // endpoint empirically validates it, so only echo `state` for the
    // Anthropic-style strategy where state equals the PKCE verifier.
    let mut params = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", verifier),
        ("client_id", config.client_id),
    ];
    if matches!(config.state_strategy, StateStrategy::EqualsVerifier) {
        params.push(("state", state));
    }
    if let Some(secret) = config.client_secret {
        params.push(("client_secret", secret));
    }
    post_token(config.token_url, config.token_body, params, None).await
}

pub async fn refresh_token(config: &OAuthConfig, refresh_token: &str) -> Result<Token, Error> {
    let mut params = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", config.client_id),
    ];
    if let Some(secret) = config.client_secret {
        params.push(("client_secret", secret));
    }
    post_token(
        config.token_url,
        config.token_body,
        params,
        Some(refresh_token),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TokenBodyFormat;
    use mockito::Matcher;

    #[tokio::test]
    async fn exchange_code_omits_state_for_random_strategy_form_body() {
        let mut server = mockito::Server::new_async().await;
        let token_url: &'static str = Box::leak(format!("{}/token", server.url()).into_boxed_str());

        let mock = server
            .mock("POST", "/token")
            .match_header(
                "content-type",
                Matcher::Regex("application/x-www-form-urlencoded".into()),
            )
            .match_body(Matcher::Exact(
                "grant_type=authorization_code&code=AUTHCODE&redirect_uri=http%3A%2F%2F127.0.0.1%2Fcb&code_verifier=VERIFIER&client_id=test-client"
                    .into(),
            ))
            .with_status(200)
            .with_body(r#"{"access_token":"AT","refresh_token":"RT","expires_in":3600}"#)
            .create_async()
            .await;

        let cfg = crate::OAuthConfig {
            client_id: "test-client",
            client_secret: None,
            auth_url: "https://unused/",
            token_url,
            scopes: &[],
            redirect_port: None,
            callback_path: "/auth/callback",
            redirect_uri_host: "127.0.0.1",
            token_body: TokenBodyFormat::Form,
            extra_auth_params: &[],
            state_strategy: crate::StateStrategy::Random,
        };
        let token = exchange_code(&cfg, "AUTHCODE", "STATE", "VERIFIER", "http://127.0.0.1/cb")
            .await
            .expect("ok");
        assert_eq!(token.access_token, "AT");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn exchange_code_omits_state_for_random_strategy_json_body() {
        let mut server = mockito::Server::new_async().await;
        let token_url: &'static str = Box::leak(format!("{}/token", server.url()).into_boxed_str());

        let mock = server
            .mock("POST", "/token")
            .match_header("content-type", Matcher::Regex("application/json".into()))
            // Matcher::JsonString compares parsed JSON, so key order in the
            // serialized HashMap does not matter.
            .match_body(Matcher::JsonString(
                r#"{"grant_type":"authorization_code","code":"AUTHCODE","redirect_uri":"http://127.0.0.1/cb","code_verifier":"VERIFIER","client_id":"test-client"}"#
                .into(),
            ))
            .with_status(200)
            .with_body(r#"{"access_token":"AT","refresh_token":"RT","expires_in":3600}"#)
            .create_async()
            .await;

        let cfg = crate::OAuthConfig {
            client_id: "test-client",
            client_secret: None,
            auth_url: "https://unused/",
            token_url,
            scopes: &[],
            redirect_port: None,
            callback_path: "/auth/callback",
            redirect_uri_host: "127.0.0.1",
            token_body: TokenBodyFormat::Json,
            extra_auth_params: &[],
            state_strategy: crate::StateStrategy::Random,
        };
        let token = exchange_code(&cfg, "AUTHCODE", "STATE", "VERIFIER", "http://127.0.0.1/cb")
            .await
            .expect("ok");
        assert_eq!(token.access_token, "AT");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn exchange_code_sends_state_for_equals_verifier_strategy_json_body() {
        let mut server = mockito::Server::new_async().await;
        let token_url: &'static str = Box::leak(format!("{}/token", server.url()).into_boxed_str());

        let mock = server
            .mock("POST", "/token")
            .match_header("content-type", Matcher::Regex("application/json".into()))
            .match_body(Matcher::JsonString(
                r#"{"grant_type":"authorization_code","code":"AUTHCODE","state":"STATE","redirect_uri":"http://127.0.0.1/cb","code_verifier":"VERIFIER","client_id":"test-client"}"#
                .into(),
            ))
            .with_status(200)
            .with_body(r#"{"access_token":"AT","refresh_token":"RT","expires_in":3600}"#)
            .create_async()
            .await;

        let cfg = crate::OAuthConfig {
            client_id: "test-client",
            client_secret: None,
            auth_url: "https://unused/",
            token_url,
            scopes: &[],
            redirect_port: None,
            callback_path: "/auth/callback",
            redirect_uri_host: "127.0.0.1",
            token_body: TokenBodyFormat::Json,
            extra_auth_params: &[],
            state_strategy: crate::StateStrategy::EqualsVerifier,
        };
        let token = exchange_code(&cfg, "AUTHCODE", "STATE", "VERIFIER", "http://127.0.0.1/cb")
            .await
            .expect("ok");
        assert_eq!(token.access_token, "AT");
        mock.assert_async().await;
    }
}
