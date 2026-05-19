use super::clob_auth::{derive_credentials, sign_clob_headers};
use super::polymarket_orders::{sign_order, Order};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::{PolymarketClobCredentials, PolymarketConfig};
use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolCategory, ToolResult};
use crate::openhuman::wallet::{secret_material, status as wallet_status, WalletChain};
use anyhow::{Context, Result};
use async_trait::async_trait;
use ethers_core::types::Address;
use ethers_signers::{coins_bip39::English, LocalWallet, MnemonicBuilder, Signer};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

const MAX_RETRY_ATTEMPTS: usize = 3;
const RETRY_BACKOFF_MS: u64 = 500;
const CONNECT_TIMEOUT_SECS: u64 = 10;
const MAX_ERROR_BODY_CHARS: usize = 240;
const POLY_CHAIN_ID: u64 = 137;
const POLY_COLLATERAL_DECIMALS: u32 = 6;
const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";

fn ensure_https(url: &str) -> Result<()> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if url.starts_with("http://127.0.0.1")
        || url.starts_with("http://[::1]")
        || url.starts_with("http://localhost")
    {
        return Ok(());
    }
    anyhow::bail!(
        "Refusing to transmit Polymarket CLOB credentials over non-HTTPS URL: \
         URL scheme must be https (loopback http allowed for local mock)"
    )
}

/// Polymarket market + trading tool (Gamma + CLOB APIs).
pub struct PolymarketTool {
    gamma_base_url: String,
    clob_base_url: String,
    polygon_rpc_url: String,
    usdc_contract: String,
    clob_exchange_contract: String,
    default_eoa_address: Option<String>,
    http: Client,
    security: Arc<SecurityPolicy>,
    timeout: Duration,
    cached_clob_credentials: Mutex<Option<PolymarketClobCredentials>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum PolymarketRequest {
    ListMarkets {
        #[serde(default)]
        slug: Option<String>,
        #[serde(default)]
        event_id: Option<String>,
        #[serde(default)]
        limit: Option<u64>,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        active: Option<bool>,
        #[serde(default)]
        closed: Option<bool>,
        #[serde(default)]
        tag: Option<String>,
    },
    GetMarket {
        #[serde(default)]
        market_id: Option<String>,
        #[serde(default)]
        slug: Option<String>,
    },
    ListEvents {
        #[serde(default)]
        event_id: Option<String>,
        #[serde(default)]
        limit: Option<u64>,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        cursor: Option<String>,
        #[serde(default)]
        active: Option<bool>,
        #[serde(default)]
        closed: Option<bool>,
        #[serde(default)]
        tag: Option<String>,
    },
    GetOrderbook {
        token_id: String,
    },
    GetPrice {
        token_id: String,
        side: String,
    },
    GetPositions {
        #[serde(default)]
        user: Option<String>,
    },
    GetBalance {
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        token: Option<String>,
    },
    GetOpenOrders {
        #[serde(default)]
        user: Option<String>,
    },
    GetUsdcAllowance {
        #[serde(default)]
        user: Option<String>,
    },
    PlaceOrder {
        side: String,
        token_id: String,
        price: f64,
        size: f64,
        #[serde(default)]
        time_in_force: Option<String>,
        #[serde(default)]
        expiration_secs: Option<u64>,
        #[serde(default)]
        approved: Option<bool>,
        #[serde(default)]
        user: Option<String>,
    },
    CancelOrder {
        order_id: String,
        #[serde(default)]
        approved: Option<bool>,
        #[serde(default)]
        user: Option<String>,
    },
}

impl PolymarketTool {
    pub fn new(config: &PolymarketConfig, security: Arc<SecurityPolicy>) -> Self {
        let timeout = Duration::from_secs(config.timeout_secs.max(1));

        let builder = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none());
        let builder =
            crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.polymarket");

        let http = builder.build().unwrap_or_else(|err| {
            panic!(
                "[polymarket] failed to build HTTP client (proxy/timeout configuration): {err}. \
                 Refusing to fall back to Client::new() — silent fallback hides the misconfiguration \
                 and produces requests that bypass the configured proxy + timeouts."
            )
        });

        let cached_credentials = config
            .derived_clob_credentials
            .clone()
            .map(PolymarketClobCredentials::from)
            .filter(PolymarketClobCredentials::is_complete);

