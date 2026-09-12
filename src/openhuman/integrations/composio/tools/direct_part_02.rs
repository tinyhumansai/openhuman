
impl ComposioTool {
    pub fn new(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
    ) -> Self {
        // Production always pins the real HTTPS endpoints.
        Self::new_internal(
            api_key,
            default_entity_id,
            security,
            COMPOSIO_API_BASE_V2.to_string(),
            COMPOSIO_API_BASE_V3.to_string(),
            false,
        )
    }

    pub(crate) fn auth_key_fingerprint(&self) -> u64 {
        crate::openhuman::integrations::composio::direct_auth::fingerprint_api_key(&self.api_key)
    }

    /// Debug-test seam for raw integration coverage: construct a direct
    /// Composio tool against explicit v2/v3 base URLs. Non-HTTPS URLs are
    /// accepted only for loopback hosts and only in debug builds.
    #[cfg(debug_assertions)]
    pub fn new_with_base_urls_for_loopback(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
        base_v2: String,
        base_v3: String,
    ) -> anyhow::Result<Self> {
        for base in [&base_v2, &base_v3] {
            if !base.starts_with("https://") && !is_loopback_http_base(base) {
                anyhow::bail!("debug Composio base URL must be HTTPS or loopback HTTP");
            }
        }
        Ok(Self::new_internal(
            api_key,
            default_entity_id,
            security,
            base_v2,
            base_v3,
            true,
        ))
    }

    /// Test-only seam: construct with an explicit Composio v3 base URL so
    /// unit tests can point the direct `/tools` request — including the
    /// `tags` filter — at a local mock instead of `backend.composio.dev`.
    ///
    /// `#[cfg(test)]`-gated on purpose: `list_tool_schemas_v3` attaches the
    /// `x-api-key` header to whatever `base_v3` holds, so the only way to
    /// reach the v3 endpoint in production is [`Self::new`], which always
    /// uses the HTTPS [`COMPOSIO_API_BASE_V3`] const. An injectable base must
    /// never carry a non-HTTPS URL outside tests.
    #[cfg(test)]
    pub(crate) fn new_with_v3_base(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
        base_v3: String,
    ) -> Self {
        Self::new_internal(
            api_key,
            default_entity_id,
            security,
            COMPOSIO_API_BASE_V2.to_string(),
            base_v3,
            true,
        )
    }

    /// Shared constructor body. Private so the injectable `base_v3` cannot be
    /// supplied by production callers — they go through [`Self::new`] (real
    /// HTTPS const) and tests through the `#[cfg(test)]` `new_with_v3_base`.
    fn new_internal(
        api_key: &str,
        default_entity_id: Option<&str>,
        security: Arc<SecurityPolicy>,
        base_v2: String,
        base_v3: String,
        allow_insecure_loopback: bool,
    ) -> Self {
        let trimmed = api_key.trim();
        if trimmed.len() != api_key.len() {
            // The key carried leading/trailing whitespace that would otherwise
            // reach Composio's `x-api-key` header verbatim and trip the
            // server-side "Invalid API key format" 401 (Sentry TAURI-RUST-D3).
            // We trim here so the request succeeds; logging the length delta
            // (never the key itself) helps trace which credential source
            // produced a dirty value without leaking the secret.
            tracing::debug!(
                original_len = api_key.len(),
                trimmed_len = trimmed.len(),
                "[composio] trimmed leading/trailing whitespace from api_key"
            );
        }
        Self {
            api_key: trimmed.to_string(),
            default_entity_id: normalize_entity_id(default_entity_id.unwrap_or("default")),
            security,
            base_v2,
            base_v3,
            allow_insecure_loopback,
        }
    }

    fn client(&self) -> Client {
        crate::openhuman::config::build_runtime_proxy_client_with_timeouts("tool.composio", 60, 10)
    }

    fn ensure_request_url(&self, url: &str) -> anyhow::Result<()> {
        if self.allow_insecure_loopback && is_loopback_http_url(url) {
            return Ok(());
        }
        ensure_https(url)
    }

    /// List available Composio apps/actions for the authenticated user.
    ///
    /// Uses v3 endpoint first and falls back to v2 for compatibility.
    pub async fn list_actions(
        &self,
        app_name: Option<&str>,
    ) -> anyhow::Result<Vec<ComposioAction>> {
        match self.list_actions_v3(app_name).await {
            Ok(items) => Ok(items),
            Err(v3_err) => {
                let v2 = self.list_actions_v2(app_name).await;
                match v2 {
                    Ok(items) => Ok(items),
                    Err(v2_err) => anyhow::bail!(
                        "Composio action listing failed on v3 ({v3_err}) and v2 fallback ({v2_err})"
                    ),
                }
            }
        }
    }

    async fn list_actions_v3(&self, app_name: Option<&str>) -> anyhow::Result<Vec<ComposioAction>> {
        let url = format!("{}/tools", self.base_v3);
        let mut req = self.client().get(&url).header("x-api-key", &self.api_key);

        // #3932: pin toolkit_versions=latest. Composio v3 otherwise defaults to
        // the 00000000_00 snapshot, which lists zero tools for any toolkit
        // published after it (Outlook and every other post-launch toolkit).
        req = req.query(&[("limit", "200"), ("toolkit_versions", "latest")]);
        if let Some(app) = app_name.map(str::trim).filter(|app| !app.is_empty()) {
            req = req.query(&[("toolkits", app), ("toolkit_slug", app)]);
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 API error: {err}");
        }

        let body: ComposioToolsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 tools response")?;
        Ok(map_v3_tools_to_actions(body.items))
    }

    async fn list_actions_v2(&self, app_name: Option<&str>) -> anyhow::Result<Vec<ComposioAction>> {
        let mut url = format!("{}/actions", self.base_v2);
        if let Some(app) = app_name {
            url = format!("{url}?appNames={app}");
        }

        let resp = self
            .client()
            .get(&url)
            .header("x-api-key", &self.api_key)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 API error: {err}");
        }

        let body: ComposioActionsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v2 actions response")?;
        Ok(body.items)
    }

    /// Build the query-parameter pairs for the Composio v3 `GET /tools`
    /// listing used by [`Self::list_tool_schemas_v3`].
    ///
    /// `toolkits` is sent as a single comma-joined `toolkits=` param (the
    /// legacy plural the v3 backend tolerates; cf. `list_actions_v3` which
    /// sends both the plural and `toolkit_slug` singular forms). `tags` is
    /// encoded as **repeated** `tags=` params (`tags=a&tags=b`) — the shape
    /// Composio v3 `/tools` documents for tag filtering ("can be specified
    /// multiple times"), NOT the comma-joined form the backend proxy uses.
    /// Blank entries are trimmed and dropped; an empty `tags` slice yields
    /// no `tags` params (treated as no filter).
    ///
    /// Pure (no I/O) so the param shape is unit-testable without a live
    /// HTTP round trip — mirrors [`Self::build_execute_action_v3_request`].
    fn build_list_tool_schemas_v3_query(
        toolkits: &[&str],
        tags: Option<&[&str]>,
    ) -> Vec<(&'static str, String)> {
        // #3932: pin toolkit_versions=latest. Without it Composio v3 defaults to
        // the 00000000_00 snapshot, which lists zero tools for any toolkit
        // published after it (Outlook and every other post-launch toolkit).
        let mut params: Vec<(&'static str, String)> = vec![
            ("limit", "200".to_string()),
            ("toolkit_versions", "latest".to_string()),
        ];

        let trimmed: Vec<&str> = toolkits
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        if !trimmed.is_empty() {
            params.push(("toolkits", trimmed.join(",")));
        }

        if let Some(tags) = tags {
            for tag in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                params.push(("tags", tag.to_string()));
            }
        }

        params
    }

    /// List v3 tool definitions for one or more toolkits, preserving the
    /// raw `input_parameters` JSON schema each action carries.
    ///
    /// Sibling of [`Self::list_actions`] but kept distinct because
    /// `list_actions` flattens to `Vec<ComposioAction>` (no parameters)
    /// for the legacy agent-discovery shape, whereas
    /// `composio_list_tools`'s direct-mode branch needs the full schema
    /// so the LLM agent can supply valid arguments without a separate
    /// round trip.
    ///
    /// `toolkits` may contain one or many slugs; when non-empty they are
    /// sent as a comma-separated `toolkits=` filter to constrain the v3
    /// catalogue scan. Empty filter returns every action across every
    /// toolkit on the user's tenant (potentially large; callers should
    /// pass a non-empty filter in practice).
    ///
    /// `tags` narrows the result by Composio action tag (OR semantics —
    /// multiple tags broaden the result). This is the direct-mode (BYO
    /// key) counterpart to the backend proxy's `tags` query param wired
    /// in [`crate::openhuman::integrations::composio::client::ComposioClient::list_tools`];
    /// without it a self-key user's `composio_list_tools(..., tags)`
    /// request would silently drop the tag filter. Blank/empty `tags`
    /// are treated as no filter.
    pub(crate) async fn list_tool_schemas_v3(
        &self,
        toolkits: &[&str],
        tags: Option<&[&str]>,
    ) -> anyhow::Result<Vec<ComposioToolSchemaV3>> {
        let url = format!("{}/tools", self.base_v3);
        let params = Self::build_list_tool_schemas_v3_query(toolkits, tags);
        tracing::debug!(
            toolkits = toolkits.len(),
            tags = tags.map(<[&str]>::len).unwrap_or(0),
            "[composio-direct] list_tool_schemas_v3: GET v3 /tools query built"
        );
        let req = self
            .client()
            .get(&url)
            .header("x-api-key", &self.api_key)
            .query(&params);

        let resp = req.send().await?;
        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 list_tool_schemas: {err}");
        }

        let body: ComposioToolsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 tools response")?;
        Ok(body
            .items
            .into_iter()
            .map(ComposioToolSchemaV3::from_v3_tool)
            .collect())
    }

    /// Execute a Composio action/tool with given parameters.
    ///
    /// Uses v3 endpoint first and falls back to v2 for compatibility.
    pub async fn execute_action(
        &self,
        action_name: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
        connected_account_ref: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        // The Composio v3 action-execute contract keys off the UPPERCASE_SNAKE
        // *action* slug (e.g. `GMAIL_SEND_EMAIL`) at `/tools/execute/{slug}`.
        // The previous code lowercased + dashed it into the *toolkit* slug
        // (`gmail-send-email`) and posted to the wrong `/tools/{slug}/execute`
        // path, so every direct-mode execute 404'd (issue #3219). Pass the
        // action slug through verbatim (trimmed only); the v2 fallback already
        // used the same untransformed name.
        let action_slug = action_name.trim();

        match self
            .execute_action_v3(
                action_slug,
                params.clone(),
                entity_id,
                connected_account_ref,
            )
            .await
        {
            Ok(result) => Ok(result),
            Err(v3_err) => match self.execute_action_v2(action_name, params, entity_id).await {
                Ok(result) => Ok(result),
                Err(v2_err) => anyhow::bail!(
                    "Composio execute failed on v3 ({v3_err}) and v2 fallback ({v2_err})"
                ),
            },
        }
    }

    fn build_execute_action_v3_request(
        action_slug: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
        connected_account_ref: Option<&str>,
    ) -> (String, serde_json::Value) {
        // POST /api/v3/tools/execute/{ACTION_SLUG} — the action slug stays
        // UPPERCASE_SNAKE (see `execute_action`). Path is `/tools/execute/{slug}`,
        // NOT `/tools/{slug}/execute` (issue #3219).
        let url = format!("{COMPOSIO_API_BASE_V3}/tools/execute/{action_slug}");
        let account_ref = connected_account_ref.and_then(|candidate| {
            let trimmed_candidate = candidate.trim();
            (!trimmed_candidate.is_empty()).then_some(trimmed_candidate)
        });

        let mut body = json!({
            "arguments": params,
        });

        if let Some(entity) = entity_id {
            body["user_id"] = json!(entity);
        }
        if let Some(account_ref) = account_ref {
            body["connected_account_id"] = json!(account_ref);
        }

        (url, body)
    }

    async fn execute_action_v3(
        &self,
        action_slug: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
        connected_account_ref: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let (_default_url, body) = Self::build_execute_action_v3_request(
            action_slug,
            params,
            entity_id,
            connected_account_ref,
        );
        let url = format!("{}/tools/execute/{action_slug}", self.base_v3);

        self.ensure_request_url(&url)?;

        let resp = self
            .client()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 action execution failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v3 execute response")?;
        Ok(result)
    }

    async fn execute_action_v2(
        &self,
        action_name: &str,
        params: serde_json::Value,
        entity_id: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let url = format!("{}/actions/{action_name}/execute", self.base_v2);

        let mut body = json!({
            "input": params,
        });

        if let Some(entity) = entity_id {
            body["entityId"] = json!(entity);
        }

        let resp = self
            .client()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 action execution failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v2 execute response")?;
        Ok(result)
    }

    /// Get the OAuth connection URL for a specific app/toolkit or auth config.
    ///
    /// Uses v3 endpoint first and falls back to v2 for compatibility.
    pub async fn get_connection_url(
        &self,
        app_name: Option<&str>,
        auth_config_id: Option<&str>,
        entity_id: &str,
    ) -> anyhow::Result<String> {
        let v3 = self
            .get_connection_url_v3(app_name, auth_config_id, entity_id)
            .await;
        match v3 {
            Ok(url) => Ok(url),
            Err(v3_err) => {
                let app = app_name.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Composio v3 connect failed ({v3_err}) and v2 fallback requires 'app'"
                    )
                })?;
                match self.get_connection_url_v2(app, entity_id).await {
                    Ok(url) => Ok(url),
                    Err(v2_err) => anyhow::bail!(
                        "Composio connect failed on v3 ({v3_err}) and v2 fallback ({v2_err})"
                    ),
                }
            }
        }
    }

    async fn get_connection_url_v3(
        &self,
        app_name: Option<&str>,
        auth_config_id: Option<&str>,
        entity_id: &str,
    ) -> anyhow::Result<String> {
        let auth_config_id = match auth_config_id {
            Some(id) => id.to_string(),
            None => {
                let app = app_name.ok_or_else(|| {
                    anyhow::anyhow!("Missing 'app' or 'auth_config_id' for v3 connect")
                })?;
                self.resolve_auth_config_id(app).await?
            }
        };

        let url = format!("{}/connected_accounts/link", self.base_v3);
        let body = json!({
            "auth_config_id": auth_config_id,
            "user_id": entity_id,
        });

        let resp = self
            .client()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 connect failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v3 connect response")?;
        extract_redirect_url(&result)
            .ok_or_else(|| anyhow::anyhow!("No redirect URL in Composio v3 response"))
    }

    async fn get_connection_url_v2(
        &self,
        app_name: &str,
        entity_id: &str,
    ) -> anyhow::Result<String> {
        let url = format!("{}/connectedAccounts", self.base_v2);

        let body = json!({
            "integrationId": app_name,
            "entityId": entity_id,
        });

        let resp = self
            .client()
            .post(&url)
            .header("x-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v2 connect failed: {err}");
        }

        let result: serde_json::Value = resp
            .json()
            .await
            .context("Failed to decode Composio v2 connect response")?;
        extract_redirect_url(&result)
            .ok_or_else(|| anyhow::anyhow!("No redirect URL in Composio v2 response"))
    }

    /// List the user's connected accounts on Composio v3.
    ///
    /// GET `https://backend.composio.dev/api/v3/connected_accounts` with
    /// `x-api-key: <user_key>`. Returns the raw item list; reshaping
    /// into [`super::super::super::composio::types::ComposioConnection`]
    /// happens at the call site in `composio/client.rs::direct_list_connections`.
    ///
    /// The v3 envelope is `{ items: [{ id, status, toolkit, created_at, ... }] }`.
    /// Toolkit may arrive as either a plain string slug or as a nested
    /// object — we tolerate both via [`ComposioConnectedAccount::toolkit_slug`].
    /// This matches the same upstream shape drift handled by
    /// `de_string_or_object` in `composio/types.rs`.
    pub async fn list_connected_accounts(&self) -> anyhow::Result<Vec<ComposioConnectedAccount>> {
        let url = format!("{}/connected_accounts", self.base_v3);
        self.ensure_request_url(&url)?;

        let resp = self
            .client()
            .get(&url)
            .header("x-api-key", &self.api_key)
            // Composio paginates; pull a generous page size so most
            // users see their full list in one round trip. If a user has
            // > 200 connected accounts (extremely rare for an individual
            // tenant) the rest will be missing until we add explicit
            // pagination — note for the follow-up.
            .query(&[("limit", "200")])
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 connected_accounts failed: {err}");
        }

        let mut body: ComposioConnectedAccountsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 connected_accounts response")?;
        // Drop rows with a blank id — serde_default means id can be ""
        // if the upstream response is malformed. An empty connectionId
        // propagated downstream causes invalid v3 API calls.
        body.items.retain(|item| !item.id.trim().is_empty());
        tracing::debug!(
            count = body.items.len(),
            "[composio-direct] list_connected_accounts: fetched connected accounts"
        );
        Ok(body.items)
    }

    async fn resolve_auth_config_id(&self, app_name: &str) -> anyhow::Result<String> {
        let url = format!("{}/auth_configs", self.base_v3);

        let resp = self
            .client()
            .get(&url)
            .header("x-api-key", &self.api_key)
            .query(&[
                ("toolkit_slug", app_name),
                ("show_disabled", "true"),
                ("limit", "25"),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let err = response_error(resp).await;
            anyhow::bail!("Composio v3 auth config lookup failed: {err}");
        }

        let body: ComposioAuthConfigsResponse = resp
            .json()
            .await
            .context("Failed to decode Composio v3 auth configs response")?;

        if body.items.is_empty() {
            anyhow::bail!(
                "No auth config found for toolkit '{app_name}'. Create one in Composio first."
            );
        }

        let preferred = body
            .items
            .iter()
            .find(|cfg| cfg.is_enabled())
            .or_else(|| body.items.first())
            .context("No usable auth config returned by Composio")?;

        Ok(preferred.id.clone())
    }
}