        Self {
            gamma_base_url: normalize_base_url(
                &config.gamma_base_url,
                "https://gamma-api.polymarket.com",
            ),
            clob_base_url: normalize_base_url(&config.clob_base_url, "https://clob.polymarket.com"),
            polygon_rpc_url: normalize_base_url(&config.polygon_rpc_url, "https://polygon-rpc.com"),
            usdc_contract: config.usdc_contract.trim().to_string(),
            clob_exchange_contract: config.clob_exchange_contract.trim().to_string(),
            default_eoa_address: config
                .eoa_address
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string),
            http,
            security,
            timeout,
            cached_clob_credentials: Mutex::new(cached_credentials),
        }
    }

    async fn handle_request(&self, request: PolymarketRequest) -> Result<Value> {
        match request {
            PolymarketRequest::ListMarkets {
                slug,
                event_id,
                limit,
                offset,
                cursor,
                active,
                closed,
                tag,
            } => {
                let mut query = Vec::new();
                push_optional_string(&mut query, "slug", slug);
                push_optional_string(&mut query, "event_id", event_id);
                push_optional_u64(&mut query, "limit", limit);
                push_optional_u64(&mut query, "offset", offset);
                push_optional_string(&mut query, "cursor", cursor);
                push_optional_bool(&mut query, "active", active);
                push_optional_bool(&mut query, "closed", closed);
                push_optional_string(&mut query, "tag", tag);

                let data = self
                    .get_json(&self.gamma_base_url, "/markets", &query, None)
                    .await?;
                Ok(json!({
                    "action": "list_markets",
                    "source": "gamma",
                    "data": data,
                }))
            }
            PolymarketRequest::GetMarket { market_id, slug } => {
                if let Some(market_id) = non_empty(market_id.as_deref()) {
                    let path = format!("/markets/{market_id}");
                    let data = self
                        .get_json(&self.gamma_base_url, &path, &[], None)
                        .await?;
                    Ok(json!({
                        "action": "get_market",
                        "source": "gamma",
                        "lookup": "market_id",
                        "market_id": market_id,
                        "data": data,
                    }))
                } else if let Some(slug) = non_empty(slug.as_deref()) {
                    let data = self
                        .get_json(
                            &self.gamma_base_url,
                            "/markets",
                            &[("slug".to_string(), slug.to_string())],
                            None,
                        )
                        .await?;

                    let market = first_item_from_collection(data, slug)?;
                    Ok(json!({
                        "action": "get_market",
                        "source": "gamma",
                        "lookup": "slug",
                        "slug": slug,
                        "data": market,
                    }))
                } else {
                    anyhow::bail!("get_market requires either 'market_id' or 'slug'")
                }
            }
            PolymarketRequest::ListEvents {
                event_id,
                limit,
                offset,
                cursor,
                active,
                closed,
                tag,
            } => {
                if let Some(event_id) = non_empty(event_id.as_deref()) {
                    let path = format!("/events/{event_id}");
                    let data = self
                        .get_json(&self.gamma_base_url, &path, &[], None)
                        .await?;
                    Ok(json!({
                        "action": "list_events",
                        "source": "gamma",
                        "lookup": "event_id",
                        "event_id": event_id,
                        "data": data,
                    }))
                } else {
                    let mut query = Vec::new();
                    push_optional_u64(&mut query, "limit", limit);
                    push_optional_u64(&mut query, "offset", offset);
                    push_optional_string(&mut query, "cursor", cursor);
                    push_optional_bool(&mut query, "active", active);
                    push_optional_bool(&mut query, "closed", closed);
                    push_optional_string(&mut query, "tag", tag);

                    let data = self
                        .get_json(&self.gamma_base_url, "/events", &query, None)
                        .await?;
                    Ok(json!({
                        "action": "list_events",
                        "source": "gamma",
                        "data": data,
                    }))
                }
            }
            PolymarketRequest::GetOrderbook { token_id } => {
                let token_id = non_empty(Some(token_id.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'token_id' cannot be empty"))?;

                let data = self
                    .get_json(
                        &self.clob_base_url,
                        "/book",
                        &[("token_id".to_string(), token_id.to_string())],
                        None,
                    )
                    .await?;

                Ok(json!({
                    "action": "get_orderbook",
                    "source": "clob",
                    "token_id": token_id,
                    "data": data,
                }))
            }
            PolymarketRequest::GetPrice { token_id, side } => {
                let token_id = non_empty(Some(token_id.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'token_id' cannot be empty"))?;
                let side = normalize_side(&side)?;

                let data = self
                    .get_json(
                        &self.clob_base_url,
                        "/price",
                        &[
                            ("token_id".to_string(), token_id.to_string()),
                            ("side".to_string(), side.to_string()),
                        ],
                        None,
                    )
                    .await?;

                Ok(json!({
                    "action": "get_price",
                    "source": "clob",
                    "token_id": token_id,
                    "side": side,
                    "data": data,
                }))
            }
            PolymarketRequest::GetPositions { user } => {
                let user = self.resolve_user_address(user).await?;
                let query = vec![("user".to_string(), user.clone())];
                let data = self
                    .get_signed_clob_json("/data/positions", &query, &user)
                    .await?;

                Ok(json!({
                    "action": "get_positions",
                    "source": "clob",
                    "user": user,
                    "data": data,
                }))
            }
            PolymarketRequest::GetBalance { user, token } => {
                let user = self.resolve_user_address(user).await?;
                let mut query = vec![("user".to_string(), user.clone())];
                query.push((
                    "token".to_string(),
                    non_empty(token.as_deref()).unwrap_or("usdce").to_string(),
                ));

                let data = self
                    .get_signed_clob_json("/data/balance", &query, &user)
                    .await?;
                Ok(json!({
                    "action": "get_balance",
                    "source": "clob",
                    "user": user,
                    "data": data,
                }))
            }
            PolymarketRequest::GetOpenOrders { user } => {
                let user = self.resolve_user_address(user).await?;
                let query = vec![("user".to_string(), user.clone())];
                let data = self.get_signed_clob_json("/orders", &query, &user).await?;

                Ok(json!({
                    "action": "get_open_orders",
                    "source": "clob",
                    "user": user,
                    "data": data,
                }))
            }
            PolymarketRequest::GetUsdcAllowance { user } => {
                let user = self.resolve_user_address(user).await?;
                let allowance = self.get_usdc_allowance_for_user(&user).await?;

                Ok(json!({
                    "action": "get_usdc_allowance",
                    "source": "polygon",
                    "user": user,
                    "token_contract": self.usdc_contract,
                    "spender": self.clob_exchange_contract,
                    "allowance": allowance,
                }))
            }
            PolymarketRequest::PlaceOrder {
                side,
                token_id,
                price,
                size,
                time_in_force,
                expiration_secs,
                approved,
                user,
            } => {
                self.security
                    .enforce_tool_operation(ToolOperation::Act, "polymarket.place_order")
                    .map_err(anyhow::Error::msg)?;
                require_write_approval(approved)?;

                let (wallet, user) = self.resolve_signer_and_user(user).await?;
                let creds = self.ensure_clob_credentials(&wallet, &user).await?;
                let nonce = self.fetch_order_nonce(&user, &creds).await?;
                let signed_order = self
                    .build_signed_limit_order(
                        &wallet,
                        &user,
                        &token_id,
                        &side,
                        price,
                        size,
                        nonce,
                        expiration_secs,
                    )
                    .await?;

                let tif = normalize_time_in_force(time_in_force.as_deref())?;
                let body = json!({
                    "order": signed_order,
                    "owner": user,
                    "orderType": "limit",
                    "timeInForce": tif,
                });

                let data = self
                    .post_signed_clob_json("/order", body, &user, &creds)
                    .await?;
                Ok(json!({
                    "action": "place_order",
                    "source": "clob",
                    "user": user,
                    "time_in_force": tif,
                    "data": data,
                }))
            }
            PolymarketRequest::CancelOrder {
                order_id,
                approved,
                user,
            } => {
                self.security
                    .enforce_tool_operation(ToolOperation::Act, "polymarket.cancel_order")
                    .map_err(anyhow::Error::msg)?;
                require_write_approval(approved)?;

                let (wallet, user) = self.resolve_signer_and_user(user).await?;
                let creds = self
                    .cached_or_derive_credentials_with_signer(&user, Some(&wallet))
                    .await?;
                let order_id = non_empty(Some(order_id.as_str()))
                    .ok_or_else(|| anyhow::anyhow!("'order_id' cannot be empty"))?;
                let path = format!("/order/{order_id}");

                let data = self.delete_signed_clob_json(&path, &user, &creds).await?;
                Ok(json!({
                    "action": "cancel_order",
                    "source": "clob",
                    "order_id": order_id,
                    "data": data,
                }))
            }
        }
    }

    async fn get_signed_clob_json(
        &self,
        path: &str,
        query: &[(String, String)],
        user: &str,
    ) -> Result<Value> {
        let creds = self.cached_or_derive_credentials(user).await?;
        self.get_signed_clob_json_with_creds(path, query, user, &creds)
            .await
    }

    async fn get_signed_clob_json_with_creds(
        &self,
        path: &str,
        query: &[(String, String)],
        user: &str,
        creds: &PolymarketClobCredentials,
    ) -> Result<Value> {
        ensure_https(&self.clob_base_url)?;
        let signed_path = signed_request_path(path, query);
        let headers = sign_clob_headers(creds, user, "GET", &signed_path, None)?;
        self.get_json(&self.clob_base_url, path, query, Some(headers))
            .await
    }

    async fn post_signed_clob_json(
        &self,
        path: &str,
        body: Value,
        user: &str,
        creds: &PolymarketClobCredentials,
    ) -> Result<Value> {
        ensure_https(&self.clob_base_url)?;
        let body_raw =
            serde_json::to_string(&body).context("Failed to serialize CLOB POST body")?;
        let mut headers = sign_clob_headers(creds, user, "POST", path, Some(&body_raw))?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        self.post_json_raw(&self.clob_base_url, path, Some(headers), body_raw)
            .await
    }

    async fn delete_signed_clob_json(
        &self,
        path: &str,
        user: &str,
        creds: &PolymarketClobCredentials,
    ) -> Result<Value> {
        ensure_https(&self.clob_base_url)?;
        let headers = sign_clob_headers(creds, user, "DELETE", path, None)?;
        self.delete_json(&self.clob_base_url, path, Some(headers))
            .await
    }

    async fn cached_or_derive_credentials(&self, user: &str) -> Result<PolymarketClobCredentials> {
        self.cached_or_derive_credentials_with_signer(user, None)
            .await
    }

    async fn cached_or_derive_credentials_with_signer(
        &self,
        user: &str,
        signer: Option<&LocalWallet>,
    ) -> Result<PolymarketClobCredentials> {
        if let Some(creds) = self.cached_clob_credentials.lock().await.clone() {
            return Ok(creds);
        }

        if let Some(wallet) = signer {
            return self.ensure_clob_credentials(wallet, user).await;
        }

        let (wallet, resolved_user) = self.resolve_signer_and_user(Some(user.to_string())).await?;
        self.ensure_clob_credentials(&wallet, &resolved_user).await
    }

    async fn ensure_clob_credentials(
        &self,
        wallet: &LocalWallet,
        user: &str,
    ) -> Result<PolymarketClobCredentials> {
        // Hold the cache lock across the derive call so two concurrent
        // callers cannot both observe an empty cache and each fire their
        // own POST /auth/api-key — the CLOB would silently retain only
        // one set of credentials and reject signatures generated with
        // the other. tokio::sync::Mutex is intentional here (not std)
        // because the critical section spans .await points.
        let mut guard = self.cached_clob_credentials.lock().await;
        if let Some(creds) = guard.as_ref() {
            return Ok(creds.clone());
        }

        ensure_https(&self.clob_base_url)?;
        let creds =
            derive_credentials(&self.http, wallet, &self.clob_base_url, POLY_CHAIN_ID, user)
                .await
                .context("Failed to derive Polymarket CLOB API credentials")?;

        *guard = Some(creds.clone());
        // Drop the cache lock before the (slow + best-effort) config
        // persist so other tool calls can proceed against the freshly
        // populated cache.
        drop(guard);

        if let Err(err) = self.persist_clob_credentials(&creds).await {
            tracing::warn!(reason = %err, "[polymarket] failed to persist derived CLOB credentials");
        }

        Ok(creds)
    }

    /// Persists derived CLOB credentials to the config TOML.
    ///
    /// **Best-effort only.** The in-memory `cached_clob_credentials` Mutex
    /// is the authoritative source within the lifetime of this
    /// PolymarketTool instance — `persist_clob_credentials` exists so that
    /// a future process restart can reuse the L2 key/secret without
    /// re-running the L1 EIP-712 handshake. The caller already logs a
    /// warn on failure and continues; the live request path is unaffected.
    ///
    /// Concurrent persists from multiple tools could clobber each other
    /// because the whole config is loaded-then-saved. Acceptable for the
    /// current single-process model; will move to the `SecretStore`
    /// transactional surface once #1900 lands.
    async fn persist_clob_credentials(&self, creds: &PolymarketClobCredentials) -> Result<()> {
        let mut config = config_rpc::load_config_with_timeout()
            .await
            .map_err(anyhow::Error::msg)
            .context("Failed to load config for persisting Polymarket credentials")?;

        config.integrations.polymarket.derived_clob_credentials = Some(creds.clone().into());
        config
            .save()
            .await
            .context("Failed to save config with Polymarket credentials")?;
        Ok(())
    }

    async fn resolve_user_address(&self, user: Option<String>) -> Result<String> {
        if let Some(user) = user.and_then(|v| non_empty(Some(v.as_str())).map(str::to_string)) {
            return validate_evm_address(&user);
        }

        if let Some(user) = self
            .default_eoa_address
            .as_deref()
            .and_then(|v| non_empty(Some(v)))
            .map(str::to_string)
        {
            return validate_evm_address(&user);
        }

        let status = wallet_status().await.map_err(anyhow::Error::msg)?;
        if let Some(account) = status
            .value
            .accounts
            .into_iter()
            .find(|account| account.chain == WalletChain::Evm)
        {
            return validate_evm_address(&account.address);
        }

        anyhow::bail!(
            "No Polymarket EOA address available. Provide 'user', configure integrations.polymarket.eoa_address, or run wallet setup."
        )
    }

    async fn resolve_signer_and_user(&self, user: Option<String>) -> Result<(LocalWallet, String)> {
        let user = self.resolve_user_address(user).await?;

        let secret = secret_material(WalletChain::Evm)
            .await
            .map_err(anyhow::Error::msg)
            .context("Polymarket writes require wallet secret material")?;

        let config = config_rpc::load_config_with_timeout()
            .await
            .map_err(anyhow::Error::msg)
            .context("Failed to load config for wallet secret decryption")?;

        let mnemonic =
            crate::openhuman::encryption::rpc::decrypt_secret(&config, &secret.encrypted_mnemonic)
                .await
                .map_err(anyhow::Error::msg)
                .context("Failed to decrypt wallet mnemonic")?
                .value;

        let wallet = MnemonicBuilder::<English>::default()
            .phrase(mnemonic.as_str())
            .derivation_path(&secret.derivation_path)
            .map_err(|e| {
                anyhow::anyhow!(
                    "Invalid EVM derivation path '{}': {e}",
                    secret.derivation_path
                )
            })?
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to derive EVM signer from wallet secret: {e}"))?
            .with_chain_id(POLY_CHAIN_ID);

        let wallet_addr = format!("{:#x}", wallet.address());
        if !same_evm_address(&wallet_addr, &user) {
            anyhow::bail!(
                "Configured/requested user address '{}' does not match wallet signer '{}'.",
                user,
                wallet_addr
            );
        }

        Ok((wallet, wallet_addr))
    }

    async fn get_usdc_allowance_for_user(&self, user: &str) -> Result<String> {
        // Polygon RPC payload carries the EOA address. Even though no CLOB
        // credentials are sent here, leaking the address over plaintext HTTP
        // still narrows the user's identity to anyone on-path. Same loopback
        // carve-out as the CLOB guard so the mock harness keeps working.
        ensure_https(&self.polygon_rpc_url)?;

        let owner = Address::from_str(user)
            .with_context(|| format!("Invalid owner EVM address '{user}'"))?;
        let spender = Address::from_str(&self.clob_exchange_contract).with_context(|| {
            format!(
                "Invalid clob_exchange_contract address '{}'",
                self.clob_exchange_contract
            )
        })?;
        let token_contract = Address::from_str(&self.usdc_contract)
            .with_context(|| format!("Invalid usdc_contract address '{}'", self.usdc_contract))?;

        // Hand-rolled ABI encoding for ERC-20 `allowance(address,address)`:
        //   * selector `0xdd62ed3e` = first 4 bytes of keccak256("allowance(address,address)")
        //   * each Address is 20 bytes = 40 hex chars; `:0>64` left-pads to
        //     the 32-byte ABI word (12 bytes of leading zeros)
        // Hand-rolled because pulling alloy-sol-types or ethabi for a single
        // view call would balloon the dependency tree; the encoding is
        // small and frozen.
        let data = format!(
            "0xdd62ed3e{:0>64}{:0>64}",
            hex::encode(owner.as_bytes()),
            hex::encode(spender.as_bytes()),
        );
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [
                {
                    "to": format!("{token_contract:#x}"),
                    "data": data,
                },
                "latest"
            ]
        });

        let mut rpc_headers = HeaderMap::new();
        rpc_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        let response = self
            .post_json_raw(
                &self.polygon_rpc_url,
                "",
                Some(rpc_headers),
                serde_json::to_string(&payload)
                    .context("Failed to serialize Polygon RPC payload")?,
            )
            .await?;

        let hex_value = response
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Polygon RPC response missing 'result'"))?;

        let allowance = parse_hex_u256(hex_value)?;
        Ok(allowance.to_string())
    }

    async fn fetch_order_nonce(
        &self,
        user: &str,
        creds: &PolymarketClobCredentials,
    ) -> Result<u64> {
        let query = vec![("user".to_string(), user.to_string())];
        let payload = self
            .get_signed_clob_json_with_creds("/nonce", &query, user, creds)
            .await
            .context("Failed to fetch Polymarket order nonce")?;

        parse_order_nonce(&payload)
    }

    async fn build_signed_limit_order(
        &self,
        wallet: &LocalWallet,
        user: &str,
        token_id: &str,
        side: &str,
        price: f64,
        size: f64,
        nonce: u64,
        expiration_secs: Option<u64>,
    ) -> Result<Value> {
        if !price.is_finite() || price <= 0.0 || price >= 1.0 {
            anyhow::bail!("'price' must be between 0 and 1 (exclusive)");
        }
        if !size.is_finite() || size <= 0.0 {
            anyhow::bail!("'size' must be greater than zero");
        }

        let side = normalize_order_side(side)?;
        let token_id = non_empty(Some(token_id))
            .ok_or_else(|| anyhow::anyhow!("'token_id' cannot be empty"))?;

        let rounded_price = round_normal(price, 4);
        let rounded_size = round_down(size, 2);
        let (maker_raw, taker_raw) = if side == "BUY" {
            let taker_raw = rounded_size;
            let maker_raw = round_down(taker_raw * rounded_price, 4);
            (maker_raw, taker_raw)
        } else {
            let maker_raw = rounded_size;
            let taker_raw = round_down(maker_raw * rounded_price, 4);
            (maker_raw, taker_raw)
        };

        let maker_amount = to_fixed_units_string(maker_raw, POLY_COLLATERAL_DECIMALS)?;
        let taker_amount = to_fixed_units_string(taker_raw, POLY_COLLATERAL_DECIMALS)?;
        if maker_amount == "0" || taker_amount == "0" {
            anyhow::bail!("Order size/price rounds to zero after fixed-point conversion")
        }

        let expiration = expiration_secs
            .map(|secs| secs.to_string())
            .unwrap_or_else(|| "0".to_string());

        // Cryptographically-random salt — non-CSPRNG (rand::random) is
        // predictable enough to enable order-replay/front-running attacks
        // against the CLOB. OsRng pulls from the OS entropy source. Same
        // pattern used by src/openhuman/encryption/core.rs.
        let salt = {
            use chacha20poly1305::aead::rand_core::RngCore;
            use chacha20poly1305::aead::OsRng;
            let mut buf = [0_u8; 8];
            OsRng.fill_bytes(&mut buf);
            u64::from_le_bytes(buf)
        };

        let order = Order {
            salt: salt.to_string(),
            maker: user.to_string(),
            signer: user.to_string(),
            taker: ZERO_ADDRESS.to_string(),
            token_id: token_id.to_string(),
            maker_amount,
            taker_amount,
            expiration,
            nonce: nonce.to_string(),
            fee_rate_bps: "0".to_string(),
            side: side.to_string(),
            signature_type: 0,
        };

        let contract = Address::from_str(&self.clob_exchange_contract).with_context(|| {
            format!(
                "Invalid clob_exchange_contract address '{}'",
                self.clob_exchange_contract
            )
        })?;

        let signature = sign_order(&order, wallet, POLY_CHAIN_ID, contract)
            .await
            .context("Failed to EIP-712 sign Polymarket order")?;

        Ok(json!({
            "salt": order.salt.parse::<u64>().unwrap_or_default(),
            "maker": order.maker,
            "signer": order.signer,
            "taker": order.taker,
            "tokenId": order.token_id,
            "makerAmount": order.maker_amount,
            "takerAmount": order.taker_amount,
            "expiration": order.expiration,
            "nonce": order.nonce,
            "feeRateBps": order.fee_rate_bps,
            "side": order.side,
            "signatureType": order.signature_type,
            "signature": signature,
        }))
    }

    async fn get_json(
        &self,
        base_url: &str,
        path: &str,
        query: &[(String, String)],
        headers: Option<HeaderMap>,
    ) -> Result<Value> {
        self.send_with_retry(reqwest::Method::GET, base_url, path, query, headers, None)
            .await
    }

    async fn post_json_raw(
        &self,
        base_url: &str,
        path: &str,
        headers: Option<HeaderMap>,
        body_raw: String,
    ) -> Result<Value> {
        self.send_with_retry(
            reqwest::Method::POST,
            base_url,
            path,
            &[],
            headers,
            Some(body_raw),
        )
        .await
    }

    async fn delete_json(
        &self,
        base_url: &str,
        path: &str,
        headers: Option<HeaderMap>,
    ) -> Result<Value> {
        self.send_with_retry(reqwest::Method::DELETE, base_url, path, &[], headers, None)
            .await
    }

    /// Shared HTTP send + retry-on-transient + status-class classification
    /// loop. All Polymarket HTTP exits funnel through here so the retry
    /// policy, timeout handling, and error-message shape stay consistent.
    async fn send_with_retry(
        &self,
        method: reqwest::Method,
        base_url: &str,
        path: &str,
        query: &[(String, String)],
        headers: Option<HeaderMap>,
        body: Option<String>,
    ) -> Result<Value> {
        let url = format!("{base_url}{path}");
        let method_label = method.as_str().to_string();

        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            let mut request = self.http.request(method.clone(), &url);
            if !query.is_empty() {
                request = request.query(query);
            }
            if let Some(h) = headers.as_ref() {
                request = request.headers(h.clone());
            }
            if let Some(b) = body.as_ref() {
                request = request.body(b.clone());
            }

            let response = match request.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    if err.is_timeout() {
                        anyhow::bail!(
                            "Polymarket request timed out after {}s: {method_label} {path}",
                            self.timeout.as_secs()
                        );
                    }

                    if attempt < MAX_RETRY_ATTEMPTS {
                        sleep(Duration::from_millis(RETRY_BACKOFF_MS)).await;
                        continue;
                    }

                    anyhow::bail!(
                        "Polymarket transient transport error for {method_label} {path}: {err} (url: {url})"
                    );
                }
            };

            let status = response.status();

            if status.is_success() {
                let text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| String::from("null"));
                if text.trim().is_empty() {
                    return Ok(Value::Null);
                }
                return serde_json::from_str(&text)
                    .with_context(|| format!("Failed to deserialize Polymarket response: {path}"));
            }

            let body = response
                .text()
                .await
                .unwrap_or_else(|_| String::from("<failed to read response body>"));
            let detail = summarize_error_body(&body);

            if status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS {
                anyhow::bail!(
                    "Polymarket client error {} for {method_label} {path}: {detail}",
                    status.as_u16()
                );
            }

            let transient = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if transient && attempt < MAX_RETRY_ATTEMPTS {
                sleep(Duration::from_millis(RETRY_BACKOFF_MS)).await;
                continue;
            }

            if status == StatusCode::TOO_MANY_REQUESTS {
                anyhow::bail!(
                    "Polymarket transient rate-limit error {} for {method_label} {path} after {attempt} attempts: {detail}",
                    status.as_u16()
                );
            }

            if status.is_server_error() {
                anyhow::bail!(
                    "Polymarket transient server error {} for {method_label} {path} after {attempt} attempts: {detail}",
                    status.as_u16()
                );
            }

            anyhow::bail!(
                "Polymarket HTTP error {} for {method_label} {path}: {detail}",
                status.as_u16()
            );
        }

        anyhow::bail!("Polymarket request failed: retry budget exhausted")
    }
}

#[async_trait]
impl Tool for PolymarketTool {
    fn name(&self) -> &str {
        "polymarket"
    }

    fn description(&self) -> &str {
        "Browse and trade Polymarket via Gamma + CLOB APIs. Supports market/event discovery, account reads, and signed order placement/cancellation."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Polymarket action to run.",
                    "enum": [
                        "list_markets",
                        "get_market",
                        "list_events",
                        "get_orderbook",
                        "get_price",
                        "get_positions",
                        "get_balance",
                        "get_open_orders",
                        "get_usdc_allowance",
                        "place_order",
                        "cancel_order"
                    ]
                },
                "market_id": {
                    "type": "string",
                    "description": "Gamma market id for get_market."
                },
                "event_id": {
                    "type": "string",
                    "description": "Optional event id filter, or exact id for list_events."
                },
                "slug": {
                    "type": "string",
                    "description": "Market slug filter; can also be used to resolve get_market."
                },
                "token_id": {
                    "type": "string",
                    "description": "CLOB token id for get_orderbook/get_price/place_order."
                },
                "side": {
                    "type": "string",
                    "description": "Side for get_price/place_order.",
                    "enum": ["buy", "sell", "BUY", "SELL"]
                },
                "price": {
                    "type": "number",
                    "description": "Limit price for place_order (0 < price < 1)."
                },
                "size": {
                    "type": "number",
                    "description": "Order size in shares for place_order."
                },
                "time_in_force": {
                    "type": "string",
                    "description": "Time in force for place_order.",
                    "enum": ["GTC", "GTD", "FOK", "FAK"]
                },
                "expiration_secs": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional expiration timestamp (UTC seconds) for place_order."
                },
                "order_id": {
                    "type": "string",
                    "description": "Order id/hash to cancel."
                },
                "user": {
                    "type": "string",
                    "description": "Optional user wallet address override for authenticated actions."
                },
                "token": {
                    "type": "string",
                    "description": "Collateral token key for get_balance (default: usdce)."
                },
                "approved": {
                    "type": "boolean",
                    "description": "Required=true for write actions (place_order, cancel_order)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Pagination limit for list_markets/list_events."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Pagination offset for list_markets/list_events."
                },
                "cursor": {
                    "type": "string",
                    "description": "Cursor token for paginated list responses."
                },
                "active": {
                    "type": "boolean",
                    "description": "Filter active markets/events."
                },
                "closed": {
                    "type": "boolean",
                    "description": "Filter closed markets/events."
                },
                "tag": {
                    "type": "string",
                    "description": "Optional topic/tag filter."
                }
            },
            "required": ["action"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Skill
    }

    fn is_concurrency_safe(&self, args: &Value) -> bool {
        // Write actions are NOT concurrency-safe — two concurrent place_order
        // calls would each call /nonce?user=<eoa> independently and receive
        // the same nonce, causing one of the signed orders to be silently
        // rejected by the CLOB. Credential derivation is similarly
        // single-flight (see ensure_clob_credentials OnceCell). Reads remain
        // concurrency-safe.
        match args.get("action").and_then(Value::as_str) {
            Some("place_order") | Some("cancel_order") => false,
            _ => true,
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }

        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        let request: PolymarketRequest = serde_json::from_value(args)
            .context("Invalid polymarket request: unable to parse parameters")?;

        match self.handle_request(request).await {
            Ok(payload) => Ok(ToolResult::json(payload)),
            Err(err) => Ok(ToolResult::error(err.to_string())),
        }
    }
}

fn normalize_base_url(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed.trim_end_matches('/').to_string()
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn push_optional_string(query: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(value) = non_empty(value.as_deref()) {
        query.push((key.to_string(), value.to_string()));
    }
}

fn push_optional_u64(query: &mut Vec<(String, String)>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        query.push((key.to_string(), value.to_string()));
    }
}

fn push_optional_bool(query: &mut Vec<(String, String)>, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        query.push((key.to_string(), value.to_string()));
    }
}

fn normalize_side(side: &str) -> Result<&'static str> {
    let side = side.trim().to_ascii_lowercase();
    match side.as_str() {
        "buy" => Ok("buy"),
        "sell" => Ok("sell"),
        _ => anyhow::bail!("Invalid 'side'. Expected one of: buy, sell"),
    }
}

fn normalize_order_side(side: &str) -> Result<&'static str> {
    let side = side.trim().to_ascii_uppercase();
    match side.as_str() {
        "BUY" => Ok("BUY"),
        "SELL" => Ok("SELL"),
        _ => anyhow::bail!("Invalid 'side'. Expected one of: BUY, SELL"),
    }
}

fn normalize_time_in_force(value: Option<&str>) -> Result<&'static str> {
    let value = value.unwrap_or("GTC").trim().to_ascii_uppercase();
    match value.as_str() {
        "GTC" => Ok("GTC"),
        "GTD" => Ok("GTD"),
        "FOK" => Ok("FOK"),
        "FAK" => Ok("FAK"),
        _ => anyhow::bail!("Invalid 'time_in_force'. Expected one of: GTC, GTD, FOK, FAK"),
    }
}

fn signed_request_path(path: &str, query: &[(String, String)]) -> String {
    if query.is_empty() {
        return path.to_string();
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in query {
        serializer.append_pair(key, value);
    }
    let encoded = serializer.finish();
    format!("{path}?{encoded}")
}

fn summarize_error_body(body: &str) -> String {
    let compact = body.trim().replace('\n', " ");
    if compact.is_empty() {
        "empty response body".to_string()
    } else {
        crate::openhuman::util::truncate_with_ellipsis(&compact, MAX_ERROR_BODY_CHARS)
    }
}

fn first_item_from_collection(data: Value, slug: &str) -> Result<Value> {
    match data {
        Value::Array(items) => items
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No Polymarket market found for slug '{slug}'")),
        Value::Object(mut map) => {
            if let Some(Value::Array(items)) = map.remove("data") {
                return items.into_iter().next().ok_or_else(|| {
                    anyhow::anyhow!("No Polymarket market found for slug '{slug}'")
                });
            }
            Ok(Value::Object(map))
        }
        other => Ok(other),
    }
}

/// Approval-flag stopgap for Polymarket writes.
///
/// **Security note:** this is a client-enforced string check on the
/// arguments JSON. If the agent harness ever invokes `execute()` without
/// the normal approval channel — or if an LLM hallucinates
/// `"approved": true` into the arguments — this gate alone stands between
/// the model and a live, EIP-712-signed order on Polymarket. The
/// `SecurityPolicy::enforce_tool_operation(ToolOperation::Act, ...)` call
/// upstream is the framework-level autonomy gate that protects against
/// hallucinated approvals in autonomy-restricted modes; this function
/// covers the friendly-flow approval. **Both must remain in place until
/// #1339 replaces this with a real out-of-band approval channel.**
fn require_write_approval(approved: Option<bool>) -> Result<()> {
    if approved.unwrap_or(false) {
        return Ok(());
    }

    // TODO(#1339): Replace this stopgap with the shared formal approval gate.
    anyhow::bail!(
        "Polymarket write requires explicit user approval. Re-invoke with arguments.approved = true after confirming with the user."
    )
}

fn validate_evm_address(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let parsed =
        Address::from_str(trimmed).with_context(|| format!("Invalid EVM address '{trimmed}'"))?;
    Ok(format!("{parsed:#x}"))
}

fn same_evm_address(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

/// Floor-rounds size to the CLOB's required precision.
///
/// Order **sizes** must round DOWN (floor): rounding up would cause the
/// signed order to commit the EOA to more collateral than the user
/// intended, which can fail the on-chain transfer or — worse — succeed
/// and over-sell. Floor is the safe direction.
fn round_down(value: f64, decimals: u32) -> f64 {
    let factor = 10_f64.powi(decimals as i32);
    (value * factor).floor() / factor
}

/// Half-up rounds price to the CLOB's required tick.
///
/// Order **prices** use bankers-default (half-up) rounding: rounding
/// down would systematically under-quote the user's bid/ask and skew
/// their fills below market. Asymmetric vs `round_down` for size is
/// intentional — different invariants for the two scalars.
fn round_normal(value: f64, decimals: u32) -> f64 {
    let factor = 10_f64.powi(decimals as i32);
    (value * factor).round() / factor
}

fn to_fixed_units_string(value: f64, decimals: u32) -> Result<String> {
    if !value.is_finite() || value < 0.0 {
        anyhow::bail!("Cannot convert non-finite or negative amount to fixed units")
    }
    let factor = 10_f64.powi(decimals as i32);
    let scaled = (value * factor).round();
    if !scaled.is_finite() || scaled < 0.0 {
        anyhow::bail!("Amount scaling overflow")
    }
    Ok(format!("{scaled:.0}"))
}

fn parse_hex_u256(value: &str) -> Result<ethers_core::types::U256> {
    let trimmed = value.trim();
    let normalized = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    ethers_core::types::U256::from_str_radix(normalized, 16)
        .with_context(|| format!("invalid hex quantity '{value}'"))
}

fn parse_order_nonce(payload: &Value) -> Result<u64> {
    if let Some(parsed) = payload
        .get("nonce")
        .or_else(|| payload.get("data").and_then(|v| v.get("nonce")))
        .and_then(parse_nonce_value)
    {
        return Ok(parsed);
    }

    if let Some(parsed) = parse_nonce_value(payload) {
        return Ok(parsed);
    }

    anyhow::bail!("Polymarket nonce response missing valid 'nonce' field")
}

fn parse_nonce_value(value: &Value) -> Option<u64> {
    match value {
        Value::Number(number) => number.as_u64(),
        Value::String(text) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
#[path = "polymarket_tests.rs"]
mod tests;
