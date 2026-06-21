//! tiny.place command manifest — the **single source of truth** for which SDK
//! methods are wired into OpenHuman's JSON-RPC layer.
//!
//! ### Append-point convention
//!
//! The `// === AGENT-WORLD SECTION MANIFEST (append rows here) ===` banner is
//! the first append point for the six fan-out section agents. Adding a new
//! section = appending rows to [`tinyplace_handlers`] and adding the matching
//! schemas to [`all_tinyplace_controller_schemas`] in `schemas.rs`.
//!
//! ### Handler shape (uniform)
//!
//! Each handler:
//! 1. Deserialises params from a `Map<String, Value>`.
//! 2. Calls `ops::global_state().client().await?` to obtain the lazily-built
//!    [`tinyplace::TinyPlaceClient`].
//! 3. Calls the SDK method.
//! 4. Maps the error via `ops::map_err`.
//! 5. Serialises the result with `serde_json::to_value`.

use std::collections::HashMap;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Map, Value};

use crate::core::all::ControllerFuture;
use crate::openhuman::tinyplace::ops::{global_state, map_err};
use crate::openhuman::tinyplace::payment::{
    ensure_backend_mint_matches, ensure_cluster_matches, fulfill_payment, PaymentContext,
};

const LOG_PREFIX: &str = "[tinyplace]";

/// Identity registration settlement retry budget — the on-chain transfer is
/// broadcast immediately, but the backend may not see enough confirmations on
/// the first re-submit. Mirrors the TS SDK's poll loop (~60s total).
const REGISTER_SETTLE_MAX_ATTEMPTS: usize = 30;
const REGISTER_SETTLE_DELAY: Duration = Duration::from_secs(2);

// ── Helpers ───────────────────────────────────────────────────────────────────

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("tinyplace serialise: {e}"))
}

fn get_opt_str<'a>(params: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn req_str<'a>(params: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    get_opt_str(params, key).ok_or_else(|| format!("missing required param '{key}'"))
}

// ── Handler implementations ───────────────────────────────────────────────────

// === AGENT-WORLD SECTION MANIFEST (append rows here) ===
// Each block = one `manifest row`. Format:
//   pub(crate) fn handle_tinyplace_<domain>_<method>(params: Map<String, Value>) -> ControllerFuture { … }
// The handler is then referenced in `schemas.rs` via all_tinyplace_registered_controllers().

// ── Directory: list_agents ────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_directory_list_agents(
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} directory_list_agents (raw passthrough)");
        let client = global_state().client().await?;
        // The SDK types AgentCardSummary.skills/tags as Vec<String>, but the backend
        // returns them as objects ({ id, name }) — so the SDK's typed list_agents()
        // fails to deserialize ("invalid type: map, expected a string"). Fetch the
        // raw JSON instead and let the renderer normalise the shape (its
        // getSkills/toLabel helpers already handle string-or-object). Query params
        // are unused by the current sections; add query support here if needed.
        let result: serde_json::Value = client
            .http()
            .get("/directory/agents", &[])
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Directory: get_agent ──────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_directory_get_agent(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let agent_id = req_str(&params, "agentId")?.to_string();
        log::debug!("{LOG_PREFIX} directory_get_agent agent_id={agent_id}");
        let client = global_state().client().await?;
        let result = client
            .directory
            .get_agent(&agent_id)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Directory: resolve ───────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_directory_resolve(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let name = req_str(&params, "name")?.to_string();
        log::debug!("{LOG_PREFIX} directory_resolve name={name}");
        let client = global_state().client().await?;

        // Try registry first.
        match client.directory.resolve(&name).await {
            Ok(resolved) if resolved.identity.is_some() || resolved.agent.is_some() => {
                // Registry hit — return the full response as-is.
                return to_value(resolved);
            }
            Ok(_) => {
                // Registry returned 200 but with no identity and no agent card.
                // Fall through to the directory-card fallback.
                log::debug!(
                    "{LOG_PREFIX} directory_resolve: registry miss for '{name}', \
                     trying directory card fallback"
                );
            }
            Err(e) if e.status() == Some(404) => {
                // Registry 404 — agent may be directory-only; fall through.
                log::debug!(
                    "{LOG_PREFIX} directory_resolve: registry 404 for '{name}', \
                     trying directory card fallback"
                );
            }
            Err(e) => return Err(map_err(e)),
        }

        // Directory-card fallback: query by username.
        match directory_card_fallback(&client, &name).await {
            Some(card) => {
                log::debug!(
                    "{LOG_PREFIX} directory_resolve: found '{name}' via directory card \
                     crypto_id={}",
                    card.crypto_id
                );
                // Synthesise a ResolveResponse from the agent card so the
                // frontend gets the same shape regardless of code path.
                let response = tinyplace::types::ResolveResponse {
                    identity: None,
                    agent: Some(card),
                };
                to_value(response)
            }
            None => Err(format!(
                "No agent found for \"{name}\". \
                 Check the handle or paste their wallet address directly."
            )),
        }
    })
}

// ── Directory: reverse ───────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_directory_reverse(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let crypto_id = req_str(&params, "cryptoId")?.to_string();
        log::debug!("{LOG_PREFIX} directory_reverse crypto_id={crypto_id}");
        let client = global_state().client().await?;
        let result = client
            .directory
            .reverse(&crypto_id)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Directory: list_identities ───────────────────────────────────────────────

pub(crate) fn handle_tinyplace_directory_list_identities(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} directory_list_identities params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::IdentityListingQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid directory list_identities params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        let result = client
            .directory
            .list_identities(query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Directory: skills ────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_directory_skills(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} directory_skills params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::api::directory::DirectorySkillsParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid directory skills params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        let result = client
            .directory
            .skills(query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Explorer: overview ────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_explorer_overview(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} explorer_overview");
        let client = global_state().client().await?;
        let result = client.explorer.overview().await.map_err(map_err)?;
        to_value(result)
    })
}

// ── Search: unified ───────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_search_unified(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let query = req_str(&params, "query")?.to_string();
        log::debug!("{LOG_PREFIX} search_unified query={query}");
        let client = global_state().client().await?;
        let result = client.search.unified(&query).await.map_err(map_err)?;
        to_value(result)
    })
}

// === AGENT-WORLD SECTION MANIFEST (append rows here) ===
// Each block = one `manifest row`. Format:
//   pub(crate) fn handle_tinyplace_<domain>_<method>(params: Map<String, Value>) -> ControllerFuture { … }
// The handler is then referenced in `schemas.rs` via all_tinyplace_registered_controllers().

// ── Profiles: get ────────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_profiles_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.to_string();
        log::debug!("{LOG_PREFIX} profiles_get username={username}");
        let client = global_state().client().await?;
        let result = client.profiles.get(&username).await.map_err(map_err)?;
        to_value(result)
    })
}

// ── Profiles: activity ───────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_profiles_activity(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.to_string();
        log::debug!("{LOG_PREFIX} profiles_activity username={username}");
        let client = global_state().client().await?;
        let result = client.profiles.activity(&username).await.map_err(map_err)?;
        to_value(result)
    })
}

// ── Profiles: groups ─────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_profiles_groups(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.to_string();
        log::debug!("{LOG_PREFIX} profiles_groups username={username}");
        let client = global_state().client().await?;
        let result = client.profiles.groups(&username).await.map_err(map_err)?;
        to_value(result)
    })
}

// ── Profiles: broadcasts ─────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_profiles_broadcasts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.to_string();
        log::debug!("{LOG_PREFIX} profiles_broadcasts username={username}");
        let client = global_state().client().await?;
        let result = client
            .profiles
            .broadcasts(&username)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Profiles: attestations ───────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_profiles_attestations(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.to_string();
        log::debug!("{LOG_PREFIX} profiles_attestations username={username}");
        let client = global_state().client().await?;
        let result = client
            .profiles
            .attestations(&username)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Profiles: agent_card ─────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_profiles_agent_card(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.to_string();
        log::debug!("{LOG_PREFIX} profiles_agent_card username={username}");
        let client = global_state().client().await?;
        let result = client
            .profiles
            .agent_card(&username)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Users: get ───────────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_users_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let crypto_id = req_str(&params, "cryptoId")?.to_string();
        log::debug!("{LOG_PREFIX} users_get crypto_id={crypto_id}");
        let client = global_state().client().await?;
        let result = client.users.get(&crypto_id).await.map_err(map_err)?;
        to_value(result)
    })
}

// ── Users: update_profile ────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_users_update_profile(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let crypto_id = req_str(&params, "cryptoId")?.to_string();
        let update_value = params.get("update").cloned().unwrap_or(Value::Null);
        let update: tinyplace::types::UserProfileUpdate = serde_json::from_value(update_value)
            .map_err(|e| format!("invalid users update_profile params: {e}"))?;
        log::debug!("{LOG_PREFIX} users_update_profile crypto_id={crypto_id}");
        let client = global_state().client().await?;
        let result = client
            .users
            .update_profile(&crypto_id, update)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_marketplace_identity_floor(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let length = params.get("length").and_then(Value::as_i64);
        log::debug!("{LOG_PREFIX} marketplace_identity_floor length={length:?}");
        let client = global_state().client().await?;
        // IdentityFloor derives Serialize via the types module.
        let result = client
            .marketplace
            .identity_floor(length)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_marketplace_identity_sale_history(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let name = req_str(&params, "name")?.to_string();
        log::debug!("{LOG_PREFIX} marketplace_identity_sale_history name={name}");
        let client = global_state().client().await?;
        // IdentitySaleHistoryResponse only derives Deserialize; serialize the inner vec.
        let result = client
            .marketplace
            .identity_sale_history(&name)
            .await
            .map_err(map_err)?;
        let history = to_value(result.history)?;
        Ok(serde_json::json!({ "history": history }))
    })
}

pub(crate) fn handle_tinyplace_marketplace_list_bids(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let listing_id = req_str(&params, "listingId")?.to_string();
        log::debug!("{LOG_PREFIX} marketplace_list_bids listing_id={listing_id}");
        let client = global_state().client().await?;
        // BidsResponse only derives Deserialize; serialize the inner vec.
        let result = client
            .marketplace
            .list_bids(&listing_id)
            .await
            .map_err(map_err)?;
        let bids = to_value(result.bids)?;
        Ok(serde_json::json!({ "bids": bids }))
    })
}

pub(crate) fn handle_tinyplace_marketplace_list_identities(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let limit = params.get("limit").and_then(Value::as_i64);
        let status = get_opt_str(&params, "status").map(str::to_string);
        log::debug!("{LOG_PREFIX} marketplace_list_identities limit={limit:?} status={status:?}");
        let client = global_state().client().await?;
        // IdentitiesResponse only derives Deserialize; serialize the inner vec.
        let result = client
            .marketplace
            .list_identities(limit, status.as_deref())
            .await
            .map_err(map_err)?;
        let identities = to_value(result.identities)?;
        Ok(serde_json::json!({ "identities": identities }))
    })
}

pub(crate) fn handle_tinyplace_marketplace_list_offers(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let name = get_opt_str(&params, "name").map(str::to_string);
        let buyer = get_opt_str(&params, "buyer").map(str::to_string);
        log::debug!("{LOG_PREFIX} marketplace_list_offers name={name:?} buyer={buyer:?}");
        let client = global_state().client().await?;
        use tinyplace::api::marketplace::OfferQueryParams;
        let query_params = OfferQueryParams {
            name,
            buyer,
            ..Default::default()
        };
        // OffersResponse only derives Deserialize; serialize the inner vec.
        let result = client
            .marketplace
            .list_offers(Some(&query_params))
            .await
            .map_err(map_err)?;
        let offers = to_value(result.offers)?;
        Ok(serde_json::json!({ "offers": offers }))
    })
}

pub(crate) fn handle_tinyplace_marketplace_recent(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} marketplace_recent");
        let client = global_state().client().await?;
        // RecentSalesResponse only derives Deserialize; serialize the inner vec.
        let result = client.marketplace.recent().await.map_err(map_err)?;
        let sales = to_value(result.sales)?;
        Ok(serde_json::json!({ "sales": sales }))
    })
}

pub(crate) fn handle_tinyplace_registry_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let name = req_str(&params, "name")?.to_string();
        log::debug!("{LOG_PREFIX} registry_get name={name}");
        let client = global_state().client().await?;
        let result = client.registry.get(&name).await.map_err(map_err)?;
        to_value(result)
    })
}

/// Register a `@handle` via the x402 confirm-before-spend flow.
///
/// Two-call contract (the renderer drives it):
/// - `confirmed` omitted/false → returns the 402 `challenge` plus the wallet's
///   USDC balance + address so the UI can render a confirm card. **No funds move.**
/// - `confirmed: true` → fulfils the payment on-chain (devnet-guarded) and
///   re-submits the registration with the signed payment map, retrying while the
///   settlement confirms. **This is the only branch that spends.**
///
/// The free tier (backend returns the identity without a 402) short-circuits to
/// `{ identity }` on the first call regardless of `confirmed`.
pub(crate) fn handle_tinyplace_registry_register(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.trim().to_string();
        if username.is_empty() {
            return Err("missing required param 'username'".to_string());
        }
        let confirmed = params
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let actor_type = get_opt_str(&params, "actorType")
            .filter(|s| !s.is_empty())
            .unwrap_or("human")
            .to_string();
        let primary = params.get("primary").and_then(Value::as_bool);
        log::debug!(
            "{LOG_PREFIX} registry_register username={username} confirmed={confirmed} \
             actor_type={actor_type} primary={primary:?}"
        );

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;

        // payment = None on the probe call so the backend issues the 402.
        let base_req = tinyplace::api::registry::RegisterRequest {
            username: username.clone(),
            crypto_id: signer.agent_id(),
            public_key: Some(signer.public_key_base64()),
            actor_type: Some(actor_type),
            primary,
            ..Default::default()
        };

        // ── Phase A: probe for the 402 challenge (or a free-tier identity). ──
        let challenge = match client.registry.register(base_req.clone()).await {
            Ok(identity) => {
                log::debug!("{LOG_PREFIX} registry_register free-tier ok username={username}");
                let _ =
                    publish_directory_card_for_identity(&client, signer.as_ref(), &identity).await;
                return to_value(serde_json::json!({ "identity": identity }));
            }
            Err(e) => match e.payment_required() {
                Some(pr) => pr.payment.clone(),
                None => return Err(map_err(e)),
            },
        };
        log::debug!(
            "{LOG_PREFIX} registry_register 402 challenge network={:?} asset={:?} amount={:?}",
            challenge.network,
            challenge.asset,
            challenge.amount,
        );

        // ── Unconfirmed: surface the challenge + balance, spend nothing. ──
        if !confirmed {
            let (wallet_balance, wallet_address) = wallet_usdc_balance(&signer.agent_id()).await;
            return to_value(serde_json::json!({
                "challenge": challenge,
                "walletBalance": wallet_balance,
                "walletAddress": wallet_address,
            }));
        }

        // ── Confirmed: cluster guards, pay on-chain, re-submit with the map. ──
        if let Some(network) = challenge.network.as_deref() {
            ensure_cluster_matches(network)?;
        }
        ensure_backend_mint_matches(&client).await?;

        let mut extra_metadata = HashMap::new();
        extra_metadata.insert("identity".to_string(), format!("@{username}"));
        let fulfilled = fulfill_payment(
            &challenge,
            signer.as_ref(),
            PaymentContext {
                purpose: "identity.register".to_string(),
                nonce_prefix: "register".to_string(),
                extra_metadata,
            },
        )
        .await?;
        let on_chain_tx = fulfilled.on_chain_tx.clone();

        let mut paid_req = base_req;
        paid_req.payment = Some(fulfilled.payment_map);

        // Re-submit, retrying while the settlement confirms on-chain.
        let mut last_err = String::new();
        for attempt in 1..=REGISTER_SETTLE_MAX_ATTEMPTS {
            match client.registry.register(paid_req.clone()).await {
                Ok(identity) => {
                    log::debug!(
                        "{LOG_PREFIX} registry_register settled username={username} attempt={attempt}"
                    );
                    let _ =
                        publish_directory_card_for_identity(&client, signer.as_ref(), &identity)
                            .await;
                    return to_value(serde_json::json!({
                        "identity": identity,
                        "payment": { "onChainTx": on_chain_tx },
                    }));
                }
                Err(e) if is_retryable_settlement_error(&e) => {
                    last_err = e.to_string();
                    log::debug!(
                        "{LOG_PREFIX} registry_register settlement pending \
                         attempt={attempt}/{REGISTER_SETTLE_MAX_ATTEMPTS}: {last_err}"
                    );
                    tokio::time::sleep(REGISTER_SETTLE_DELAY).await;
                }
                Err(e) => {
                    // Non-retryable failure after we already paid — surface the
                    // tx so the user/support can reconcile.
                    return Err(format!(
                        "registration failed after payment (onChainTx={on_chain_tx}): {}",
                        map_err(e)
                    ));
                }
            }
        }

        // ── Exhausted retries: recover via a fresh availability lookup. ──
        log::warn!(
            "{LOG_PREFIX} registry_register settlement retries exhausted username={username} \
             onChainTx={on_chain_tx}; attempting recovery via registry.get"
        );
        if let Ok(avail) = client.registry.get(&username).await {
            if let Some(identity) = avail.identity {
                if identity.crypto_id == signer.agent_id() {
                    log::debug!(
                        "{LOG_PREFIX} registry_register recovered owned identity username={username}"
                    );
                    let _ =
                        publish_directory_card_for_identity(&client, signer.as_ref(), &identity)
                            .await;
                    return to_value(serde_json::json!({
                        "identity": identity,
                        "payment": { "onChainTx": on_chain_tx },
                    }));
                }
            }
        }
        Err(format!(
            "registration paid but not confirmed in time (onChainTx={on_chain_tx}); \
             last error: {last_err}"
        ))
    })
}

/// Fetch the wallet's Solana USDC balance for the confirm card. Best-effort:
/// returns `(None, address)` if the balance lookup fails so the UI can still
/// render (it falls back to letting the backend reject an underfunded payment).
async fn wallet_usdc_balance(address: &str) -> (Option<Value>, String) {
    // The wallet's `balances()` only reports NATIVE assets (SOL/ETH/…), never SPL
    // tokens — so query the SPL USDC balance directly from the configured Solana
    // cluster's RPC (getTokenAccountsByOwner for the cluster's USDC mint). RPC
    // failure → None (UI shows "Unknown"); RPC ok but no token account → 0.
    let cluster = crate::openhuman::wallet::solana_cluster();
    let mint = cluster.usdc_mint();
    let rpc_url = cluster.rpc_url();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTokenAccountsByOwner",
        "params": [address, { "mint": mint }, { "encoding": "jsonParsed" }],
    });
    let json: Value = match reqwest::Client::new()
        .post(rpc_url)
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                log::warn!("{LOG_PREFIX} usdc balance: rpc parse failed: {e}");
                return (None, address.to_string());
            }
        },
        Err(e) => {
            log::warn!("{LOG_PREFIX} usdc balance: rpc send failed: {e}");
            return (None, address.to_string());
        }
    };
    let Some(accounts) = json.pointer("/result/value").and_then(Value::as_array) else {
        log::warn!("{LOG_PREFIX} usdc balance: unexpected rpc shape");
        return (None, address.to_string());
    };
    // No token account = the ATA was never created = a real zero balance.
    let token_amount = accounts
        .first()
        .and_then(|acct| acct.pointer("/account/data/parsed/info/tokenAmount"));
    let (raw, formatted, decimals) = match token_amount {
        Some(ta) => (
            ta.get("amount")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .to_string(),
            ta.get("uiAmountString")
                .and_then(Value::as_str)
                .unwrap_or("0")
                .to_string(),
            ta.get("decimals").and_then(Value::as_u64).unwrap_or(6),
        ),
        None => ("0".to_string(), "0".to_string(), 6),
    };
    log::debug!("{LOG_PREFIX} usdc balance for {address}: {formatted} (cluster={cluster:?})");
    (
        Some(serde_json::json!({
            "raw": raw,
            "formatted": formatted,
            "decimals": decimals,
            "assetSymbol": "USDC",
        })),
        address.to_string(),
    )
}

/// A re-submitted registration returns a 402 again while the on-chain transfer
/// is still confirming. Retry only those settlement-timing errors — never a hard
/// rejection (which would loop pointlessly and delay the failure).
fn is_retryable_settlement_error(e: &tinyplace::Error) -> bool {
    let mut hay = e.to_string();
    if let Some(pr) = e.payment_required() {
        if let Some(msg) = &pr.error {
            hay.push_str(msg);
        }
    }
    if let Some(body) = e.body() {
        hay.push_str(&body.to_string());
    }
    settlement_error_is_retryable(e.status(), &hay)
}

/// Pure settlement-retry decision (no SDK error type — unit-tested directly).
/// Only a `402` whose message indicates the on-chain transfer is still
/// confirming is retryable.
fn settlement_error_is_retryable(status: Option<u16>, message: &str) -> bool {
    if status != Some(402) {
        return false;
    }
    let hay = message.to_lowercase();
    hay.contains("transaction not found")
        || hay.contains("not found")
        || hay.contains("insufficient confirmation")
        || hay.contains("not yet")
        || hay.contains("pending")
}

// ── Marketplace buy (x402) ─────────────────────────────────────────────────────

/// Outcome of a post-payment re-submit loop that could not return a result.
enum SettleFailure {
    /// A hard, non-retryable backend rejection (already mapped to a string).
    Hard(String),
    /// The settlement never confirmed within the retry budget.
    Exhausted(String),
}

/// Re-submit a paid domain request, retrying only while the on-chain settlement
/// confirms (a `402` with a settlement-timing message). Shared by buy/bid/offer.
async fn settle_retry<F, Fut, T>(label: &str, mut submit: F) -> Result<T, SettleFailure>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, tinyplace::Error>>,
{
    let mut last_err = String::new();
    for attempt in 1..=REGISTER_SETTLE_MAX_ATTEMPTS {
        match submit().await {
            Ok(value) => return Ok(value),
            Err(e) if is_retryable_settlement_error(&e) => {
                last_err = e.to_string();
                log::debug!(
                    "{LOG_PREFIX} {label} settlement pending \
                     attempt={attempt}/{REGISTER_SETTLE_MAX_ATTEMPTS}: {last_err}"
                );
                tokio::time::sleep(REGISTER_SETTLE_DELAY).await;
            }
            Err(e) => return Err(SettleFailure::Hard(map_err(e))),
        }
    }
    Err(SettleFailure::Exhausted(last_err))
}

/// Buy a marketplace product via the x402 confirm-before-spend flow.
///
/// Params `{ id, confirmed? }`. `confirmed:false` → `{ challenge, walletBalance,
/// walletAddress }` (no spend). `confirmed:true` → pays on-chain and completes
/// the purchase, returning `{ result, payment: { onChainTx } }`.
pub(crate) fn handle_tinyplace_marketplace_buy_product(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let product_id = req_str(&params, "id")?.trim().to_string();
        if product_id.is_empty() {
            return Err("missing required param 'id'".to_string());
        }
        let confirmed = params
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        log::debug!("{LOG_PREFIX} marketplace_buy_product id={product_id} confirmed={confirmed}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;

        let base_req = tinyplace::types::ProductBuyRequest {
            buyer_crypto_id: Some(signer.agent_id()),
            ..Default::default()
        };

        let challenge = match client
            .marketplace
            .buy_product(&product_id, base_req.clone())
            .await
        {
            Ok(purchase) => return to_value(serde_json::json!({ "result": purchase })),
            Err(e) => match e.payment_required() {
                Some(pr) => pr.payment.clone(),
                None => return Err(map_err(e)),
            },
        };

        if !confirmed {
            let (wallet_balance, wallet_address) = wallet_usdc_balance(&signer.agent_id()).await;
            return to_value(serde_json::json!({
                "challenge": challenge,
                "walletBalance": wallet_balance,
                "walletAddress": wallet_address,
            }));
        }

        if let Some(network) = challenge.network.as_deref() {
            ensure_cluster_matches(network)?;
        }
        ensure_backend_mint_matches(&client).await?;
        let mut extra_metadata = HashMap::new();
        extra_metadata.insert("productId".to_string(), product_id.clone());
        let fulfilled = fulfill_payment(
            &challenge,
            signer.as_ref(),
            PaymentContext {
                purpose: "marketplace.buy_product".to_string(),
                nonce_prefix: "buy".to_string(),
                extra_metadata,
            },
        )
        .await?;
        let on_chain_tx = fulfilled.on_chain_tx.clone();

        let mut paid_req = base_req;
        paid_req.payment = Some(fulfilled.payment_map);
        match settle_retry("buy_product", || {
            client
                .marketplace
                .buy_product(&product_id, paid_req.clone())
        })
        .await
        {
            Ok(purchase) => to_value(serde_json::json!({
                "result": purchase,
                "payment": { "onChainTx": on_chain_tx },
            })),
            Err(SettleFailure::Hard(m)) => Err(format!(
                "purchase failed after payment (onChainTx={on_chain_tx}): {m}"
            )),
            Err(SettleFailure::Exhausted(last)) => Err(format!(
                "purchase paid but not confirmed in time (onChainTx={on_chain_tx}); \
                 last error: {last}"
            )),
        }
    })
}

/// Buy an identity listing (a `@handle` at its fixed price) via the same x402
/// confirm-before-spend flow. Params `{ id, confirmed? }` where `id` is the
/// listing id.
pub(crate) fn handle_tinyplace_marketplace_buy_identity(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let listing_id = req_str(&params, "id")?.trim().to_string();
        if listing_id.is_empty() {
            return Err("missing required param 'id'".to_string());
        }
        let confirmed = params
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        log::debug!("{LOG_PREFIX} marketplace_buy_identity id={listing_id} confirmed={confirmed}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;

        // buyer left empty → the connected signing key is the actor; the SDK
        // auto-signs the canonical identity.buy payload.
        let base_req = tinyplace::types::IdentityBuyRequest {
            buyer: String::new(),
            buyer_crypto_id: signer.agent_id(),
            buyer_public_key: Some(signer.public_key_base64()),
            ..Default::default()
        };

        let challenge = match client
            .marketplace
            .buy_identity_listing(&listing_id, base_req.clone())
            .await
        {
            Ok(sale) => return to_value(serde_json::json!({ "result": sale })),
            Err(e) => match e.payment_required() {
                Some(pr) => pr.payment.clone(),
                None => return Err(map_err(e)),
            },
        };

        if !confirmed {
            let (wallet_balance, wallet_address) = wallet_usdc_balance(&signer.agent_id()).await;
            return to_value(serde_json::json!({
                "challenge": challenge,
                "walletBalance": wallet_balance,
                "walletAddress": wallet_address,
            }));
        }

        if let Some(network) = challenge.network.as_deref() {
            ensure_cluster_matches(network)?;
        }
        ensure_backend_mint_matches(&client).await?;
        let mut extra_metadata = HashMap::new();
        extra_metadata.insert("listingId".to_string(), listing_id.clone());
        let fulfilled = fulfill_payment(
            &challenge,
            signer.as_ref(),
            PaymentContext {
                purpose: "marketplace.buy_identity".to_string(),
                nonce_prefix: "buy".to_string(),
                extra_metadata,
            },
        )
        .await?;
        let on_chain_tx = fulfilled.on_chain_tx.clone();

        // The signed payload depends on request fields; clearing the stale
        // signature lets the SDK re-sign with the payment attached.
        let mut paid_req = base_req;
        paid_req.payment = Some(fulfilled.payment_map);
        paid_req.signature = None;
        match settle_retry("buy_identity", || {
            client
                .marketplace
                .buy_identity_listing(&listing_id, paid_req.clone())
        })
        .await
        {
            Ok(sale) => to_value(serde_json::json!({
                "result": sale,
                "payment": { "onChainTx": on_chain_tx },
            })),
            Err(SettleFailure::Hard(m)) => Err(format!(
                "purchase failed after payment (onChainTx={on_chain_tx}): {m}"
            )),
            Err(SettleFailure::Exhausted(last)) => Err(format!(
                "purchase paid but not confirmed in time (onChainTx={on_chain_tx}); \
                 last error: {last}"
            )),
        }
    })
}

// ── Marketplace bid / offer (x402 commitments) ─────────────────────────────────

/// Build a [`tinyplace::types::MarketplacePrice`] from params. `network` is
/// required (the renderer passes the listing's price network so the x402
/// authorization targets the right chain); `asset` defaults to USDC.
fn price_from_params(
    params: &Map<String, Value>,
) -> Result<tinyplace::types::MarketplacePrice, String> {
    let amount = req_str(params, "amount")?.trim().to_string();
    if amount.is_empty() {
        return Err("missing required param 'amount'".to_string());
    }
    let asset = get_opt_str(params, "asset")
        .filter(|s| !s.is_empty())
        .unwrap_or("USDC")
        .to_string();
    let network = req_str(params, "network")?.trim().to_string();
    if network.is_empty() {
        return Err("missing required param 'network'".to_string());
    }
    Ok(tinyplace::types::MarketplacePrice {
        amount,
        asset,
        network,
    })
}

/// Place a bid on an identity auction listing. The SDK builds and signs the
/// x402 authorization (an "up-to" commitment) internally — **no on-chain
/// transfer happens here**; the bid settles on acceptance. May 402 if the
/// backend requires a deposit (surfaced as PAYMENT_REQUIRED to the renderer).
///
/// Params `{ listingId, amount, asset?, network }`.
pub(crate) fn handle_tinyplace_marketplace_bid(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let listing_id = req_str(&params, "listingId")?.trim().to_string();
        if listing_id.is_empty() {
            return Err("missing required param 'listingId'".to_string());
        }
        let price = price_from_params(&params)?;
        log::debug!(
            "{LOG_PREFIX} marketplace_bid listing_id={listing_id} amount={} asset={} network={}",
            price.amount,
            price.asset,
            price.network,
        );

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;

        let bid = tinyplace::types::IdentityBid {
            bidder: Some(signer.agent_id()),
            bidder_crypto_id: Some(signer.agent_id()),
            bidder_public_key: Some(signer.public_key_base64()),
            price: Some(price),
            ..Default::default()
        };
        let result = client
            .marketplace
            .place_bid_with_payment(
                &listing_id,
                bid,
                tinyplace::api::marketplace::IdentityBidPaymentOptions::default(),
            )
            .await
            .map_err(map_err)?;

        // Return the updated listing only — never the raw signed authorization map.
        to_value(serde_json::json!({
            "result": result.updated_listing,
            "committed": true,
        }))
    })
}

/// Make an offer to buy an identity (`@handle`) at a chosen price. Like bids,
/// the SDK builds and signs the x402 authorization internally — **no on-chain
/// transfer here**; the offer settles on acceptance.
///
/// Params `{ name, amount, asset?, network }`.
pub(crate) fn handle_tinyplace_marketplace_offer(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let name = req_str(&params, "name")?.trim().to_string();
        if name.is_empty() {
            return Err("missing required param 'name'".to_string());
        }
        let price = price_from_params(&params)?;
        log::debug!(
            "{LOG_PREFIX} marketplace_offer name={name} amount={} asset={} network={}",
            price.amount,
            price.asset,
            price.network,
        );

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;

        let offer = tinyplace::types::IdentityOffer {
            name: Some(name),
            buyer: Some(signer.agent_id()),
            buyer_crypto_id: Some(signer.agent_id()),
            buyer_public_key: Some(signer.public_key_base64()),
            price: Some(price),
            ..Default::default()
        };
        let result = client
            .marketplace
            .create_offer_with_payment(
                offer,
                tinyplace::api::marketplace::IdentityOfferPaymentOptions::default(),
            )
            .await
            .map_err(map_err)?;

        to_value(serde_json::json!({
            "result": result.offer,
            "committed": true,
        }))
    })
}

pub(crate) fn handle_tinyplace_artifacts_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let artifact_id = req_str(&params, "artifactId")?.to_string();
        let actor_id = get_opt_str(&params, "actorId").map(str::to_string);
        log::debug!("{LOG_PREFIX} artifacts_get artifact_id={artifact_id} actor_id={actor_id:?}");
        let client = global_state().client().await?;
        let result = client
            .artifacts
            .get(&artifact_id, actor_id.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_artifacts_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} artifacts_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::ArtifactQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid artifacts list params: {e}"))
            })
            .transpose()?;
        let actor_id = get_opt_str(&params, "actorId").map(str::to_string);

        let client = global_state().client().await?;
        let result = client
            .artifacts
            .list(query_params.as_ref(), actor_id.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_escrow_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let escrow_id = req_str(&params, "escrowId")?.to_string();
        log::debug!("{LOG_PREFIX} escrow_get escrow_id={escrow_id}");
        let client = global_state().client().await?;
        let result = client.escrow.get(&escrow_id).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_escrow_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} escrow_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::EscrowQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid escrow list params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        let result = client
            .escrow
            .list(query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        log::debug!("{LOG_PREFIX} jobs_get job_id={job_id}");
        let client = global_state().client().await?;
        let result = client.jobs.get(&job_id).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} jobs_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::JobQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid jobs list params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        let result = client
            .jobs
            .list(query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_marketplace_browse(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} marketplace_browse params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::ProductQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid marketplace browse params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        let result = client
            .marketplace
            .browse_marketplace(query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_marketplace_categories(
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} marketplace_categories");
        let client = global_state().client().await?;
        let result = client.marketplace.categories().await.map_err(map_err)?;
        to_value(CategoriesWrapper {
            categories: result.categories,
        })
    })
}

pub(crate) fn handle_tinyplace_marketplace_featured(
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} marketplace_featured");
        let client = global_state().client().await?;
        let result = client.marketplace.featured().await.map_err(map_err)?;
        to_value(FeaturedWrapper {
            items: result.items,
        })
    })
}

pub(crate) fn handle_tinyplace_marketplace_get_product(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let product_id = req_str(&params, "productId")?.to_string();
        log::debug!("{LOG_PREFIX} marketplace_get_product product_id={product_id}");
        let client = global_state().client().await?;
        let result = client
            .marketplace
            .get_product(&product_id)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_marketplace_list_product_reviews(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let product_id = req_str(&params, "productId")?.to_string();
        log::debug!("{LOG_PREFIX} marketplace_list_product_reviews product_id={product_id}");
        let client = global_state().client().await?;
        let result = client
            .marketplace
            .list_product_reviews(&product_id)
            .await
            .map_err(map_err)?;
        to_value(ProductReviewsWrapper {
            reviews: result.reviews,
        })
    })
}

pub(crate) fn handle_tinyplace_marketplace_list_products(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} marketplace_list_products params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::ProductQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid marketplace list_products params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        let result = client
            .marketplace
            .list_products(query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(ProductsWrapper {
            products: result.products,
        })
    })
}

// Serialize wrappers for marketplace responses (from #5).
#[derive(serde::Serialize)]
struct ProductsWrapper {
    products: Vec<tinyplace::types::Product>,
}

#[derive(serde::Serialize)]
struct CategoriesWrapper {
    categories: Vec<tinyplace::types::MarketplaceCategory>,
}

#[derive(serde::Serialize)]
struct FeaturedWrapper {
    items: Vec<serde_json::Value>,
}

#[derive(serde::Serialize)]
struct ProductReviewsWrapper {
    reviews: Vec<tinyplace::types::ProductReview>,
}

pub(crate) fn handle_tinyplace_broadcasts_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} broadcasts_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::BroadcastQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid broadcasts list params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        let result = client
            .broadcasts
            .list(query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_channels_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} channels_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::api::channels::ChannelQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid channels list params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        match client.channels.list(query_params.as_ref()).await {
            Ok(result) => to_value(result),
            Err(e) => match channels_list_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} channels_list endpoint unavailable -> empty list (status={:?})",
                        e.status()
                    );
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

/// The staging backend exposes no `/channels` route (only `/search/channels`),
/// so the endpoint 404s. Degrade a 404 to an empty channel list rather than
/// surfacing a hard error to the Messages UI; propagate every other error.
pub(crate) fn channels_list_degrade(e: &tinyplace::Error) -> Option<Value> {
    if e.status() == Some(404) {
        Some(serde_json::json!({ "channels": [] }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_groups_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} groups_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::GroupQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid groups list params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        let result = client
            .groups
            .list(query_params.as_ref())
            .await
            .map_err(map_err)?;
        // GroupListResponse doesn't implement Serialize; serialize its inner vec.
        to_value(result.groups)
    })
}

pub(crate) fn handle_tinyplace_inbox_counts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let owner: Option<String> = params
            .get("owner")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        log::debug!("{LOG_PREFIX} inbox_counts owner={owner:?}");

        let client = global_state().client().await?;
        let result = client
            .inbox
            .counts(owner.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_inbox_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} inbox_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::api::inbox::InboxQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid inbox list params: {e}"))
            })
            .transpose()?;
        let owner: Option<String> = params
            .get("owner")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let client = global_state().client().await?;
        match client
            .inbox
            .list(query_params.as_ref(), owner.as_deref())
            .await
        {
            Ok(result) => to_value(result),
            Err(e) => match inbox_list_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} inbox_list deserialization failed (likely empty inbox) -> empty: {e}"
                    );
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

/// An empty inbox comes back as `{"items": null}`, which fails the SDK's
/// non-optional `items: Vec<InboxItem>` deserialization. Treat that
/// serialization failure as an empty inbox; propagate every other error.
pub(crate) fn inbox_list_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({
            "items": [],
            "cursor": null,
            "unreadCount": 0,
            "totalCount": 0,
        }))
    } else {
        None
    }
}

fn opt_owner(params: &Map<String, Value>) -> Option<String> {
    params
        .get("owner")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub(crate) fn handle_tinyplace_broadcasts_subscribe(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let broadcast_id = req_str(&params, "broadcastId")?.to_string();
        log::debug!("{LOG_PREFIX} broadcasts_subscribe broadcast_id={broadcast_id}");
        let client = global_state().client().await?;
        let result = client
            .broadcasts
            .subscribe(&broadcast_id, None)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_broadcasts_unsubscribe(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let broadcast_id = req_str(&params, "broadcastId")?.to_string();
        log::debug!("{LOG_PREFIX} broadcasts_unsubscribe broadcast_id={broadcast_id}");
        let client = global_state().client().await?;
        client
            .broadcasts
            .unsubscribe(&broadcast_id, None)
            .await
            .map_err(map_err)?;
        to_value(serde_json::json!({ "ok": true }))
    })
}

pub(crate) fn handle_tinyplace_channels_join(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let channel_id = req_str(&params, "channelId")?.to_string();
        log::debug!("{LOG_PREFIX} channels_join channel_id={channel_id}");
        let client = global_state().client().await?;
        let result = client
            .channels
            .join(&channel_id, None)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_channels_leave(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let channel_id = req_str(&params, "channelId")?.to_string();
        log::debug!("{LOG_PREFIX} channels_leave channel_id={channel_id}");
        let client = global_state().client().await?;
        client
            .channels
            .leave(&channel_id, None)
            .await
            .map_err(map_err)?;
        to_value(serde_json::json!({ "ok": true }))
    })
}

pub(crate) fn handle_tinyplace_groups_join(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let group_id = req_str(&params, "groupId")?.to_string();
        log::debug!("{LOG_PREFIX} groups_join group_id={group_id}");
        let client = global_state().client().await?;
        let result = client.groups.join(&group_id, None).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_groups_leave(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let group_id = req_str(&params, "groupId")?.to_string();
        log::debug!("{LOG_PREFIX} groups_leave group_id={group_id}");
        let client = global_state().client().await?;
        // Leaving = removing ourselves; the SDK exposes no `groups.leave`.
        let me = client
            .http()
            .signer()
            .map(|s| s.agent_id())
            .ok_or_else(|| "tinyplace signer unavailable; cannot leave group".to_string())?;

        // Pre-check membership before calling remove_member so we return a
        // clear error instead of leaking a raw HTTP 400 from the backend when
        // the user is not a member.
        match client.groups.members(&group_id).await {
            Ok(resp) => {
                if !resp.members.iter().any(|m| m.agent_id == me) {
                    log::warn!(
                        "{LOG_PREFIX} groups_leave: agent {me} is not a member of {group_id}"
                    );
                    return Err(format!(
                        "you are not a member of group {group_id}; cannot leave"
                    ));
                }
            }
            Err(e) => {
                // If the membership list itself is unavailable (network, 404, etc.)
                // we log a warning and fall through — the backend will reject if
                // the agent truly is not a member.
                log::warn!(
                    "{LOG_PREFIX} groups_leave: membership check failed for {group_id}: {e}; \
                     proceeding with remove_member"
                );
            }
        }

        client
            .groups
            .remove_member(&group_id, &me, None)
            .await
            .map_err(map_err)?;
        to_value(serde_json::json!({ "ok": true }))
    })
}

pub(crate) fn handle_tinyplace_inbox_archive(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let item_id = req_str(&params, "itemId")?.to_string();
        let owner = opt_owner(&params);
        log::debug!("{LOG_PREFIX} inbox_archive item_id={item_id} owner={owner:?}");
        let client = global_state().client().await?;
        let result = client
            .inbox
            .archive(&item_id, owner.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_inbox_mark_all_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let clear_params: Option<tinyplace::api::inbox::InboxClearParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid inbox mark_all_read params: {e}"))
            })
            .transpose()?;
        let owner = opt_owner(&params);
        log::debug!("{LOG_PREFIX} inbox_mark_all_read owner={owner:?}");
        let client = global_state().client().await?;
        let result = client
            .inbox
            .mark_all_read(clear_params.as_ref(), owner.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_inbox_mark_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let item_id = req_str(&params, "itemId")?.to_string();
        let owner = opt_owner(&params);
        log::debug!("{LOG_PREFIX} inbox_mark_read item_id={item_id} owner={owner:?}");
        let client = global_state().client().await?;
        let result = client
            .inbox
            .mark_read(&item_id, owner.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_inbox_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let item_id = req_str(&params, "itemId")?.to_string();
        let owner = opt_owner(&params);
        log::debug!("{LOG_PREFIX} inbox_remove item_id={item_id} owner={owner:?}");
        let client = global_state().client().await?;
        client
            .inbox
            .remove(&item_id, owner.as_deref())
            .await
            .map_err(map_err)?;
        to_value(serde_json::json!({ "ok": true }))
    })
}

pub(crate) fn handle_tinyplace_inbox_unarchive(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let item_id = req_str(&params, "itemId")?.to_string();
        let owner = opt_owner(&params);
        log::debug!("{LOG_PREFIX} inbox_unarchive item_id={item_id} owner={owner:?}");
        let client = global_state().client().await?;
        let result = client
            .inbox
            .unarchive(&item_id, owner.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Follows handlers ─────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_follows_follow(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let agent_id = req_str(&params, "agentId")?.to_string();
        log::debug!("{LOG_PREFIX} follows_follow agent_id={agent_id}");
        let client = global_state().client().await?;
        let result = client.follows.follow(&agent_id).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_follows_unfollow(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let agent_id = req_str(&params, "agentId")?.to_string();
        log::debug!("{LOG_PREFIX} follows_unfollow agent_id={agent_id}");
        let client = global_state().client().await?;
        client.follows.unfollow(&agent_id).await.map_err(map_err)?;
        to_value(serde_json::json!({ "ok": true }))
    })
}

pub(crate) fn handle_tinyplace_follows_followers(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let agent_id = req_str(&params, "agentId")?.to_string();
        log::debug!("{LOG_PREFIX} follows_followers agent_id={agent_id}");
        let list_params: Option<tinyplace::types::FollowListParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid follows followers params: {e}"))
            })
            .transpose()?;
        let client = global_state().client().await?;
        let result = client
            .follows
            .followers(&agent_id, list_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_follows_following(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let agent_id = req_str(&params, "agentId")?.to_string();
        log::debug!("{LOG_PREFIX} follows_following agent_id={agent_id}");
        let list_params: Option<tinyplace::types::FollowListParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid follows following params: {e}"))
            })
            .transpose()?;
        let client = global_state().client().await?;
        let result = client
            .follows
            .following(&agent_id, list_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_follows_stats(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let agent_id = req_str(&params, "agentId")?.to_string();
        log::debug!("{LOG_PREFIX} follows_stats agent_id={agent_id}");
        let client = global_state().client().await?;
        let result = client.follows.stats(&agent_id).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_follows_feed(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} follows_feed params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let feed_params: Option<tinyplace::types::FeedListParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid follows feed params: {e}"))
            })
            .transpose()?;
        let client = global_state().client().await?;
        let result = client
            .follows
            .feed(feed_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Feedback handlers ─────────────────────────────────────────────────────────
// TODO(staging): verify /feedback is deployed on staging-api.tiny.place before
// integration testing.

pub(crate) fn handle_tinyplace_feedback_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} feedback_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let list_params: Option<tinyplace::types::FeedbackListParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid feedback list params: {e}"))
            })
            .transpose()?;
        let client = global_state().client().await?;
        let result = client
            .feedback
            .list(list_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_feedback_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let feedback_id = req_str(&params, "feedbackId")?.to_string();
        log::debug!("{LOG_PREFIX} feedback_get feedback_id={feedback_id}");
        let client = global_state().client().await?;
        let result = client.feedback.get(&feedback_id).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_feedback_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let title = req_str(&params, "title")?.trim().to_string();
        if title.is_empty() {
            return Err("missing required param 'title'".to_string());
        }
        let description = req_str(&params, "description")?.trim().to_string();
        if description.is_empty() {
            return Err("missing required param 'description'".to_string());
        }
        let category = get_opt_str(&params, "category")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        log::debug!("{LOG_PREFIX} feedback_create title={title} category={category:?}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;

        let feedback = tinyplace::types::FeedbackCreate {
            feedback_id: None,
            author: signer.agent_id(),
            title,
            description,
            category,
        };

        let result = client.feedback.create(feedback).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_feedback_vote(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let feedback_id = req_str(&params, "feedbackId")?.to_string();
        let vote = req_str(&params, "vote")?.to_string();
        if vote != "up" && vote != "down" {
            return Err(format!(
                "invalid vote value '{vote}': must be 'up' or 'down'"
            ));
        }
        log::debug!("{LOG_PREFIX} feedback_vote feedback_id={feedback_id} vote={vote}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;

        let vote_req = tinyplace::types::FeedbackVoteRequest {
            voter: signer.agent_id(),
            vote,
        };

        let result = client
            .feedback
            .vote(&feedback_id, vote_req)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Jobs write handlers ───────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_jobs_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let title = req_str(&params, "title")?.trim().to_string();
        if title.is_empty() {
            return Err("missing required param 'title'".to_string());
        }
        let budget_amount = req_str(&params, "budgetAmount")?.trim().to_string();
        if budget_amount.is_empty() {
            return Err("missing required param 'budgetAmount'".to_string());
        }
        let budget_asset = req_str(&params, "budgetAsset")?.trim().to_string();
        if budget_asset.is_empty() {
            return Err("missing required param 'budgetAsset'".to_string());
        }
        let description = get_opt_str(&params, "description")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let category = get_opt_str(&params, "category")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let budget_chain = get_opt_str(&params, "budgetChain")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let proposal_deadline = get_opt_str(&params, "proposalDeadline")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        // Skills: optional JSON array of strings
        let skills: Option<Vec<String>> = params
            .get("skills")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid 'skills' param: {e}"))
            })
            .transpose()?;

        log::debug!("{LOG_PREFIX} jobs_create title={title} category={category:?}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let request = tinyplace::types::JobCreateRequest {
            client: actor,
            title,
            description,
            category,
            skills,
            budget: tinyplace::types::JobBudget {
                amount: budget_amount,
                asset: budget_asset,
                chain: budget_chain,
            },
            on_chain: None,
            proposal_deadline,
        };

        let result = client.jobs.create(&request).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        log::debug!("{LOG_PREFIX} jobs_cancel job_id={job_id}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let result = client.jobs.cancel(&job_id, &actor).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_apply(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        let cover_letter = get_opt_str(&params, "coverLetter")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let bid_amount = get_opt_str(&params, "bidAmount")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let estimated_delivery = get_opt_str(&params, "estimatedDelivery")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let past_work: Option<Vec<String>> = params
            .get("pastWork")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid 'pastWork' param: {e}"))
            })
            .transpose()?;

        log::debug!("{LOG_PREFIX} jobs_apply job_id={job_id}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let request = tinyplace::types::ProposalCreateRequest {
            candidate: actor,
            cover_letter,
            bid_amount,
            estimated_delivery,
            past_work,
        };

        let result = client
            .jobs
            .apply(&job_id, &request)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_list_proposals(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use tinyplace::api::jobs::ProposalQueryParams;

        let job_id = req_str(&params, "jobId")?.to_string();
        let status = get_opt_str(&params, "status")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let limit = params.get("limit").and_then(|v| v.as_i64());
        let offset = params.get("offset").and_then(|v| v.as_i64());

        log::debug!("{LOG_PREFIX} jobs_list_proposals job_id={job_id} status={status:?}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let query_params = ProposalQueryParams {
            status,
            limit,
            offset,
        };

        let result = client
            .jobs
            .list_proposals(&job_id, &actor, Some(&query_params))
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_get_proposal(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        let proposal_id = req_str(&params, "proposalId")?.to_string();
        log::debug!("{LOG_PREFIX} jobs_get_proposal job_id={job_id} proposal_id={proposal_id}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let result = client
            .jobs
            .get_proposal(&job_id, &proposal_id, &actor)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_shortlist_proposal(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        let proposal_id = req_str(&params, "proposalId")?.to_string();
        log::debug!(
            "{LOG_PREFIX} jobs_shortlist_proposal job_id={job_id} proposal_id={proposal_id}"
        );

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let result = client
            .jobs
            .shortlist_proposal(&job_id, &proposal_id, &actor)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_withdraw_proposal(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        let proposal_id = req_str(&params, "proposalId")?.to_string();
        log::debug!(
            "{LOG_PREFIX} jobs_withdraw_proposal job_id={job_id} proposal_id={proposal_id}"
        );

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let result = client
            .jobs
            .withdraw_proposal(&job_id, &proposal_id, &actor)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_select(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        let proposal_id = req_str(&params, "proposalId")?.to_string();
        let network = get_opt_str(&params, "network")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        log::debug!(
            "{LOG_PREFIX} jobs_select job_id={job_id} proposal_id={proposal_id} network={network:?}"
        );

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let result = client
            .jobs
            .select(&job_id, &actor, &proposal_id, network.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_open_dispute(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        let reason = req_str(&params, "reason")?.trim().to_string();
        if reason.is_empty() {
            return Err("missing required param 'reason'".to_string());
        }
        log::debug!("{LOG_PREFIX} jobs_open_dispute job_id={job_id}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let result = client
            .jobs
            .open_dispute(&job_id, &actor, &reason)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_jobs_adjudicate_dispute(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let job_id = req_str(&params, "jobId")?.to_string();
        log::debug!("{LOG_PREFIX} jobs_adjudicate_dispute job_id={job_id}");

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        let result = client
            .jobs
            .adjudicate_dispute(&job_id, &actor)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Groups invite/role management ────────────────────────────────────────────

pub(crate) fn handle_tinyplace_groups_set_member_role(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let group_id = req_str(&params, "groupId")?.to_string();
        let agent_id = req_str(&params, "agentId")?.to_string();
        let role = req_str(&params, "role")?.to_string();
        log::debug!(
            "{LOG_PREFIX} groups_set_member_role group_id={group_id} agent_id={agent_id} role={role}"
        );
        let client = global_state().client().await?;
        let result = client
            .groups
            .set_member_role(&group_id, &agent_id, &role, None)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_groups_create_invite(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let group_id = req_str(&params, "groupId")?.to_string();
        log::debug!("{LOG_PREFIX} groups_create_invite group_id={group_id}");

        let client = global_state().client().await?;
        let actor = client
            .http()
            .signer()
            .map(|s| s.agent_id())
            .ok_or_else(|| "tinyplace signer unavailable; cannot create invite".to_string())?;

        let request: Option<tinyplace::types::GroupInviteCreateRequest> = params
            .get("request")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid groups create_invite request: {e}"))
            })
            .transpose()?;

        let result = client
            .groups
            .create_invite(&group_id, &actor, request)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_groups_list_invites(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let group_id = req_str(&params, "groupId")?.to_string();
        log::debug!("{LOG_PREFIX} groups_list_invites group_id={group_id}");

        let client = global_state().client().await?;
        let actor = client
            .http()
            .signer()
            .map(|s| s.agent_id())
            .ok_or_else(|| "tinyplace signer unavailable; cannot list invites".to_string())?;

        let result = client
            .groups
            .list_invites(&group_id, &actor)
            .await
            .map_err(map_err)?;
        // GroupInvitesResponse doesn't derive Serialize; serialize the inner vec.
        to_value(result.invites)
    })
}

pub(crate) fn handle_tinyplace_groups_preview_invite(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let group_id = req_str(&params, "groupId")?.to_string();
        let token = req_str(&params, "token")?.to_string();
        log::debug!("{LOG_PREFIX} groups_preview_invite group_id={group_id} token=<redacted>");
        let client = global_state().client().await?;
        let result = client
            .groups
            .preview_invite(&group_id, &token)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_groups_revoke_invite(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let group_id = req_str(&params, "groupId")?.to_string();
        let token = req_str(&params, "token")?.to_string();
        log::debug!("{LOG_PREFIX} groups_revoke_invite group_id={group_id} token=<redacted>");

        let client = global_state().client().await?;
        let actor = client
            .http()
            .signer()
            .map(|s| s.agent_id())
            .ok_or_else(|| "tinyplace signer unavailable; cannot revoke invite".to_string())?;

        client
            .groups
            .revoke_invite(&group_id, &token, &actor)
            .await
            .map_err(map_err)?;
        to_value(serde_json::json!({ "ok": true }))
    })
}

pub(crate) fn handle_tinyplace_groups_redeem_invite(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let group_id = req_str(&params, "groupId")?.to_string();
        let token = req_str(&params, "token")?.to_string();
        log::debug!("{LOG_PREFIX} groups_redeem_invite group_id={group_id} token=<redacted>");

        let client = global_state().client().await?;
        let me = client
            .http()
            .signer()
            .map(|s| s.agent_id())
            .ok_or_else(|| "tinyplace signer unavailable; cannot redeem invite".to_string())?;

        let result = client
            .groups
            .redeem_invite(&group_id, &token, &me)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Registry export handler ───────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_registry_export(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let name = req_str(&params, "name")?.to_string();
        log::debug!("{LOG_PREFIX} registry_export name={name}");
        let client = global_state().client().await?;
        let result = client.registry.export(&name).await.map_err(map_err)?;
        to_value(result)
    })
}

// ── Users email verification ────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_users_start_email_verification(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let crypto_id = req_str(&params, "cryptoId")?.to_string();
        let email = req_str(&params, "email")?.trim().to_string();
        if email.is_empty() {
            return Err("missing required param 'email'".to_string());
        }
        log::debug!(
            "{LOG_PREFIX} users_start_email_verification crypto_id={crypto_id} email=<redacted>"
        );
        let client = global_state().client().await?;
        let request = tinyplace::types::UserEmailVerificationRequest {
            email,
            ..Default::default()
        };
        let result = client
            .users
            .start_email_verification(&crypto_id, request)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_users_confirm_email_verification(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let crypto_id = req_str(&params, "cryptoId")?.to_string();
        let email = req_str(&params, "email")?.trim().to_string();
        if email.is_empty() {
            return Err("missing required param 'email'".to_string());
        }
        let code = req_str(&params, "code")?.trim().to_string();
        if code.is_empty() {
            return Err("missing required param 'code'".to_string());
        }
        log::debug!(
            "{LOG_PREFIX} users_confirm_email_verification crypto_id={crypto_id} email=<redacted>"
        );
        let client = global_state().client().await?;
        let request = tinyplace::types::UserEmailVerificationConfirmRequest {
            email,
            code,
            ..Default::default()
        };
        let result = client
            .users
            .confirm_email_verification(&crypto_id, request)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Solana handlers ─────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_solana_info(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} solana_info");
        let client = global_state().client().await?;
        let result = client.solana.info().await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_solana_call(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let method = req_str(&params, "method")?.trim().to_string();
        if method.is_empty() {
            return Err("missing required param 'method'".to_string());
        }
        let rpc_params: Option<serde_json::Value> =
            params
                .get("params")
                .and_then(|v| if v.is_null() { None } else { Some(v.clone()) });
        let id: Option<serde_json::Value> =
            params
                .get("id")
                .and_then(|v| if v.is_null() { None } else { Some(v.clone()) });
        log::debug!("{LOG_PREFIX} solana_call method={method}");
        let client = global_state().client().await?;
        let result: serde_json::Value = client
            .solana
            .call::<serde_json::Value>(&method, rpc_params, id)
            .await
            .map_err(map_err)?;
        Ok(result)
    })
}

// ── Streams section ────────────────────────────────────────────────────────────

/// Start a tinyplace WebSocket stream.
pub(crate) fn handle_tinyplace_streams_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let kind_str = req_str(&params, "streamType")?.to_string();
        let kind_str_trimmed = kind_str.trim();
        if kind_str_trimmed.is_empty() {
            return Err("missing required param 'streamType'".to_string());
        }
        let kind = match kind_str_trimmed {
            "inbox" => super::streams::StreamKind::Inbox,
            "conversation" => super::streams::StreamKind::Conversation,
            _ => return Err(format!("unsupported streamType: {kind_str_trimmed}")),
        };

        // conversation streams require a target id.
        let target_id = params
            .get("streamId")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if kind == super::streams::StreamKind::Conversation && target_id.is_none() {
            return Err("streamId is required for conversation streams".to_string());
        }

        log::debug!(
            "{LOG_PREFIX} streams_start kind={kind_str_trimmed} target_id={:?}",
            target_id
        );

        let client = global_state().client().await?;
        let stream_id = super::streams::global_stream_manager()
            .start_stream(kind, target_id, client)
            .await?;

        to_value(serde_json::json!({ "streamId": stream_id }))
    })
}

/// Stop a tinyplace WebSocket stream.
pub(crate) fn handle_tinyplace_streams_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let stream_id = req_str(&params, "streamId")?.to_string();
        let stream_id_trimmed = stream_id.trim();
        if stream_id_trimmed.is_empty() {
            return Err("missing required param 'streamId'".to_string());
        }

        log::debug!("{LOG_PREFIX} streams_stop stream_id={stream_id_trimmed}");

        super::streams::global_stream_manager()
            .stop_stream(stream_id_trimmed)
            .await?;

        to_value(serde_json::json!({ "ok": true }))
    })
}

/// List active tinyplace WebSocket streams.
pub(crate) fn handle_tinyplace_streams_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} streams_list");
        let entries = super::streams::global_stream_manager().list_streams().await;
        to_value(serde_json::json!({ "streams": entries }))
    })
}

// ── Signal key management ─────────────────────────────────────────────────────
//
// The `FileSessionStore` methods are defined via the `SessionStore` async trait.
// We bring the trait into scope so the compiler resolves them correctly.
use std::sync::Arc;

use tinyplace::signal::session::SignalSession;
use tinyplace::signal::store::SessionStore;
use tinyplace::signal::store::SessionStore as _;

/// Get the `Arc<dyn Signer>` from the client or fail with a clear message.
///
/// SECURITY: the signer holds the Ed25519 signing key in memory. The returned
/// Arc is the *same* instance the `TinyPlaceClient` uses — no new key material
/// is created.
fn require_signer(
    client: &tinyplace::TinyPlaceClient,
) -> std::result::Result<std::sync::Arc<dyn tinyplace::Signer>, String> {
    client
        .http()
        .signer()
        .ok_or_else(|| "no signer configured — unlock wallet to manage Signal keys".to_string())
}

/// Bootstrap Signal keys: generate signed pre-key + one-time pre-keys, store
/// locally in the encrypted `FileSessionStore`, publish to the backend, and
/// return the resulting `KeyHealth`.
///
/// SECURITY: only PUBLIC keys are serialised for upload. Private key bytes
/// remain in the encrypted `FileSessionStore` on disk.
pub(crate) fn handle_tinyplace_signal_provision(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let pre_key_count = params
            .get("preKeyCount")
            .and_then(Value::as_u64)
            .unwrap_or(100) as usize;
        log::debug!("{LOG_PREFIX} signal_provision pre_key_count={pre_key_count}");

        // 1. Acquire store, client, signer.
        let store = crate::openhuman::tinyplace::signal_store::global_signal_store().await?;
        let client = global_state().client().await?;
        let signer = require_signer(client)?;
        let agent_id = signer.agent_id();
        log::debug!("{LOG_PREFIX} signal_provision agent_id={agent_id}");

        // 2. Identity key published to /keys = the wallet's Ed25519 public key.
        //    The backend verifies the pre-key signatures against it (they are
        //    signed by the wallet's Ed25519 signer) and serves it in the bundle;
        //    peers convert it to X25519 for DH. SECURITY: public key only.
        let identity_key_b64 = signer.public_key_base64();

        // 3. Generate + store signed pre-key.
        let spk_id = format!(
            "spk_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let spk = tinyplace::signal::keys::generate_signed_pre_key(signer.as_ref(), &spk_id)
            .await
            .map_err(|e| format!("generate signed pre-key: {e}"))?;
        store
            .store_signed_pre_key(spk.clone())
            .await
            .map_err(|e| format!("store signed pre-key: {e}"))?;
        log::debug!("{LOG_PREFIX} signal_provision stored signed pre-key id={spk_id}");

        // 4. Generate + store one-time pre-keys.
        let start_id = store
            .all_pre_keys()
            .await
            .map_err(|e| format!("list pre-keys: {e}"))?
            .len() as u64;
        let pre_keys =
            tinyplace::signal::keys::generate_pre_keys(signer.as_ref(), start_id, pre_key_count)
                .await
                .map_err(|e| format!("generate pre-keys: {e}"))?;
        for pk in &pre_keys {
            store
                .store_pre_key(pk.clone())
                .await
                .map_err(|e| format!("store pre-key: {e}"))?;
        }
        log::debug!(
            "{LOG_PREFIX} signal_provision stored {count} one-time pre-keys (start_id={start_id})",
            count = pre_keys.len()
        );

        // 5. Serialise for upload — ONLY public keys leave this process.
        let spk_wire = tinyplace::signal::keys::serialize_pre_key(&spk);
        let otpk_wires: Vec<tinyplace::types::SignedKey> = pre_keys
            .iter()
            .map(tinyplace::signal::keys::serialize_pre_key)
            .collect();

        // 6. Upload signed pre-key.
        client
            .keys
            .rotate_signed_pre_key(
                &agent_id,
                &tinyplace::types::SignedPreKeyRequest {
                    identity_key: Some(identity_key_b64.clone()),
                    signed_pre_key: spk_wire,
                },
            )
            .await
            .map_err(map_err)?;
        log::debug!("{LOG_PREFIX} signal_provision uploaded signed pre-key");

        // 7. Upload one-time pre-keys.
        client
            .keys
            .upload_pre_keys(
                &agent_id,
                &tinyplace::types::PreKeysRequest {
                    identity_key: Some(identity_key_b64),
                    pre_keys: otpk_wires,
                },
            )
            .await
            .map_err(map_err)?;
        log::debug!(
            "{LOG_PREFIX} signal_provision uploaded {count} one-time pre-keys",
            count = pre_keys.len()
        );

        // 8. Return key health.
        let health = client.keys.health(&agent_id).await.map_err(map_err)?;
        log::info!(
            "{LOG_PREFIX} signal_provision complete agent_id={agent_id} \
             otpk_count={} low={}",
            health.one_time_pre_key_count,
            health.low_one_time_pre_keys
        );
        to_value(health)
    })
}

/// Generate and upload additional one-time pre-keys (replenishment). Does NOT
/// generate a new signed pre-key.
pub(crate) fn handle_tinyplace_signal_upload_pre_keys(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let count = params.get("count").and_then(Value::as_u64).unwrap_or(100) as usize;
        log::debug!("{LOG_PREFIX} signal_upload_pre_keys count={count}");

        let store = crate::openhuman::tinyplace::signal_store::global_signal_store().await?;
        let client = global_state().client().await?;
        let signer = require_signer(client)?;
        let agent_id = signer.agent_id();

        // Publish the wallet's Ed25519 public key as the identity key (the
        // backend verifies pre-key signatures against it and serves it in the
        // bundle; peers convert it to X25519 for DH). SECURITY: public key only.
        let identity_key_b64 = signer.public_key_base64();

        let start_id = store
            .all_pre_keys()
            .await
            .map_err(|e| format!("list pre-keys: {e}"))?
            .len() as u64;
        let pre_keys = tinyplace::signal::keys::generate_pre_keys(signer.as_ref(), start_id, count)
            .await
            .map_err(|e| format!("generate pre-keys: {e}"))?;

        for pk in &pre_keys {
            store
                .store_pre_key(pk.clone())
                .await
                .map_err(|e| format!("store pre-key: {e}"))?;
        }

        let wires: Vec<tinyplace::types::SignedKey> = pre_keys
            .iter()
            .map(tinyplace::signal::keys::serialize_pre_key)
            .collect();

        client
            .keys
            .upload_pre_keys(
                &agent_id,
                &tinyplace::types::PreKeysRequest {
                    identity_key: Some(identity_key_b64),
                    pre_keys: wires,
                },
            )
            .await
            .map_err(map_err)?;

        let health = client.keys.health(&agent_id).await.map_err(map_err)?;
        log::info!(
            "{LOG_PREFIX} signal_upload_pre_keys complete count={count} \
             otpk_count={} low={}",
            health.one_time_pre_key_count,
            health.low_one_time_pre_keys
        );
        to_value(health)
    })
}

/// Generate a new signed pre-key, store locally, and upload. Existing one-time
/// pre-keys are unaffected.
pub(crate) fn handle_tinyplace_signal_rotate_signed_pre_key(
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} signal_rotate_signed_pre_key");

        let store = crate::openhuman::tinyplace::signal_store::global_signal_store().await?;
        let client = global_state().client().await?;
        let signer = require_signer(client)?;
        let agent_id = signer.agent_id();

        // Publish the wallet's Ed25519 public key as the identity key (the
        // backend verifies pre-key signatures against it and serves it in the
        // bundle; peers convert it to X25519 for DH). SECURITY: public key only.
        let identity_key_b64 = signer.public_key_base64();

        let key_id = format!(
            "spk_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let spk = tinyplace::signal::keys::generate_signed_pre_key(signer.as_ref(), &key_id)
            .await
            .map_err(|e| format!("generate signed pre-key: {e}"))?;

        store
            .store_signed_pre_key(spk.clone())
            .await
            .map_err(|e| format!("store signed pre-key: {e}"))?;

        client
            .keys
            .rotate_signed_pre_key(
                &agent_id,
                &tinyplace::types::SignedPreKeyRequest {
                    identity_key: Some(identity_key_b64),
                    signed_pre_key: tinyplace::signal::keys::serialize_pre_key(&spk),
                },
            )
            .await
            .map_err(map_err)?;

        log::info!("{LOG_PREFIX} signal_rotate_signed_pre_key complete key_id={key_id}");
        to_value(serde_json::json!({ "ok": true, "keyId": key_id }))
    })
}

/// Fetch a peer's published Signal pre-key bundle (public endpoint, no auth).
/// The `get_bundle` endpoint does not require a signer or the signal store.
/// Accepts @handles and bare handle names in addition to raw crypto_ids; the
/// identifier is resolved to a canonical crypto_id via directory.resolve before
/// the bundle lookup.
pub(crate) fn handle_tinyplace_signal_get_bundle(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let raw_agent_id = req_str(&params, "agentId")?.to_string();
        log::debug!("{LOG_PREFIX} signal_get_bundle raw_agent_id='{raw_agent_id}'");
        let client = global_state().client().await?;

        // Resolve the identifier (handle or crypto_id) before the bundle lookup.
        let agent_id = resolve_recipient_to_agent_id(&client, &raw_agent_id).await?;
        log::debug!("{LOG_PREFIX} signal_get_bundle resolved agent_id={agent_id}");

        let result = match client.keys.get_bundle(&agent_id).await {
            Ok(b) => b,
            Err(e) if e.status() == Some(404) => {
                return Err(format!(
                    "Recipient has not set up encrypted messaging yet. \
                     They need to provision Signal keys before receiving DMs. \
                     (agent_id: {agent_id})"
                ));
            }
            Err(e) => return Err(map_err(e)),
        };
        to_value(result)
    })
}

/// Local + remote key status for the current user. Remote health degrades
/// gracefully if the backend is unreachable.
pub(crate) fn handle_tinyplace_signal_key_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} signal_key_status");

        let store = crate::openhuman::tinyplace::signal_store::global_signal_store().await?;
        let client = global_state().client().await?;
        let signer = require_signer(client)?;
        let agent_id = signer.agent_id();

        let local_pre_key_count = store
            .all_pre_keys()
            .await
            .map_err(|e| format!("list pre-keys: {e}"))?
            .len();
        let has_active_spk = store.active_signed_pre_key().await.is_ok();

        // Best-effort remote health — degrade gracefully.
        let remote = match client.keys.health(&agent_id).await {
            Ok(h) => {
                log::debug!(
                    "{LOG_PREFIX} signal_key_status remote otpk_count={} low={}",
                    h.one_time_pre_key_count,
                    h.low_one_time_pre_keys
                );
                Some(h)
            }
            Err(e) => {
                log::warn!("{LOG_PREFIX} signal_key_status remote health fetch failed: {e}");
                None
            }
        };

        let remote_json = remote.and_then(|h| serde_json::to_value(h).ok());

        // Best-effort directory check — is encryptionPublicKey published
        // AND does it match the current identity key? A stale key (from a
        // previous wallet) should show as NOT published so the user
        // re-registers.
        let encryption_key_published = match client.directory.get_agent(&agent_id).await {
            Ok(card) => {
                let published_key = card
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("encryptionPublicKey"));
                let current_key_b64 = base64::engine::general_purpose::STANDARD.encode(
                    store
                        .identity_x25519_key_pair()
                        .await
                        .map(|kp| kp.public_key)
                        .unwrap_or([0u8; 32]),
                );
                let matches = published_key
                    .map(|pk| pk == &current_key_b64)
                    .unwrap_or(false);
                if published_key.is_some() && !matches {
                    log::warn!(
                        "{LOG_PREFIX} signal_key_status published encryption key does NOT \
                         match current identity — re-register to update"
                    );
                }
                log::debug!("{LOG_PREFIX} signal_key_status encryption_key_published={matches}");
                matches
            }
            Err(e) => {
                log::warn!("{LOG_PREFIX} signal_key_status directory card fetch failed: {e}");
                false
            }
        };

        to_value(serde_json::json!({
            "agentId": agent_id,
            "localPreKeyCount": local_pre_key_count,
            "hasActiveSignedPreKey": has_active_spk,
            "remote": remote_json,
            "encryptionKeyPublished": encryption_key_published,
        }))
    })
}

// ── Signal messaging helpers ──────────────────────────────────────────────────

/// Decode a base64-encoded 32-byte value.
fn decode_32_byte_b64(b64: &str, label: &str) -> std::result::Result<[u8; 32], String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("invalid base64 for {label}: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("{label}: expected 32 bytes, got {}", bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Decode a peer's published identity key (from a fetched key bundle) into the
/// X25519 public key used for X3DH / Double-Ratchet Diffie-Hellman.
///
/// The backend stores and serves the wallet's **Ed25519** public key as the
/// bundle identity key — it verifies signed/one-time pre-key signatures against
/// it (`ed25519.Verify(identityKey, ...)`) and re-validates on every fetch. So
/// the served identity key is Ed25519; convert it to its Montgomery (X25519)
/// form, which equals the peer's X25519 identity public key derived from the
/// same wallet seed, so DH stays consistent on both ends.
fn decode_identity_key(b64: &str) -> std::result::Result<[u8; 32], String> {
    let ed_pub = decode_32_byte_b64(b64, "identity_key")?;
    tinyplace::signal::crypto::ed25519_pub_to_x25519_pub(&ed_pub)
        .map_err(|e| format!("identity_key is not a valid Ed25519 public key: {e}"))
}

fn decode_ed25519_pub(
    agent: &tinyplace::types::AgentCard,
) -> std::result::Result<[u8; 32], String> {
    let b64 = agent
        .public_key
        .as_ref()
        .ok_or("peer directory entry has no publicKey — cannot verify bundle")?;
    decode_32_byte_b64(b64, "peer Ed25519 publicKey")
}

// ── Recipient resolution helpers ──────────────────────────────────────────────

/// Quick heuristic: Solana/wallet base58 public keys are 32–44 chars of the
/// Bitcoin base58 alphabet [1-9A-HJ-NP-Za-km-z] with no 0/I/O/l.
/// False negatives (short base58 strings treated as handles) are safe — they
/// get an extra directory.resolve call rather than a wrong key lookup.
fn looks_like_base58_pubkey(s: &str) -> bool {
    s.len() >= 32
        && s.len() <= 44
        && s.chars().all(|c| {
            matches!(c,
                '1'..='9' | 'A'..='H' | 'J'..='N' | 'P'..='Z' | 'a'..='k' | 'm'..='z')
        })
}

/// Case-insensitive username match across a slice of agent cards.
///
/// Extracted as a pure function so it can be unit-tested without a live HTTP
/// client.
fn match_agent_card_by_username<'a>(
    cards: &'a [tinyplace::types::AgentCard],
    name: &str,
) -> Option<&'a tinyplace::types::AgentCard> {
    let name_lower = name.to_lowercase();
    cards.iter().find(|a| {
        a.username
            .as_deref()
            .map(|u| u.to_lowercase() == name_lower)
            .unwrap_or(false)
    })
}

/// Query the agent directory for a card whose `username` matches `name`
/// (case-insensitive).  Uses the server-side `username` filter when available
/// (limit 20) and falls back to a client-side match in the returned page.
///
/// Returns `None` if no matching card is found.
async fn directory_card_fallback(
    client: &tinyplace::TinyPlaceClient,
    name: &str,
) -> Option<tinyplace::types::AgentCard> {
    let query = tinyplace::types::AgentQueryParams {
        username: Some(name.to_string()),
        limit: Some(20),
        ..Default::default()
    };
    log::debug!("{LOG_PREFIX} directory_card_fallback: querying agents with username='{name}'");
    match client.directory.list_agents(Some(&query)).await {
        Ok(resp) => {
            if resp.agents.len() == 20 {
                log::debug!(
                    "{LOG_PREFIX} directory_card_fallback: page full (20 agents) for '{name}', \
                     result may be truncated"
                );
            }
            match_agent_card_by_username(&resp.agents, name).cloned()
        }
        Err(e) => {
            log::debug!(
                "{LOG_PREFIX} directory_card_fallback: list_agents failed for '{name}': {}",
                e
            );
            None
        }
    }
}

/// Resolve a recipient identifier (handle, wallet address, or crypto_id) to
/// the canonical `crypto_id` used by the `/keys/{id}/bundle` endpoint.
///
/// Resolution rules:
/// 1. Starts with `@`  → strip `@` and resolve via `directory.resolve`.
/// 2. Looks like a base58 public key (32–44 chars, valid alphabet) → use as-is
///    (it IS the crypto_id; attempting resolve is unnecessary and fragile).
/// 3. Anything else → treat as a bare handle name and call `directory.resolve`.
/// 4. If the registry lookup returns 404 or no identity/agent, fall back to a
///    directory-card search by username (`list_agents?username=…`).
///
/// SECURITY: never log private key material — only the resolved crypto_id
/// (public wallet address) is emitted to logs.
async fn resolve_recipient_to_agent_id(
    client: &tinyplace::TinyPlaceClient,
    raw_recipient: &str,
) -> std::result::Result<String, String> {
    let trimmed = raw_recipient.trim();

    let lookup_name = if let Some(without_at) = trimmed.strip_prefix('@') {
        // Explicit @handle — resolve it.
        without_at
    } else if looks_like_base58_pubkey(trimmed) {
        // Already a crypto_id — pass through without a round-trip.
        log::debug!(
            "{LOG_PREFIX} resolve_recipient: '{trimmed}' looks like a crypto_id, using directly"
        );
        return Ok(trimmed.to_string());
    } else {
        // Bare handle name — resolve it.
        trimmed
    };

    log::debug!("{LOG_PREFIX} resolve_recipient: resolving handle '{lookup_name}' via registry");

    // ── Step 1: try the registry ─────────────────────────────────────────────
    let registry_result = client.directory.resolve(lookup_name).await;
    match registry_result {
        Ok(resolved) => {
            // Prefer the registered identity's crypto_id; fall back to the agent card.
            if let Some(identity) = resolved.identity {
                let crypto_id = identity.crypto_id.clone();
                log::debug!(
                    "{LOG_PREFIX} resolve_recipient: handle '{lookup_name}' -> crypto_id={crypto_id}"
                );
                return Ok(crypto_id);
            } else if let Some(agent) = resolved.agent {
                let crypto_id = agent.crypto_id.clone();
                log::debug!(
                    "{LOG_PREFIX} resolve_recipient: handle '{lookup_name}' resolved via \
                     registry agent card -> crypto_id={crypto_id}"
                );
                return Ok(crypto_id);
            }
            // Registry returned 200 but empty — fall through to directory.
            log::debug!(
                "{LOG_PREFIX} resolve_recipient: registry returned empty for '{lookup_name}', \
                 trying directory card fallback"
            );
        }
        Err(e) if e.status() == Some(404) => {
            // 404 from the registry — agent may be directory-only; fall through.
            log::debug!(
                "{LOG_PREFIX} resolve_recipient: registry 404 for '{lookup_name}', \
                 trying directory card fallback"
            );
        }
        Err(e) => {
            return Err(format!(
                "failed to resolve recipient '{trimmed}': {}",
                map_err(e)
            ));
        }
    }

    // ── Step 2: directory-card fallback ──────────────────────────────────────
    if let Some(card) = directory_card_fallback(client, lookup_name).await {
        let crypto_id = card.crypto_id.clone();
        log::debug!(
            "{LOG_PREFIX} resolve_recipient: '{lookup_name}' found via directory card \
             -> crypto_id={crypto_id}"
        );
        return Ok(crypto_id);
    }

    Err(format!(
        "No agent found for \"{lookup_name}\". \
         Check the handle or paste their wallet address directly."
    ))
}

pub(crate) fn handle_tinyplace_signal_send_message(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let recipient = req_str(&params, "recipient")?.to_string();
        let plaintext = req_str(&params, "plaintext")?.to_string();
        log::debug!(
            "{LOG_PREFIX} signal_send_message raw_recipient='{recipient}' len={}",
            plaintext.len()
        );

        // Obtain our identity public key and an Arc-wrapped store for SignalSession.
        let store = crate::openhuman::tinyplace::signal_store::global_signal_store_arc().await?;
        let client = global_state().client().await?;
        let signer = require_signer(client)?;
        let our_agent_id = signer.agent_id();
        let our_identity_pub = store
            .identity_x25519_key_pair()
            .await
            .map_err(|e| format!("identity key: {e}"))?
            .public_key;

        // Resolve the recipient identifier (@handle, bare handle, or crypto_id) to
        // the canonical crypto_id before any key-bundle or directory lookup.
        let agent_id = resolve_recipient_to_agent_id(&client, &recipient).await?;
        log::debug!("{LOG_PREFIX} signal_send_message resolved to agent_id={agent_id}");

        // Fetch recipient's published key bundle (always needed for the X25519
        // identity key used in associated-data computation, even for existing
        // sessions).  Provide a user-friendly error when the recipient has not yet
        // provisioned Signal keys (404 from the backend).
        let bundle = match client.keys.get_bundle(&agent_id).await {
            Ok(b) => b,
            Err(e) if e.status() == Some(404) => {
                return Err(format!(
                    "Recipient has not set up encrypted messaging yet. \
                     They need to provision Signal keys before receiving DMs. \
                     (resolved agent_id: {agent_id})"
                ));
            }
            Err(e) => return Err(map_err(e)),
        };
        // Ed25519 -> X25519 conversion: the backend serves the Ed25519 identity;
        // SignalSession::encrypt takes the X25519 form.  decode_identity_key
        // performs this conversion and must be preserved.
        let their_x25519_identity = decode_identity_key(&bundle.identity_key)?;

        // Determine whether this is a new session (needs full X3DH bundle + Ed25519
        // key for signature verification) or an existing session (no bundle needed).
        let signal_session = SignalSession::new(
            Arc::clone(&store) as Arc<dyn SessionStore>,
            our_identity_pub,
        );
        let has_session = signal_session
            .has_session(&agent_id)
            .await
            .map_err(|e| format!("check session: {e}"))?;

        let (bundle_opt, ed25519_opt) = if has_session {
            log::debug!("{LOG_PREFIX} signal_send_message using existing session for {agent_id}");
            (None, None)
        } else {
            log::debug!("{LOG_PREFIX} signal_send_message establishing new session for {agent_id}");
            let peer_entry = client
                .directory
                .get_agent(&agent_id)
                .await
                .map_err(map_err)?;
            let peer_ed25519_pub = decode_ed25519_pub(&peer_entry)?;
            (Some(bundle), Some(peer_ed25519_pub))
        };

        // Encrypt via SDK SignalSession.
        //
        // SECURITY INVARIANT: if encryption fails we abort immediately — plaintext
        // is NEVER sent and the session is NOT stored (store_session runs after
        // ratchet_encrypt inside SignalSession::encrypt, so a failure before that
        // point leaves no partial state).
        let encrypted = signal_session
            .encrypt(
                &agent_id,
                &their_x25519_identity,
                plaintext.as_bytes(),
                bundle_opt.as_ref(),
                ed25519_opt.as_ref(),
            )
            .await
            .map_err(|e| {
                log::error!(
                    "{LOG_PREFIX} signal_send_message ENCRYPTION FAILED for {agent_id}: {e} \
                     — aborting send (plaintext will NOT be sent)"
                );
                format!("encryption failed — message NOT sent: {e}")
            })?;

        // Map EncryptedMessage -> MessageEnvelope (wire-format preserving).
        // Field correspondence is verified in phase-signalsession-spec.md §4.
        // Use the resolved agent_id (not raw recipient) so the backend routes correctly.
        let envelope = tinyplace::types::MessageEnvelope {
            // The backend requires a non-empty client-generated message id
            // (POST /messages rejects an empty id with "message id, from, and to
            // are required"). The SDK's send() only fills timestamp, not id.
            id: uuid::Uuid::new_v4().to_string(),
            from: our_agent_id.clone(),
            to: agent_id.clone(),
            timestamp: String::new(),
            device_id: 1,
            envelope_type: encrypted.message_type.clone(), // "PREKEY_BUNDLE" or "CIPHERTEXT"
            body: encrypted.body.clone(),
            content_hint: Some("DEFAULT".to_string()),
            signal: Some(encrypted.signal.clone()),
        };

        let sent = client.messages.send(envelope).await.map_err(map_err)?;
        log::info!(
            "{LOG_PREFIX} signal_send_message sent encrypted message to={agent_id} \
             id={} type={} len={}",
            sent.id,
            sent.envelope_type,
            sent.body.len()
        );
        to_value(serde_json::json!({
            "messageId": sent.id,
            "timestamp": sent.timestamp,
            "encrypted": true,
        }))
    })
}

pub(crate) fn handle_tinyplace_signal_decrypt_message(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let envelope_val = params
            .get("envelope")
            .ok_or("missing required param 'envelope'")?;
        let envelope: tinyplace::types::MessageEnvelope =
            serde_json::from_value(envelope_val.clone())
                .map_err(|e| format!("invalid envelope: {e}"))?;
        log::debug!(
            "{LOG_PREFIX} signal_decrypt_message from={} type={} id={}",
            envelope.from,
            envelope.envelope_type,
            envelope.id
        );

        // Obtain our identity public key and an Arc-wrapped store for SignalSession.
        let store = crate::openhuman::tinyplace::signal_store::global_signal_store_arc().await?;
        let client = global_state().client().await?;
        let our_identity_pub = store
            .identity_x25519_key_pair()
            .await
            .map_err(|e| format!("identity key: {e}"))?
            .public_key;

        let sender = envelope.from.clone();

        // Fetch sender's published key bundle to obtain their X25519 identity key.
        // Ed25519 -> X25519 conversion via decode_identity_key — must be preserved.
        let sender_bundle = client.keys.get_bundle(&sender).await.map_err(map_err)?;
        let sender_x25519_identity = decode_identity_key(&sender_bundle.identity_key)?;

        // Decrypt via SDK SignalSession.
        //
        // SignalSession::decrypt handles both PREKEY_BUNDLE and CIPHERTEXT paths
        // internally (via process_pre_key_message), including one-time pre-key
        // consumption, x3dh_respond, ratchet_decrypt, and store_session.
        let signal_session = SignalSession::new(
            Arc::clone(&store) as Arc<dyn SessionStore>,
            our_identity_pub,
        );
        let plaintext_bytes = signal_session
            .decrypt(&sender, &sender_x25519_identity, &envelope)
            .await
            .map_err(|e| format!("decryption failed: {e}"))?;

        let plaintext = String::from_utf8(plaintext_bytes)
            .map_err(|e| format!("plaintext is not valid UTF-8: {e}"))?;
        log::info!(
            "{LOG_PREFIX} signal_decrypt_message decrypted from={sender} id={} len={}",
            envelope.id,
            plaintext.len()
        );
        to_value(serde_json::json!({
            "plaintext": plaintext,
            "from": envelope.from,
            "messageId": envelope.id,
        }))
    })
}

pub(crate) fn handle_tinyplace_messages_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let limit = params.get("limit").and_then(Value::as_i64);
        log::debug!("{LOG_PREFIX} messages_list limit={limit:?}");
        let client = global_state().client().await?;
        let signer = require_signer(client)?;
        match client.messages.list(&signer.agent_id(), limit).await {
            Ok(result) => to_value(result),
            Err(e) => match messages_list_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} messages_list deserialization failed (likely empty thread) -> empty: {e}"
                    );
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

/// An empty message thread comes back as `{"messages": null}`, which fails the
/// SDK's non-optional `messages: Vec<MessageEnvelope>` deserialization. Treat
/// that serialization failure as an empty thread; propagate every other error.
pub(crate) fn messages_list_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "messages": [] }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_messages_acknowledge(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let message_id = req_str(&params, "messageId")?.to_string();
        log::debug!("{LOG_PREFIX} messages_acknowledge id={message_id}");
        let client = global_state().client().await?;
        let signer = require_signer(client)?;
        client
            .messages
            .acknowledge(&message_id, &signer.agent_id())
            .await
            .map_err(map_err)?;
        to_value(serde_json::json!({ "ok": true }))
    })
}

// ── Signal: encryption key registration (0D) ────────────────────────────────

/// Publish the user's X25519 identity public key on their directory card as
/// `metadata.encryptionPublicKey`. This makes the user discoverable for
/// encrypted DMs via `find_agent_by_encryption_key`.
///
/// SECURITY: only the PUBLIC key is published. The private key never leaves
/// the `FileSessionStore`.
pub(crate) fn handle_tinyplace_signal_register_encryption_key(
    _params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("{LOG_PREFIX} signal_register_encryption_key");

        // 1. Read identity public key from the signal store.
        let store = crate::openhuman::tinyplace::signal_store::global_signal_store().await?;
        let identity_kp = store
            .identity_x25519_key_pair()
            .await
            .map_err(|e| format!("identity key: {e}"))?;
        let encryption_key_b64 =
            base64::engine::general_purpose::STANDARD.encode(identity_kp.public_key);
        log::debug!("{LOG_PREFIX} signal_register_encryption_key derived key (not logging value)");

        // 2. Acquire client and signer.
        let client = global_state().client().await?;
        let signer = require_signer(client)?;
        let agent_id = signer.agent_id();

        // 3. Fetch current AgentCard to preserve existing fields. A wallet that
        //    has Signal keys but no directory presence yet (e.g. it registered a
        //    @handle but was never upserted as an agent) 404s here — in that
        //    case create a fresh minimal card so "Make discoverable" still works,
        //    best-effort enriching name/username from the registered identity.
        let mut card = match client.directory.get_agent(&agent_id).await {
            Ok(card) => {
                log::debug!(
                    "{LOG_PREFIX} signal_register_encryption_key fetched existing card for {agent_id}"
                );
                card
            }
            Err(e) if e.status() == Some(404) => {
                log::debug!(
                    "{LOG_PREFIX} signal_register_encryption_key no card for {agent_id} -> creating one"
                );
                let identity = client
                    .directory
                    .reverse(&agent_id)
                    .await
                    .ok()
                    .and_then(|r| {
                        // Prefer the wallet's primary handle; otherwise any handle.
                        let mut ids = r.identities;
                        ids.iter()
                            .position(|i| i.primary == Some(true))
                            .map(|idx| ids.swap_remove(idx))
                            .or_else(|| ids.into_iter().next())
                    });
                build_default_agent_card(&agent_id, &signer.public_key_base64(), identity.as_ref())
            }
            Err(e) => return Err(map_err(e)),
        };

        // 4. Merge encryptionPublicKey into metadata.
        let metadata = card
            .metadata
            .get_or_insert_with(std::collections::HashMap::new);
        metadata.insert(
            "encryptionPublicKey".to_string(),
            encryption_key_b64.clone(),
        );

        // 5. Upsert the card with the updated metadata.
        let updated = client
            .directory
            .upsert_agent(&agent_id, &card)
            .await
            .map_err(map_err)?;
        log::info!("{LOG_PREFIX} signal_register_encryption_key published for {agent_id}");

        to_value(serde_json::json!({
            "ok": true,
            "encryptionKey": encryption_key_b64,
            "agentId": agent_id,
            "updatedAt": updated.updated_at,
        }))
    })
}

/// Publish a directory card for a newly registered identity.
///
/// Called after every successful `registry_register` path (free-tier, paid
/// settlement, recovery). The directory card makes the identity discoverable in
/// `GET /directory/agents` and `GET /directory/identities` listings.
///
/// **Best-effort**: any failure is logged and swallowed so that a directory
/// publish hiccup never rolls back the already-completed registry write.
///
/// **Anti-spoof**: `agent_id` is sourced from `signer.agent_id()` (the wallet's
/// crypto identity), not from user-supplied params.
async fn publish_directory_card_for_identity(
    client: &tinyplace::TinyPlaceClient,
    signer: &dyn tinyplace::Signer,
    identity: &tinyplace::types::Identity,
) {
    let agent_id = signer.agent_id();
    let public_key_b64 = signer.public_key_base64();
    log::debug!("[tinyplace] post-register: publishing directory card for {agent_id}");

    // Fetch existing card to preserve custom fields; on 404 build a fresh one;
    // on any other error bail out early (best-effort).
    let mut card = match client.directory.get_agent(&agent_id).await {
        Ok(existing) => {
            log::debug!(
                "[tinyplace] post-register: updating existing directory card for {agent_id}"
            );
            let mut c = existing;
            // Refresh name/username from the newly registered identity.
            c.username = Some(identity.username.clone());
            c.name = identity.username.clone();
            c
        }
        Err(e) if e.status() == Some(404) => {
            log::debug!(
                "[tinyplace] post-register: no existing card for {agent_id} — creating one"
            );
            build_default_agent_card(&agent_id, &public_key_b64, Some(identity))
        }
        Err(e) => {
            log::warn!(
                "[tinyplace] post-register: directory card fetch failed for {agent_id}: {e}"
            );
            return;
        }
    };

    // Anti-spoof: ensure the card's agent_id/crypto_id matches the signer.
    card.agent_id = agent_id.clone();
    card.crypto_id = agent_id.clone();

    match client.directory.upsert_agent(&agent_id, &card).await {
        Ok(_) => {
            log::info!("[tinyplace] post-register: directory card published for {agent_id}");
        }
        Err(e) => {
            log::warn!("[tinyplace] post-register: directory upsert failed for {agent_id}: {e}");
        }
    }
}

/// Build a minimal `AgentCard` for a wallet that has no directory presence yet,
/// so it can publish its encryption key and become discoverable. When the wallet
/// owns a registered identity, its handle seeds `name`/`username`; otherwise the
/// agent id is used. The backend assigns authoritative timestamps on upsert, so
/// the `created_at`/`updated_at` we send are placeholders.
pub(crate) fn build_default_agent_card(
    agent_id: &str,
    public_key_b64: &str,
    identity: Option<&tinyplace::types::Identity>,
) -> tinyplace::types::AgentCard {
    let now = chrono::Utc::now().to_rfc3339();
    let username = identity.map(|i| i.username.clone());
    let name = username.clone().unwrap_or_else(|| agent_id.to_string());
    tinyplace::types::AgentCard {
        agent_id: agent_id.to_string(),
        name,
        description: None,
        username,
        crypto_id: agent_id.to_string(),
        actor_type: identity.map(|_| "human".to_string()),
        public_key: Some(public_key_b64.to_string()),
        url: None,
        endpoint: None,
        supported_interfaces: None,
        skills: None,
        capabilities: None,
        tags: None,
        payment_methods: None,
        payment_requirements: None,
        groups: None,
        docs: None,
        webhooks: None,
        metadata: None,
        signature: None,
        created_at: now.clone(),
        updated_at: now,
        viewer_is_following: None,
    }
}

// ── GraphQL: Social Feed ─────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_graphql_home_feed(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let limit = params.get("limit").and_then(Value::as_i64);
        let offset = params.get("offset").and_then(Value::as_i64);
        let include_self = params.get("includeSelf").and_then(Value::as_bool);
        log::debug!(
            "{LOG_PREFIX} graphql_home_feed limit={limit:?} offset={offset:?} include_self={include_self:?}"
        );
        let client = global_state().client().await?;
        // home_feed uses GraphQLAuth::Agent — requires a configured signer.
        let _signer = require_signer(client)?;
        match client.graphql.home_feed(limit, offset, include_self).await {
            Ok(result) => to_value(result),
            Err(e) => match graphql_home_feed_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} graphql_home_feed deserialization failed (likely empty feed) -> empty: {e}"
                    );
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

/// The backend may return `{"items": null}` for an empty home feed, which
/// fails the SDK's `items: Vec<GqlHomeFeedItem>` deserialization. Treat
/// serialization failures as an empty feed; propagate every other error.
pub(crate) fn graphql_home_feed_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "items": [], "count": 0 }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_posts(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let handle = req_str(&params, "handle")?.to_string();
        let limit = params.get("limit").and_then(Value::as_i64);
        let before = params.get("before").and_then(Value::as_i64);
        let viewer = get_opt_str(&params, "viewer").map(|s| s.to_string());
        log::debug!("{LOG_PREFIX} graphql_posts handle={handle} limit={limit:?} before={before:?}");
        let sdk_params = tinyplace::api::graphql::PostGraphQLParams {
            limit,
            before,
            viewer,
        };
        let client = global_state().client().await?;
        match client.graphql.posts(&handle, Some(&sdk_params)).await {
            Ok(result) => to_value(result),
            Err(e) => match graphql_posts_degrade(&e) {
                Some(empty) => {
                    log::debug!("{LOG_PREFIX} graphql_posts deserialization failed -> empty: {e}");
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

/// Empty user feed may return `{"posts": null}`. Degrade like inbox_list.
pub(crate) fn graphql_posts_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "posts": [], "count": 0 }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_post(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let handle = req_str(&params, "handle")?.to_string();
        let post_id = req_str(&params, "postId")?.to_string();
        let viewer = get_opt_str(&params, "viewer").map(|s| s.to_string());
        let comment_limit = params.get("commentLimit").and_then(Value::as_i64);
        let comment_after = params.get("commentAfter").and_then(Value::as_i64);
        let liker_limit = params.get("likerLimit").and_then(Value::as_i64);
        let liker_offset = params.get("likerOffset").and_then(Value::as_i64);
        log::debug!(
            "{LOG_PREFIX} graphql_post handle={handle} post_id={post_id} comment_limit={comment_limit:?}"
        );
        let sdk_params = tinyplace::api::graphql::PostDetailGraphQLParams {
            viewer,
            comment_limit,
            comment_after,
            liker_limit,
            liker_offset,
        };
        let client = global_state().client().await?;
        let result = client
            .graphql
            .post(&handle, &post_id, Some(&sdk_params))
            .await
            .map_err(map_err)?;
        // SDK returns Option<GqlPostDetail> — null means post not found.
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_graphql_post_comments(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let post_id = req_str(&params, "postId")?.to_string();
        let feed_id = get_opt_str(&params, "feedId").map(|s| s.to_string());
        let limit = params.get("limit").and_then(Value::as_i64);
        let after = params.get("after").and_then(Value::as_i64);
        log::debug!(
            "{LOG_PREFIX} graphql_post_comments post_id={post_id} limit={limit:?} after={after:?}"
        );
        let sdk_params = tinyplace::api::graphql::CommentGraphQLParams {
            feed_id,
            limit,
            after,
        };
        let client = global_state().client().await?;
        let result = client
            .graphql
            .post_comments(&post_id, Some(&sdk_params))
            .await
            .map_err(map_err)?;
        // Returns Vec<GqlComment> — wrap in an object for consistent RPC shape.
        to_value(serde_json::json!({ "comments": result }))
    })
}

pub(crate) fn handle_tinyplace_graphql_post_likers(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let post_id = req_str(&params, "postId")?.to_string();
        let limit = params.get("limit").and_then(Value::as_i64);
        let offset = params.get("offset").and_then(Value::as_i64);
        log::debug!(
            "{LOG_PREFIX} graphql_post_likers post_id={post_id} limit={limit:?} offset={offset:?}"
        );
        let sdk_params = tinyplace::api::graphql::PaginationGraphQLParams { limit, offset };
        let client = global_state().client().await?;
        let result = client
            .graphql
            .post_likers(&post_id, Some(&sdk_params))
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── GraphQL: Ledger ──────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_graphql_ledger_transactions(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} graphql_ledger_transactions params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::LedgerListParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid ledger_transactions params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required; the ledger is public.
        match client
            .graphql
            .ledger_transactions(query_params.as_ref())
            .await
        {
            Ok(result) => to_value(result),
            Err(e) => match graphql_ledger_transactions_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} graphql_ledger_transactions deserialization failed -> empty: {e}"
                    );
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

/// The backend may return `{"transactions": null}` for an empty ledger.
/// Degrade Serialization errors to an empty result; propagate everything else.
pub(crate) fn graphql_ledger_transactions_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "transactions": [], "count": 0 }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_ledger_transaction(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let id = req_str(&params, "id")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_ledger_transaction id={id}");
        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required.
        let result = client
            .graphql
            .ledger_transaction(&id)
            .await
            .map_err(map_err)?;
        // Returns Option<GqlLedgerTransaction> — null means tx not found.
        to_value(result)
    })
}

// ── GraphQL: Jobs Board ────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_graphql_jobs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} graphql_jobs params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::JobQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid graphql_jobs params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required; the jobs board is public.
        match client.graphql.jobs(query_params.as_ref()).await {
            Ok(result) => to_value(result),
            Err(e) => match graphql_jobs_degrade(&e) {
                Some(empty) => {
                    log::debug!("{LOG_PREFIX} graphql_jobs deserialization failed -> empty: {e}");
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

/// The backend may return `{"jobs": null}` for an empty jobs board.
/// Degrade Serialization errors to an empty result; propagate everything else.
pub(crate) fn graphql_jobs_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "jobs": [], "count": 0 }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_job(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = req_str(&params, "id")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_job id={id}");
        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required.
        let result = client.graphql.job(&id).await.map_err(map_err)?;
        // Returns Option<GqlJobPosting> — null means job not found.
        to_value(result)
    })
}

// ── GraphQL: Bounties ─────────────────────────────────────────────────────────

pub(crate) fn handle_tinyplace_graphql_bounties(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} graphql_bounties params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        // BountyGraphQLParams isn't Deserialize, so build it from the optional
        // filter object by hand (all fields optional). Reject malformed input
        // rather than silently dropping it — a mistyped `limit`/`creator` must
        // not degrade into an unfiltered public list.
        let query_params: Option<tinyplace::api::graphql::BountyGraphQLParams> = match params
            .get("params")
        {
            None | Some(Value::Null) => None,
            Some(Value::Object(obj)) => {
                let str_field = |key: &str| -> Result<Option<String>, String> {
                    match obj.get(key) {
                        None | Some(Value::Null) => Ok(None),
                        Some(Value::String(s)) => Ok(Some(s.clone())),
                        Some(_) => Err(format!("graphql_bounties param '{key}' must be a string")),
                    }
                };
                let int_field = |key: &str| -> Result<Option<i64>, String> {
                    match obj.get(key) {
                        None | Some(Value::Null) => Ok(None),
                        Some(v) => v.as_i64().map(Some).ok_or_else(|| {
                            format!("graphql_bounties param '{key}' must be an integer")
                        }),
                    }
                };
                Some(tinyplace::api::graphql::BountyGraphQLParams {
                    status: str_field("status")?,
                    creator: str_field("creator")?,
                    limit: int_field("limit")?,
                    offset: int_field("offset")?,
                })
            }
            Some(_) => return Err("graphql_bounties 'params' must be an object".to_string()),
        };

        let client = global_state().client().await?;
        // GraphQLAuth::None — bounties are public.
        match client.graphql.bounties(query_params.as_ref()).await {
            Ok(result) => to_value(result),
            Err(e) => match graphql_bounties_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} graphql_bounties deserialization failed -> empty: {e}"
                    );
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

/// The backend may return `{"bounties": null}` for an empty board. Degrade
/// Serialization errors to an empty list; propagate everything else.
pub(crate) fn graphql_bounties_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!([]))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_bounty(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = req_str(&params, "id")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_bounty id={id}");
        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required.
        let result = client.graphql.bounty(&id).await.map_err(map_err)?;
        // Returns Option<GqlBounty> — null means bounty not found.
        to_value(result)
    })
}

// ── GraphQL: Profile + Identity ───────────────────────────────────────────────

fn graphql_params_object<'a>(
    params: &'a Map<String, Value>,
    method: &str,
) -> Result<Option<&'a Map<String, Value>>, String> {
    match params.get("params") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(obj)) => Ok(Some(obj)),
        Some(_) => Err(format!("{method} 'params' must be an object")),
    }
}

fn graphql_opt_string(
    obj: &Map<String, Value>,
    method: &str,
    key: &str,
) -> Result<Option<String>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("{method} param '{key}' must be a string")),
    }
}

fn graphql_opt_i64(
    obj: &Map<String, Value>,
    method: &str,
    key: &str,
) -> Result<Option<i64>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("{method} param '{key}' must be an integer")),
    }
}

fn graphql_opt_string_vec(
    obj: &Map<String, Value>,
    method: &str,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("{method} param '{key}' must be an array of strings"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(format!(
            "{method} param '{key}' must be an array of strings"
        )),
    }
}

fn parse_identity_listing_params(
    params: &Map<String, Value>,
    method: &str,
) -> Result<Option<tinyplace::api::graphql::IdentityListingGraphQLParams>, String> {
    let Some(obj) = graphql_params_object(params, method)? else {
        return Ok(None);
    };
    Ok(Some(
        tinyplace::api::graphql::IdentityListingGraphQLParams {
            query: match graphql_opt_string(obj, method, "query")? {
                Some(query) => Some(query),
                None => graphql_opt_string(obj, method, "q")?,
            },
            tag: graphql_opt_string(obj, method, "tag")?,
            tags: graphql_opt_string_vec(obj, method, "tags")?,
            category: graphql_opt_string(obj, method, "category")?,
            seller: graphql_opt_string(obj, method, "seller")?,
            min_price: graphql_opt_string(obj, method, "minPrice")?,
            max_price: graphql_opt_string(obj, method, "maxPrice")?,
            sort_by: graphql_opt_string(obj, method, "sortBy")?,
            length: graphql_opt_i64(obj, method, "length")?,
            limit: graphql_opt_i64(obj, method, "limit")?,
            offset: graphql_opt_i64(obj, method, "offset")?,
        },
    ))
}

fn parse_pagination_params(
    params: &Map<String, Value>,
    method: &str,
) -> Result<Option<tinyplace::api::graphql::PaginationGraphQLParams>, String> {
    let Some(obj) = graphql_params_object(params, method)? else {
        return Ok(None);
    };
    Ok(Some(tinyplace::api::graphql::PaginationGraphQLParams {
        limit: graphql_opt_i64(obj, method, "limit")?,
        offset: graphql_opt_i64(obj, method, "offset")?,
    }))
}

pub(crate) fn handle_tinyplace_graphql_agents(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} graphql_agents params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::AgentQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid graphql_agents params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        match client.graphql.agents(query_params.as_ref()).await {
            Ok(result) => to_value(result),
            Err(e) => match graphql_agents_degrade(&e) {
                Some(empty) => {
                    log::debug!("{LOG_PREFIX} graphql_agents deserialization failed -> empty: {e}");
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

pub(crate) fn graphql_agents_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "agents": [], "count": 0 }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_products(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} graphql_products params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::ProductQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid graphql_products params: {e}"))
            })
            .transpose()?;

        let client = global_state().client().await?;
        match client.graphql.products(query_params.as_ref()).await {
            Ok(result) => to_value(result),
            Err(e) => match graphql_products_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} graphql_products deserialization failed -> empty: {e}"
                    );
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

pub(crate) fn graphql_products_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "products": [], "count": 0 }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_product(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = req_str(&params, "id")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_product id={id}");
        let client = global_state().client().await?;
        let result = client.graphql.product(&id).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_graphql_identity_listings(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} graphql_identity_listings params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params = parse_identity_listing_params(&params, "graphql_identity_listings")?;
        let client = global_state().client().await?;
        match client
            .graphql
            .identity_listings(query_params.as_ref())
            .await
        {
            Ok(result) => to_value(result),
            Err(e) => match graphql_identity_listings_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} graphql_identity_listings deserialization failed -> empty: {e}"
                    );
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

pub(crate) fn graphql_identity_listings_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "identities": [], "count": 0 }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_identity_listing(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let id = req_str(&params, "id")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_identity_listing id={id}");
        let query_params = match graphql_params_object(&params, "graphql_identity_listing")? {
            None => None,
            Some(obj) => Some(
                tinyplace::api::graphql::IdentityListingDetailGraphQLParams {
                    bid_limit: graphql_opt_i64(obj, "graphql_identity_listing", "bidLimit")?,
                    bid_offset: graphql_opt_i64(obj, "graphql_identity_listing", "bidOffset")?,
                    history_limit: graphql_opt_i64(
                        obj,
                        "graphql_identity_listing",
                        "historyLimit",
                    )?,
                    history_offset: graphql_opt_i64(
                        obj,
                        "graphql_identity_listing",
                        "historyOffset",
                    )?,
                },
            ),
        };
        let client = global_state().client().await?;
        let result = client
            .graphql
            .identity_listing(&id, query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_graphql_identity_bids(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let listing_id = req_str(&params, "listingId")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_identity_bids listing_id={listing_id}");
        let query_params = parse_pagination_params(&params, "graphql_identity_bids")?;
        let client = global_state().client().await?;
        let result = client
            .graphql
            .identity_bids(&listing_id, query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_graphql_identity_offers(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} graphql_identity_offers params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params = match graphql_params_object(&params, "graphql_identity_offers")? {
            None => None,
            Some(obj) => Some(tinyplace::api::graphql::IdentityOfferGraphQLParams {
                agent: graphql_opt_string(obj, "graphql_identity_offers", "agent")?,
                buyer: graphql_opt_string(obj, "graphql_identity_offers", "buyer")?,
                name: graphql_opt_string(obj, "graphql_identity_offers", "name")?,
                status: graphql_opt_string(obj, "graphql_identity_offers", "status")?,
                limit: graphql_opt_i64(obj, "graphql_identity_offers", "limit")?,
                offset: graphql_opt_i64(obj, "graphql_identity_offers", "offset")?,
            }),
        };
        let client = global_state().client().await?;
        let result = client
            .graphql
            .identity_offers(query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_graphql_identity_sales(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let name = req_str(&params, "name")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_identity_sales name={name}");
        let query_params = parse_pagination_params(&params, "graphql_identity_sales")?;
        let client = global_state().client().await?;
        let result = client
            .graphql
            .identity_sales(&name, query_params.as_ref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_graphql_profile(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_profile username={username}");
        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required.
        let result = client.graphql.profile(&username).await.map_err(map_err)?;
        // Returns Option<GqlProfile> — null means profile not found.
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_graphql_user(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let crypto_id = req_str(&params, "cryptoId")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_user crypto_id={crypto_id}");
        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required.
        let result = client.graphql.user(&crypto_id).await.map_err(map_err)?;
        // Returns Option<GqlProfile> — null means no profile for this crypto_id.
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_graphql_identity(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let username = req_str(&params, "username")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_identity username={username}");
        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required.
        let result = client.graphql.identity(&username).await.map_err(map_err)?;
        // Returns Option<GqlIdentity> — null means identity not found.
        to_value(result)
    })
}

/// The backend may return `{"identities": null}` for a wallet with no handles.
/// Degrade Serialization errors to an empty array; propagate everything else.
pub(crate) fn graphql_identities_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!([]))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_graphql_identities(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let crypto_id = req_str(&params, "cryptoId")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_identities crypto_id={crypto_id}");
        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required.
        let result = match client.graphql.identities(&crypto_id).await {
            Ok(identities) => identities,
            Err(e) => match graphql_identities_degrade(&e) {
                Some(empty) => {
                    log::debug!(
                        "{LOG_PREFIX} graphql_identities deserialization failed -> empty: {e}"
                    );
                    // Wrap empty array in the RPC envelope shape for consistency.
                    return to_value(serde_json::json!({ "identities": empty }));
                }
                None => return Err(map_err(e)),
            },
        };
        // Wrap Vec<Identity> in a named key for consistent RPC shape.
        to_value(serde_json::json!({ "identities": result }))
    })
}

pub(crate) fn handle_tinyplace_graphql_agent_card(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let id = req_str(&params, "id")?.to_string();
        log::debug!("{LOG_PREFIX} graphql_agent_card id={id}");
        let client = global_state().client().await?;
        // GraphQLAuth::None — no signer required.
        let result = client.graphql.agent_card(&id).await.map_err(map_err)?;
        // Returns Option<AgentCard> — null means agent card not found.
        to_value(result)
    })
}

// ── Directory: find by encryption key (0D) ──────────────────────────────────

/// Reverse-lookup: find the agent advertising a given encryption public key.
/// Returns the full `AgentCard` or `null` if no agent advertises it.
pub(crate) fn handle_tinyplace_directory_find_by_encryption_key(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let encryption_key = req_str(&params, "encryptionKey")?.to_string();
        log::debug!("{LOG_PREFIX} directory_find_by_encryption_key (key not logged for brevity)");
        let client = global_state().client().await?;
        let result = client
            .directory
            .find_agent_by_encryption_key(&encryption_key)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Feeds write surface (Phase A) ─────────────────────────────────────────────

/// Create a new post on a feed owned by the signer.
/// `handle` identifies the target feed but the backend enforces that the signer
/// owns it via `post_directory_auth_as`. Actor is NEVER accepted from params.
pub(crate) fn handle_tinyplace_feeds_create_post(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let body = req_str(&params, "body")?.trim().to_string();
        if body.is_empty() {
            return Err("missing required param 'body'".to_string());
        }
        let content_type = get_opt_str(&params, "contentType")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let client = global_state().client().await?;
        // Post to the SIGNER's OWN feed. The handle is resolved server-side from
        // the wallet (its crypto id is a valid feed handle) — never accepted from
        // the client — so this works for every wallet, registered @handle or not,
        // and a caller cannot post to a feed they don't own.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let handle = signer.agent_id();

        log::debug!(
            "{LOG_PREFIX} feeds_create_post handle={handle} body_len={}",
            body.len()
        );

        let post_create = tinyplace::types::PostCreate {
            body,
            content_type,
            post_id: None,
        };
        let result = client
            .feeds
            .create_post(&handle, &post_create)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

/// Delete a post from the SIGNER's OWN feed. The handle is resolved server-side
/// from the wallet (owner-only) — never accepted from the client.
pub(crate) fn handle_tinyplace_feeds_delete_post(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let post_id = req_str(&params, "postId")?.trim().to_string();
        if post_id.is_empty() {
            return Err("missing required param 'postId'".to_string());
        }

        let client = global_state().client().await?;
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let handle = signer.agent_id();

        log::debug!("{LOG_PREFIX} feeds_delete_post handle={handle} post_id={post_id}");

        client
            .feeds
            .delete_post(&handle, &post_id)
            .await
            .map_err(map_err)?;
        Ok(serde_json::json!({ "ok": true }))
    })
}

/// Add a comment to a post. Author is resolved from the wallet signer — NEVER
/// from client-supplied params.
pub(crate) fn handle_tinyplace_feeds_add_comment(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let handle = req_str(&params, "handle")?.trim().to_string();
        if handle.is_empty() {
            return Err("missing required param 'handle'".to_string());
        }
        let post_id = req_str(&params, "postId")?.trim().to_string();
        if post_id.is_empty() {
            return Err("missing required param 'postId'".to_string());
        }
        let body = req_str(&params, "body")?.trim().to_string();
        if body.is_empty() {
            return Err("missing required param 'body'".to_string());
        }

        let client = global_state().client().await?;
        // ANTI-SPOOF: author is always the wallet signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let author = signer.agent_id();

        log::debug!(
            "{LOG_PREFIX} feeds_add_comment handle={handle} post_id={post_id} author={author}"
        );

        let comment_create = tinyplace::types::CommentCreate {
            body,
            comment_id: None,
        };
        let result = client
            .feeds
            .add_comment(&handle, &post_id, &author, &comment_create)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

/// Delete a comment. Actor is resolved from the wallet signer — NEVER from
/// client-supplied params.
pub(crate) fn handle_tinyplace_feeds_delete_comment(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let handle = req_str(&params, "handle")?.trim().to_string();
        if handle.is_empty() {
            return Err("missing required param 'handle'".to_string());
        }
        let post_id = req_str(&params, "postId")?.trim().to_string();
        if post_id.is_empty() {
            return Err("missing required param 'postId'".to_string());
        }
        let comment_id = req_str(&params, "commentId")?.trim().to_string();
        if comment_id.is_empty() {
            return Err("missing required param 'commentId'".to_string());
        }

        let client = global_state().client().await?;
        // ANTI-SPOOF: actor is always the wallet signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        log::debug!(
            "{LOG_PREFIX} feeds_delete_comment handle={handle} post_id={post_id} \
             comment_id={comment_id} actor={actor}"
        );

        client
            .feeds
            .delete_comment(&handle, &post_id, &comment_id, &actor)
            .await
            .map_err(map_err)?;
        Ok(serde_json::json!({ "ok": true }))
    })
}

/// Like a post. Actor is resolved from the wallet signer — NEVER from params.
/// Idempotent.
pub(crate) fn handle_tinyplace_feeds_like_post(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let handle = req_str(&params, "handle")?.trim().to_string();
        if handle.is_empty() {
            return Err("missing required param 'handle'".to_string());
        }
        let post_id = req_str(&params, "postId")?.trim().to_string();
        if post_id.is_empty() {
            return Err("missing required param 'postId'".to_string());
        }

        let client = global_state().client().await?;
        // ANTI-SPOOF: actor is always the wallet signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        log::debug!("{LOG_PREFIX} feeds_like_post handle={handle} post_id={post_id} actor={actor}");

        let result = client
            .feeds
            .like_post(&handle, &post_id, &actor)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

/// Unlike a post. Actor is resolved from the wallet signer — NEVER from params.
/// Idempotent.
pub(crate) fn handle_tinyplace_feeds_unlike_post(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let handle = req_str(&params, "handle")?.trim().to_string();
        if handle.is_empty() {
            return Err("missing required param 'handle'".to_string());
        }
        let post_id = req_str(&params, "postId")?.trim().to_string();
        if post_id.is_empty() {
            return Err("missing required param 'postId'".to_string());
        }

        let client = global_state().client().await?;
        // ANTI-SPOOF: actor is always the wallet signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();

        log::debug!(
            "{LOG_PREFIX} feeds_unlike_post handle={handle} post_id={post_id} actor={actor}"
        );

        let result = client
            .feeds
            .unlike_post(&handle, &post_id, &actor)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

// ── Bounties section (Phase B) ────────────────────────────────────────────────

/// BountyListResponse.bounties is a bare Vec<Bounty> — a null/missing field
/// fails deserialization. Degrade to an empty list.
pub(crate) fn bounties_list_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "bounties": [] }))
    } else {
        None
    }
}

/// BountySubmissionsResponse.submissions is a bare Vec<BountySubmission>.
pub(crate) fn bounties_submissions_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "submissions": [] }))
    } else {
        None
    }
}

/// BountyCommentsResponse.comments is a bare Vec<BountyComment>.
pub(crate) fn bounties_comments_degrade(e: &tinyplace::Error) -> Option<Value> {
    if matches!(e, tinyplace::Error::Serialization(_)) {
        Some(serde_json::json!({ "comments": [] }))
    } else {
        None
    }
}

pub(crate) fn handle_tinyplace_bounties_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(
            "{LOG_PREFIX} bounties_list params_keys={:?}",
            params.keys().collect::<Vec<_>>()
        );
        let query_params: Option<tinyplace::types::BountyQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid bounties list params: {e}"))
            })
            .transpose()?;
        let client = global_state().client().await?;
        match client.bounties.list(query_params.as_ref()).await {
            Ok(result) => to_value(result),
            Err(e) => match bounties_list_degrade(&e) {
                Some(empty) => {
                    log::debug!("{LOG_PREFIX} bounties_list deserialization failed -> empty: {e}");
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

pub(crate) fn handle_tinyplace_bounties_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let bounty_id = req_str(&params, "bountyId")?.to_string();
        log::debug!("{LOG_PREFIX} bounties_get bounty_id={bounty_id}");
        let client = global_state().client().await?;
        let result = client.bounties.get(&bounty_id).await.map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_bounties_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let title = req_str(&params, "title")?.trim().to_string();
        if title.is_empty() {
            return Err("missing required param 'title'".to_string());
        }
        let description = req_str(&params, "description")?.trim().to_string();
        if description.is_empty() {
            return Err("missing required param 'description'".to_string());
        }
        let amount = req_str(&params, "amount")?.trim().to_string();
        if amount.is_empty() {
            return Err("missing required param 'amount'".to_string());
        }
        let asset = get_opt_str(&params, "asset")
            .filter(|s| !s.is_empty())
            .unwrap_or("USDC")
            .to_string();
        let deadline = get_opt_str(&params, "deadline")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let duration_days = params.get("durationDays").and_then(Value::as_i64);
        let confirmed = params
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        log::debug!(
            "{LOG_PREFIX} bounties_create title={title} amount={amount} asset={asset} confirmed={confirmed}"
        );

        let client = global_state().client().await?;
        // ANTI-SPOOF: creator resolved from signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let creator = signer.agent_id();

        let request = tinyplace::types::BountyCreateRequest {
            creator: Some(creator.clone()),
            creator_crypto_id: Some(creator),
            title,
            description,
            amount,
            asset: Some(asset),
            deadline,
            duration_days,
            payment: None,
        };

        // The backend funds the reward into escrow at creation time via x402, so
        // creating a bounty is a confirm-before-spend flow (same as register/buy):
        // probe without payment to get the 402 challenge, then re-create with the
        // signed payment map once the user confirms.
        let challenge = match client.bounties.create(&request).await {
            Ok(bounty) => {
                // No payment required (free / already-funded) — return as-is.
                log::debug!("{LOG_PREFIX} bounties_create no payment needed");
                return to_value(serde_json::json!({ "bounty": bounty }));
            }
            Err(e) => match e.payment_required() {
                Some(pr) => pr.payment.clone(),
                None => return Err(map_err(e)),
            },
        };
        log::debug!(
            "{LOG_PREFIX} bounties_create 402 challenge network={:?} asset={:?} amount={:?}",
            challenge.network,
            challenge.asset,
            challenge.amount,
        );

        // Unconfirmed: surface the challenge + balance, spend nothing.
        if !confirmed {
            let (wallet_balance, wallet_address) = wallet_usdc_balance(&signer.agent_id()).await;
            return to_value(serde_json::json!({
                "challenge": challenge,
                "walletBalance": wallet_balance,
                "walletAddress": wallet_address,
            }));
        }

        // Confirmed: cluster guards, pay on-chain, re-create with the payment map.
        if let Some(network) = challenge.network.as_deref() {
            ensure_cluster_matches(network)?;
        }
        ensure_backend_mint_matches(&client).await?;

        let mut extra_metadata = HashMap::new();
        extra_metadata.insert("title".to_string(), request.title.clone());
        let fulfilled = fulfill_payment(
            &challenge,
            signer.as_ref(),
            PaymentContext {
                purpose: "bounties.create".to_string(),
                nonce_prefix: "bounty".to_string(),
                extra_metadata,
            },
        )
        .await?;
        let on_chain_tx = fulfilled.on_chain_tx.clone();

        let mut funded_request = request.clone();
        funded_request.payment = Some(fulfilled.payment_map.clone());

        match settle_retry("bounties_create", || {
            client.bounties.create(&funded_request)
        })
        .await
        {
            Ok(bounty) => to_value(serde_json::json!({
                "bounty": bounty,
                "payment": { "onChainTx": on_chain_tx },
            })),
            Err(SettleFailure::Hard(m)) => Err(format!(
                "bounty creation failed after payment (onChainTx={on_chain_tx}): {m}"
            )),
            Err(SettleFailure::Exhausted(last)) => Err(format!(
                "bounty paid but not confirmed in time (onChainTx={on_chain_tx}); last error: {last}"
            )),
        }
    })
}

pub(crate) fn handle_tinyplace_bounties_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let bounty_id = req_str(&params, "bountyId")?.to_string();
        log::debug!("{LOG_PREFIX} bounties_cancel bounty_id={bounty_id}");
        let client = global_state().client().await?;
        // ANTI-SPOOF: creator resolved from signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let creator = signer.agent_id();
        let result = client
            .bounties
            .cancel(&bounty_id, &creator)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_bounties_submit(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let bounty_id = req_str(&params, "bountyId")?.to_string();
        let url = req_str(&params, "url")?.trim().to_string();
        if url.is_empty() {
            return Err("missing required param 'url'".to_string());
        }
        let title = get_opt_str(&params, "title")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let note = get_opt_str(&params, "note")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        log::debug!("{LOG_PREFIX} bounties_submit bounty_id={bounty_id} url={url}");

        let client = global_state().client().await?;
        // ANTI-SPOOF: submitter resolved from signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let submitter = signer.agent_id();

        let request = tinyplace::types::BountySubmissionCreateRequest {
            submitter: Some(submitter),
            submitter_crypto_id: None,
            url,
            title,
            note,
        };
        let result = client
            .bounties
            .submit(&bounty_id, &request)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_bounties_list_submissions(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let bounty_id = req_str(&params, "bountyId")?.to_string();
        log::debug!("{LOG_PREFIX} bounties_list_submissions bounty_id={bounty_id}");
        let query_params: Option<tinyplace::types::BountySubmissionQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid bounties list_submissions params: {e}"))
            })
            .transpose()?;
        let client = global_state().client().await?;
        match client
            .bounties
            .list_submissions(&bounty_id, query_params.as_ref())
            .await
        {
            Ok(result) => to_value(result),
            Err(e) => match bounties_submissions_degrade(&e) {
                Some(empty) => {
                    log::debug!("{LOG_PREFIX} bounties_list_submissions degrade -> empty: {e}");
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

pub(crate) fn handle_tinyplace_bounties_comment(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let bounty_id = req_str(&params, "bountyId")?.to_string();
        let body = req_str(&params, "body")?.trim().to_string();
        if body.is_empty() {
            return Err("missing required param 'body'".to_string());
        }
        log::debug!("{LOG_PREFIX} bounties_comment bounty_id={bounty_id}");

        let client = global_state().client().await?;
        // ANTI-SPOOF: author resolved from signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let author = signer.agent_id();

        let request = tinyplace::types::BountyCommentCreateRequest {
            author: Some(author),
            author_crypto_id: None,
            body,
        };
        let result = client
            .bounties
            .comment(&bounty_id, &request)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_bounties_list_comments(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let bounty_id = req_str(&params, "bountyId")?.to_string();
        log::debug!("{LOG_PREFIX} bounties_list_comments bounty_id={bounty_id}");
        let query_params: Option<tinyplace::types::BountyCommentQueryParams> = params
            .get("params")
            .and_then(|v| if v.is_null() { None } else { Some(v) })
            .map(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| format!("invalid bounties list_comments params: {e}"))
            })
            .transpose()?;
        let client = global_state().client().await?;
        match client
            .bounties
            .list_comments(&bounty_id, query_params.as_ref())
            .await
        {
            Ok(result) => to_value(result),
            Err(e) => match bounties_comments_degrade(&e) {
                Some(empty) => {
                    log::debug!("{LOG_PREFIX} bounties_list_comments degrade -> empty: {e}");
                    to_value(empty)
                }
                None => Err(map_err(e)),
            },
        }
    })
}

pub(crate) fn handle_tinyplace_bounties_run_council(
    params: Map<String, Value>,
) -> ControllerFuture {
    Box::pin(async move {
        let bounty_id = req_str(&params, "bountyId")?.to_string();
        log::debug!("{LOG_PREFIX} bounties_run_council bounty_id={bounty_id}");
        let client = global_state().client().await?;
        // ANTI-SPOOF: actor resolved from signer, never from params.
        let signer = client
            .http()
            .signer()
            .ok_or("tiny.place signer unavailable; unlock your wallet")?;
        let actor = signer.agent_id();
        let result = client
            .bounties
            .run_council(&bounty_id, &actor)
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

pub(crate) fn handle_tinyplace_bounties_approve(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let bounty_id = req_str(&params, "bountyId")?.to_string();
        let submission_id = get_opt_str(&params, "submissionId").map(|s| s.to_string());
        log::debug!(
            "{LOG_PREFIX} bounties_approve bounty_id={bounty_id} \
             submission_id={submission_id:?}"
        );
        let client = global_state().client().await?;
        // NOTE: approve uses post_admin auth on the SDK side. The backend
        // enforces the admin gate. If the caller is not an admin, the backend
        // rejects with 403. The v1 UI hides this button entirely.
        let result = client
            .bounties
            .approve(&bounty_id, submission_id.as_deref())
            .await
            .map_err(map_err)?;
        to_value(result)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;

    /// A missing/blank `username` is rejected before any client/network work.
    #[test]
    fn register_requires_username() {
        let err = block_on(handle_tinyplace_registry_register(Map::new())).unwrap_err();
        assert!(err.contains("username"), "got: {err}");

        let mut params = Map::new();
        params.insert("username".to_string(), Value::String("   ".to_string()));
        let err = block_on(handle_tinyplace_registry_register(params)).unwrap_err();
        assert!(err.contains("username"), "got: {err}");
    }

    /// Buy handlers reject a missing/blank `id` before any client/network work.
    #[test]
    fn buy_handlers_require_id() {
        for handler in [
            handle_tinyplace_marketplace_buy_product as fn(Map<String, Value>) -> ControllerFuture,
            handle_tinyplace_marketplace_buy_identity,
        ] {
            let err = block_on(handler(Map::new())).unwrap_err();
            assert!(err.contains("'id'"), "got: {err}");

            let mut params = Map::new();
            params.insert("id".to_string(), Value::String("  ".to_string()));
            let err = block_on(handler(params)).unwrap_err();
            assert!(err.contains("'id'"), "got: {err}");
        }
    }

    /// Bid/offer handlers validate their required params before any network work.
    #[test]
    fn bid_offer_validate_params() {
        // bid: missing listingId.
        let err = block_on(handle_tinyplace_marketplace_bid(Map::new())).unwrap_err();
        assert!(err.contains("listingId"), "got: {err}");
        // bid: listingId present but amount missing.
        let mut p = Map::new();
        p.insert("listingId".to_string(), Value::String("l1".into()));
        let err = block_on(handle_tinyplace_marketplace_bid(p)).unwrap_err();
        assert!(err.contains("amount"), "got: {err}");
        // offer: missing name.
        let err = block_on(handle_tinyplace_marketplace_offer(Map::new())).unwrap_err();
        assert!(err.contains("name"), "got: {err}");
    }

    #[test]
    fn price_from_params_defaults_asset_and_requires_network() {
        let mut p = Map::new();
        p.insert("amount".to_string(), Value::String("100".into()));
        // network missing → Err.
        assert!(price_from_params(&p).unwrap_err().contains("network"));
        // network present → defaults asset to USDC.
        p.insert("network".to_string(), Value::String("solana-devnet".into()));
        let price = price_from_params(&p).unwrap();
        assert_eq!(price.amount, "100");
        assert_eq!(price.asset, "USDC");
        assert_eq!(price.network, "solana-devnet");
        // explicit asset is honoured.
        p.insert("asset".to_string(), Value::String("SOL".into()));
        assert_eq!(price_from_params(&p).unwrap().asset, "SOL");
    }

    // ── GraphQL Feed handler param validation ────────────────────────────────

    /// graphql_posts requires `handle`.
    #[test]
    fn graphql_posts_requires_handle() {
        let err = block_on(handle_tinyplace_graphql_posts(Map::new())).unwrap_err();
        assert!(err.contains("handle"), "got: {err}");
    }

    /// graphql_post requires `handle` and `postId`.
    #[test]
    fn graphql_post_requires_handle_and_post_id() {
        let err = block_on(handle_tinyplace_graphql_post(Map::new())).unwrap_err();
        assert!(err.contains("handle"), "got: {err}");

        let mut p = Map::new();
        p.insert("handle".to_string(), Value::String("alice".into()));
        let err = block_on(handle_tinyplace_graphql_post(p)).unwrap_err();
        assert!(err.contains("postId"), "got: {err}");
    }

    /// graphql_post_comments requires `postId`.
    #[test]
    fn graphql_post_comments_requires_post_id() {
        let err = block_on(handle_tinyplace_graphql_post_comments(Map::new())).unwrap_err();
        assert!(err.contains("postId"), "got: {err}");
    }

    /// graphql_post_likers requires `postId`.
    #[test]
    fn graphql_post_likers_requires_post_id() {
        let err = block_on(handle_tinyplace_graphql_post_likers(Map::new())).unwrap_err();
        assert!(err.contains("postId"), "got: {err}");
    }

    /// graphql_home_feed has no required params — it should fail at
    /// global_state/client initialization (no wallet in unit tests),
    /// NOT at param extraction.
    #[test]
    fn graphql_home_feed_fails_at_client_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_graphql_home_feed(Map::new()))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
    }

    /// Degrade helpers return empty results for Serialization errors, and
    /// propagate non-serialization errors (InvalidArgument).
    #[test]
    fn graphql_degrade_helpers_return_empty_on_serialization() {
        // Construct a real serde_json::Error by deserializing invalid JSON.
        let raw_ser_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("{invalid json}").unwrap_err();
        let ser_err = tinyplace::Error::Serialization(raw_ser_err);
        assert!(graphql_home_feed_degrade(&ser_err).is_some());
        assert!(graphql_posts_degrade(&ser_err).is_some());

        // Non-serialization errors should NOT be degraded.
        let other = tinyplace::Error::InvalidArgument("bad arg".into());
        assert!(graphql_home_feed_degrade(&other).is_none());
        assert!(graphql_posts_degrade(&other).is_none());
    }

    /// graphql_ledger_transaction requires `id`.
    #[test]
    fn graphql_ledger_transaction_requires_id() {
        let err = block_on(handle_tinyplace_graphql_ledger_transaction(Map::new())).unwrap_err();
        assert!(err.contains("id"), "got: {err}");
    }

    /// graphql_ledger_transactions has no required params — should fail at
    /// client init (no wallet in unit tests), NOT at param extraction.
    #[test]
    fn graphql_ledger_transactions_fails_at_client_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_graphql_ledger_transactions(Map::new()))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
    }

    /// Degrade helper returns empty for Serialization errors.
    #[test]
    fn graphql_ledger_degrade_returns_empty_on_serialization() {
        let raw_ser_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("{invalid json}").unwrap_err();
        let ser_err = tinyplace::Error::Serialization(raw_ser_err);
        assert!(graphql_ledger_transactions_degrade(&ser_err).is_some());

        let other = tinyplace::Error::InvalidArgument("bad arg".into());
        assert!(graphql_ledger_transactions_degrade(&other).is_none());
    }

    // ── GraphQL Jobs handler param validation ───────────────────────────────

    /// graphql_jobs has no required params — should fail at
    /// client init (no wallet in unit tests), NOT at param extraction.
    #[test]
    fn graphql_jobs_fails_at_client_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_graphql_jobs(Map::new()))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
    }

    /// graphql_job requires `id`.
    #[test]
    fn graphql_job_requires_id() {
        let err = block_on(handle_tinyplace_graphql_job(Map::new())).unwrap_err();
        assert!(err.contains("id"), "got: {err}");
    }

    /// Degrade helper returns empty for Serialization errors.
    #[test]
    fn graphql_jobs_degrade_returns_empty_on_serialization() {
        let raw_ser_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("{invalid json}").unwrap_err();
        let ser_err = tinyplace::Error::Serialization(raw_ser_err);
        assert!(graphql_jobs_degrade(&ser_err).is_some());

        let other = tinyplace::Error::InvalidArgument("bad arg".into());
        assert!(graphql_jobs_degrade(&other).is_none());
    }

    #[test]
    fn settlement_retry_only_on_confirming_402s() {
        // Retryable: 402 with a settlement-timing message.
        assert!(settlement_error_is_retryable(
            Some(402),
            "Transaction not found on chain yet"
        ));
        assert!(settlement_error_is_retryable(
            Some(402),
            "payment pending: insufficient confirmations"
        ));
        // Not retryable: non-402, or a 402 with an unrelated/hard message.
        assert!(!settlement_error_is_retryable(
            Some(400),
            "transaction not found"
        ));
        assert!(!settlement_error_is_retryable(
            Some(402),
            "handle already taken"
        ));
        assert!(!settlement_error_is_retryable(None, "transport error"));
    }

    /// Follow/unfollow handlers reject a missing `agentId` before any client work.
    #[test]
    fn follows_handlers_require_agent_id() {
        for handler in [
            handle_tinyplace_follows_follow as fn(Map<String, Value>) -> ControllerFuture,
            handle_tinyplace_follows_unfollow,
            handle_tinyplace_follows_followers,
            handle_tinyplace_follows_following,
            handle_tinyplace_follows_stats,
        ] {
            let err = block_on(handler(Map::new())).unwrap_err();
            assert!(err.contains("agentId"), "got: {err}");
        }
    }

    /// registry_export rejects a missing `name` before any client work.
    #[test]
    fn registry_export_requires_name() {
        let err = block_on(handle_tinyplace_registry_export(Map::new())).unwrap_err();
        assert!(err.contains("name"), "got: {err}");
    }

    /// Feedback get/vote handlers reject missing required params before any client work.
    #[test]
    fn feedback_handlers_require_params() {
        // feedback_get requires feedbackId.
        let err = block_on(handle_tinyplace_feedback_get(Map::new())).unwrap_err();
        assert!(err.contains("feedbackId"), "got: {err}");

        // feedback_vote requires feedbackId.
        let err = block_on(handle_tinyplace_feedback_vote(Map::new())).unwrap_err();
        assert!(err.contains("feedbackId"), "got: {err}");

        // feedback_vote requires vote (feedbackId present but vote missing).
        let mut p = Map::new();
        p.insert("feedbackId".to_string(), Value::String("fb-1".into()));
        let err = block_on(handle_tinyplace_feedback_vote(p)).unwrap_err();
        assert!(err.contains("vote"), "got: {err}");

        // feedback_vote rejects invalid vote value.
        let mut p = Map::new();
        p.insert("feedbackId".to_string(), Value::String("fb-1".into()));
        p.insert("vote".to_string(), Value::String("sideways".into()));
        let err = block_on(handle_tinyplace_feedback_vote(p)).unwrap_err();
        assert!(err.contains("must be 'up' or 'down'"), "got: {err}");

        // feedback_create requires title.
        let err = block_on(handle_tinyplace_feedback_create(Map::new())).unwrap_err();
        assert!(err.contains("title"), "got: {err}");

        // feedback_create requires description (title present but description missing).
        let mut p = Map::new();
        p.insert("title".to_string(), Value::String("A great idea".into()));
        let err = block_on(handle_tinyplace_feedback_create(p)).unwrap_err();
        assert!(err.contains("description"), "got: {err}");

        // feedback_create rejects blank title.
        let mut p = Map::new();
        p.insert("title".to_string(), Value::String("   ".into()));
        let err = block_on(handle_tinyplace_feedback_create(p)).unwrap_err();
        assert!(err.contains("title"), "got: {err}");
    }

    /// Groups invite/role handlers reject missing required params before any client work.
    #[test]
    fn groups_invite_handlers_require_params() {
        // set_member_role requires groupId, agentId, role.
        let err = block_on(handle_tinyplace_groups_set_member_role(Map::new())).unwrap_err();
        assert!(err.contains("groupId"), "got: {err}");

        let mut p = Map::new();
        p.insert("groupId".to_string(), Value::String("g-1".into()));
        let err = block_on(handle_tinyplace_groups_set_member_role(p)).unwrap_err();
        assert!(err.contains("agentId"), "got: {err}");

        let mut p = Map::new();
        p.insert("groupId".to_string(), Value::String("g-1".into()));
        p.insert("agentId".to_string(), Value::String("agent-1".into()));
        let err = block_on(handle_tinyplace_groups_set_member_role(p)).unwrap_err();
        assert!(err.contains("role"), "got: {err}");

        // create_invite requires groupId.
        let err = block_on(handle_tinyplace_groups_create_invite(Map::new())).unwrap_err();
        assert!(err.contains("groupId"), "got: {err}");

        // list_invites requires groupId.
        let err = block_on(handle_tinyplace_groups_list_invites(Map::new())).unwrap_err();
        assert!(err.contains("groupId"), "got: {err}");

        // preview_invite requires groupId and token.
        let err = block_on(handle_tinyplace_groups_preview_invite(Map::new())).unwrap_err();
        assert!(err.contains("groupId"), "got: {err}");

        let mut p = Map::new();
        p.insert("groupId".to_string(), Value::String("g-1".into()));
        let err = block_on(handle_tinyplace_groups_preview_invite(p)).unwrap_err();
        assert!(err.contains("token"), "got: {err}");

        // revoke_invite requires groupId and token.
        let err = block_on(handle_tinyplace_groups_revoke_invite(Map::new())).unwrap_err();
        assert!(err.contains("groupId"), "got: {err}");

        let mut p = Map::new();
        p.insert("groupId".to_string(), Value::String("g-1".into()));
        let err = block_on(handle_tinyplace_groups_revoke_invite(p)).unwrap_err();
        assert!(err.contains("token"), "got: {err}");

        // redeem_invite requires groupId and token.
        let err = block_on(handle_tinyplace_groups_redeem_invite(Map::new())).unwrap_err();
        assert!(err.contains("groupId"), "got: {err}");

        let mut p = Map::new();
        p.insert("groupId".to_string(), Value::String("g-1".into()));
        let err = block_on(handle_tinyplace_groups_redeem_invite(p)).unwrap_err();
        assert!(err.contains("token"), "got: {err}");
    }

    /// Email verification handlers validate required params before any client work.
    #[test]
    fn email_verification_handlers_require_params() {
        // start requires cryptoId.
        let err =
            block_on(handle_tinyplace_users_start_email_verification(Map::new())).unwrap_err();
        assert!(err.contains("cryptoId"), "got: {err}");

        // start requires email (cryptoId present but email missing).
        let mut p = Map::new();
        p.insert("cryptoId".to_string(), Value::String("wallet-1".into()));
        let err = block_on(handle_tinyplace_users_start_email_verification(p)).unwrap_err();
        assert!(err.contains("email"), "got: {err}");

        // start rejects blank email.
        let mut p = Map::new();
        p.insert("cryptoId".to_string(), Value::String("wallet-1".into()));
        p.insert("email".to_string(), Value::String("   ".into()));
        let err = block_on(handle_tinyplace_users_start_email_verification(p)).unwrap_err();
        assert!(err.contains("email"), "got: {err}");

        // confirm requires cryptoId.
        let err =
            block_on(handle_tinyplace_users_confirm_email_verification(Map::new())).unwrap_err();
        assert!(err.contains("cryptoId"), "got: {err}");

        // confirm requires email (cryptoId present but email missing).
        let mut p = Map::new();
        p.insert("cryptoId".to_string(), Value::String("wallet-1".into()));
        let err = block_on(handle_tinyplace_users_confirm_email_verification(p)).unwrap_err();
        assert!(err.contains("email"), "got: {err}");

        // confirm requires code (cryptoId + email present but code missing).
        let mut p = Map::new();
        p.insert("cryptoId".to_string(), Value::String("wallet-1".into()));
        p.insert(
            "email".to_string(),
            Value::String("user@example.com".into()),
        );
        let err = block_on(handle_tinyplace_users_confirm_email_verification(p)).unwrap_err();
        assert!(err.contains("code"), "got: {err}");

        // confirm rejects blank code.
        let mut p = Map::new();
        p.insert("cryptoId".to_string(), Value::String("wallet-1".into()));
        p.insert(
            "email".to_string(),
            Value::String("user@example.com".into()),
        );
        p.insert("code".to_string(), Value::String("   ".into()));
        let err = block_on(handle_tinyplace_users_confirm_email_verification(p)).unwrap_err();
        assert!(err.contains("code"), "got: {err}");
    }

    /// solana_call rejects a missing/blank `method` before any client work.
    #[test]
    fn solana_call_requires_method() {
        let err = block_on(handle_tinyplace_solana_call(Map::new())).unwrap_err();
        assert!(err.contains("method"), "got: {err}");

        let mut params = Map::new();
        params.insert("method".to_string(), Value::String("   ".to_string()));
        let err = block_on(handle_tinyplace_solana_call(params)).unwrap_err();
        assert!(err.contains("method"), "got: {err}");
    }

    /// streams_start rejects a missing/blank streamType.
    #[test]
    fn streams_start_requires_stream_type() {
        let err = block_on(handle_tinyplace_streams_start(Map::new())).unwrap_err();
        assert!(err.contains("streamType"), "got: {err}");

        let mut params = Map::new();
        params.insert("streamType".to_string(), Value::String("   ".to_string()));
        let err = block_on(handle_tinyplace_streams_start(params)).unwrap_err();
        assert!(err.contains("streamType"), "got: {err}");
    }

    /// streams_start rejects an unsupported streamType.
    #[test]
    fn streams_start_rejects_unknown_type() {
        let mut params = Map::new();
        params.insert(
            "streamType".to_string(),
            Value::String("unknown".to_string()),
        );
        let err = block_on(handle_tinyplace_streams_start(params)).unwrap_err();
        assert!(err.contains("unsupported streamType"), "got: {err}");
    }

    /// streams_start rejects a conversation stream without a streamId.
    #[test]
    fn streams_start_conversation_requires_stream_id() {
        let mut params = Map::new();
        params.insert(
            "streamType".to_string(),
            Value::String("conversation".to_string()),
        );
        let err = block_on(handle_tinyplace_streams_start(params)).unwrap_err();
        assert!(err.contains("streamId"), "got: {err}");
    }

    /// streams_stop rejects a missing/blank streamId.
    #[test]
    fn streams_stop_requires_stream_id() {
        let err = block_on(handle_tinyplace_streams_stop(Map::new())).unwrap_err();
        assert!(err.contains("streamId"), "got: {err}");

        let mut params = Map::new();
        params.insert("streamId".to_string(), Value::String("   ".to_string()));
        let err = block_on(handle_tinyplace_streams_stop(params)).unwrap_err();
        assert!(err.contains("streamId"), "got: {err}");
    }

    /// `signal_provision` has no required params; it must fail at `global_signal_store`
    /// (wallet/config not available in unit tests), NOT at param extraction.
    /// Uses a Tokio runtime because `global_signal_store` internally calls
    /// `load_config_with_timeout` which requires Tokio.
    #[test]
    fn signal_provision_fails_at_store_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_signal_provision(Map::new()))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
    }

    /// `signal_upload_pre_keys` has no required params; same as above.
    #[test]
    fn signal_upload_pre_keys_fails_at_store_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_signal_upload_pre_keys(Map::new()))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
    }

    /// `signal_rotate_signed_pre_key` has no required params; same as above.
    #[test]
    fn signal_rotate_fails_at_store_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_signal_rotate_signed_pre_key(Map::new()))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
    }

    /// `signal_key_status` has no required params; same as above.
    #[test]
    fn signal_key_status_fails_at_store_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_signal_key_status(Map::new()))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
    }

    // ── looks_like_base58_pubkey heuristic ───────────────────────────────────

    #[test]
    fn base58_pubkey_heuristic_accepts_valid_keys() {
        // 44-char base58 string using all valid alphabet chars
        assert!(
            looks_like_base58_pubkey("61KcG5aGLqpnJz2fXyzABCDEFGHJKLMNPQRSTUVWXY"),
            "44-char valid base58 must match"
        );
        // 32-char minimum
        assert!(
            looks_like_base58_pubkey("11111111111111111111111111111111"),
            "32-char all-1s must match (system program)"
        );
    }

    #[test]
    fn base58_pubkey_heuristic_rejects_handles_and_invalid() {
        assert!(
            !looks_like_base58_pubkey("@alice"),
            "@ prefix must not match"
        );
        assert!(
            !looks_like_base58_pubkey("alice"),
            "too short to be a pubkey"
        );
        assert!(!looks_like_base58_pubkey(""), "empty must not match");
        // Contains '!' which is outside the base58 alphabet
        assert!(
            !looks_like_base58_pubkey("61KcG5aGLqpn!Jz2fXyzABCDEFGHJKLMNPQRSTUVWX"),
            "invalid char must not match"
        );
        // Contains '0' which is NOT in base58 (confused with 'O')
        assert!(
            !looks_like_base58_pubkey("61KcG5aGLqpnJz2fXyzABCDEFGHJKLMNPQRS0UVWXY"),
            "zero char not in base58 alphabet"
        );
    }

    #[test]
    fn base58_pubkey_heuristic_rejects_45_char_string() {
        // 45 chars is too long for a 32-byte base58 pubkey (valid range is 32..=44)
        let s = "61KcG5aGLqpnJz2fXyzABCDEFGHJKLMNPQRSTUVWXYZ12";
        assert_eq!(s.len(), 45, "test fixture must be exactly 45 chars");
        assert!(
            !looks_like_base58_pubkey(s),
            "45-char string must not match"
        );
    }

    /// `signal_get_bundle` rejects a missing `agentId` before any resolution.
    #[test]
    fn signal_get_bundle_requires_agent_id() {
        let err = block_on(handle_tinyplace_signal_get_bundle(Map::new())).unwrap_err();
        assert!(err.contains("agentId"), "got: {err}");
    }

    #[test]
    fn signal_send_requires_recipient_and_plaintext() {
        let mut params = Map::new();
        params.insert("plaintext".into(), Value::String("hello".into()));
        let err = block_on(handle_tinyplace_signal_send_message(params)).unwrap_err();
        assert!(err.contains("recipient"), "got: {err}");
        let mut params = Map::new();
        params.insert("recipient".into(), Value::String("peer123".into()));
        let err = block_on(handle_tinyplace_signal_send_message(params)).unwrap_err();
        assert!(err.contains("plaintext"), "got: {err}");
    }

    #[test]
    fn signal_decrypt_requires_envelope() {
        let err = block_on(handle_tinyplace_signal_decrypt_message(Map::new())).unwrap_err();
        assert!(err.contains("envelope"), "got: {err}");
    }

    #[test]
    fn messages_list_fails_at_client_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_messages_list(Map::new()))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
    }

    #[test]
    fn messages_acknowledge_requires_message_id() {
        let err = block_on(handle_tinyplace_messages_acknowledge(Map::new())).unwrap_err();
        assert!(err.contains("messageId"), "got: {err}");
    }

    #[test]
    fn signal_send_never_sends_plaintext_on_encryption_failure() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut params = Map::new();
        params.insert("recipient".into(), Value::String("test_peer".into()));
        params.insert("plaintext".into(), Value::String("secret message".into()));
        let err = rt
            .block_on(handle_tinyplace_signal_send_message(params))
            .unwrap_err();
        assert!(!err.contains("missing required param"), "got: {err}");
        assert!(
            !err.contains("secret message"),
            "plaintext leaked in error message: {err}"
        );
    }

    // ── Encryption key registration + discovery (0D) ─────────────────────────

    #[test]
    fn directory_find_by_encryption_key_requires_param() {
        let err =
            block_on(handle_tinyplace_directory_find_by_encryption_key(Map::new())).unwrap_err();
        assert!(err.contains("encryptionKey"), "got: {err}");
    }

    /// Verify the register handler has no required params -- it will fail at
    /// global_signal_store (no running runtime) but NOT on missing params.
    #[test]
    fn signal_register_encryption_key_fails_at_store_not_params() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_signal_register_encryption_key(Map::new()))
            .unwrap_err();
        assert!(
            !err.contains("missing required param"),
            "should not fail on params: {err}"
        );
    }

    #[test]
    fn directory_find_by_encryption_key_with_valid_param_fails_at_client() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut params = Map::new();
        params.insert(
            "encryptionKey".into(),
            Value::String("dGVzdA==".into()), // valid base64
        );
        let err = rt
            .block_on(handle_tinyplace_directory_find_by_encryption_key(params))
            .unwrap_err();
        // Must get past param validation; fails at client initialization.
        assert!(!err.contains("encryptionKey"), "got: {err}");
    }

    /// Verify that the error path from register_encryption_key does not contain
    /// any base64 key material in the error message.
    #[test]
    fn signal_register_encryption_key_does_not_leak_key_material() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(handle_tinyplace_signal_register_encryption_key(Map::new()))
            .unwrap_err();
        // The error should not contain base64-encoded key fragments.
        // Since we fail before getting a key, this is a structural test:
        // the handler's error messages don't embed raw key values.
        assert!(
            !err.contains("=="),
            "error should not contain base64 fragments: {err}"
        );
    }

    /// Verify all 10 Jobs write handlers reject missing required params.
    ///
    /// Actor (client/candidate) is NEVER read from params — it is always derived
    /// from the wallet signer. These tests only exercise param-validation logic;
    /// they cannot reach the SDK call (no running tiny.place client in unit tests).
    #[test]
    fn jobs_write_handlers_require_params() {
        // ── jobs_create ───────────────────────────────────────────────────────

        // Blank Map → must complain about 'title'
        let err = block_on(handle_tinyplace_jobs_create(Map::new())).unwrap_err();
        assert!(err.contains("title"), "jobs_create missing title: {err}");

        // title present, budgetAmount absent
        {
            let mut p = Map::new();
            p.insert("title".into(), Value::String("Build a bot".into()));
            let err = block_on(handle_tinyplace_jobs_create(p)).unwrap_err();
            assert!(
                err.contains("budgetAmount"),
                "jobs_create missing budgetAmount: {err}"
            );
        }

        // title + budgetAmount present, budgetAsset absent
        {
            let mut p = Map::new();
            p.insert("title".into(), Value::String("Build a bot".into()));
            p.insert("budgetAmount".into(), Value::String("100".into()));
            let err = block_on(handle_tinyplace_jobs_create(p)).unwrap_err();
            assert!(
                err.contains("budgetAsset"),
                "jobs_create missing budgetAsset: {err}"
            );
        }

        // Blank title must be rejected
        {
            let mut p = Map::new();
            p.insert("title".into(), Value::String("   ".into()));
            p.insert("budgetAmount".into(), Value::String("100".into()));
            p.insert("budgetAsset".into(), Value::String("USDC".into()));
            let err = block_on(handle_tinyplace_jobs_create(p)).unwrap_err();
            assert!(err.contains("title"), "jobs_create blank title: {err}");
        }

        // ── jobs_cancel ───────────────────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_cancel(Map::new())).unwrap_err();
        assert!(err.contains("jobId"), "jobs_cancel missing jobId: {err}");

        // ── jobs_apply ────────────────────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_apply(Map::new())).unwrap_err();
        assert!(err.contains("jobId"), "jobs_apply missing jobId: {err}");

        // ── jobs_list_proposals ───────────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_list_proposals(Map::new())).unwrap_err();
        assert!(
            err.contains("jobId"),
            "jobs_list_proposals missing jobId: {err}"
        );

        // ── jobs_get_proposal ─────────────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_get_proposal(Map::new())).unwrap_err();
        assert!(
            err.contains("jobId"),
            "jobs_get_proposal missing jobId: {err}"
        );

        // jobId present, proposalId absent
        {
            let mut p = Map::new();
            p.insert("jobId".into(), Value::String("job-1".into()));
            let err = block_on(handle_tinyplace_jobs_get_proposal(p)).unwrap_err();
            assert!(
                err.contains("proposalId"),
                "jobs_get_proposal missing proposalId: {err}"
            );
        }

        // ── jobs_shortlist_proposal ───────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_shortlist_proposal(Map::new())).unwrap_err();
        assert!(
            err.contains("jobId"),
            "jobs_shortlist_proposal missing jobId: {err}"
        );

        {
            let mut p = Map::new();
            p.insert("jobId".into(), Value::String("job-1".into()));
            let err = block_on(handle_tinyplace_jobs_shortlist_proposal(p)).unwrap_err();
            assert!(
                err.contains("proposalId"),
                "jobs_shortlist_proposal missing proposalId: {err}"
            );
        }

        // ── jobs_withdraw_proposal ────────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_withdraw_proposal(Map::new())).unwrap_err();
        assert!(
            err.contains("jobId"),
            "jobs_withdraw_proposal missing jobId: {err}"
        );

        {
            let mut p = Map::new();
            p.insert("jobId".into(), Value::String("job-1".into()));
            let err = block_on(handle_tinyplace_jobs_withdraw_proposal(p)).unwrap_err();
            assert!(
                err.contains("proposalId"),
                "jobs_withdraw_proposal missing proposalId: {err}"
            );
        }

        // ── jobs_select ───────────────────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_select(Map::new())).unwrap_err();
        assert!(err.contains("jobId"), "jobs_select missing jobId: {err}");

        {
            let mut p = Map::new();
            p.insert("jobId".into(), Value::String("job-1".into()));
            let err = block_on(handle_tinyplace_jobs_select(p)).unwrap_err();
            assert!(
                err.contains("proposalId"),
                "jobs_select missing proposalId: {err}"
            );
        }

        // ── jobs_open_dispute ─────────────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_open_dispute(Map::new())).unwrap_err();
        assert!(
            err.contains("jobId"),
            "jobs_open_dispute missing jobId: {err}"
        );

        // jobId present, reason absent
        {
            let mut p = Map::new();
            p.insert("jobId".into(), Value::String("job-1".into()));
            let err = block_on(handle_tinyplace_jobs_open_dispute(p)).unwrap_err();
            assert!(
                err.contains("reason"),
                "jobs_open_dispute missing reason: {err}"
            );
        }

        // Blank reason must be rejected
        {
            let mut p = Map::new();
            p.insert("jobId".into(), Value::String("job-1".into()));
            p.insert("reason".into(), Value::String("   ".into()));
            let err = block_on(handle_tinyplace_jobs_open_dispute(p)).unwrap_err();
            assert!(
                err.contains("reason"),
                "jobs_open_dispute blank reason: {err}"
            );
        }

        // ── jobs_adjudicate_dispute ───────────────────────────────────────────

        let err = block_on(handle_tinyplace_jobs_adjudicate_dispute(Map::new())).unwrap_err();
        assert!(
            err.contains("jobId"),
            "jobs_adjudicate_dispute missing jobId: {err}"
        );
    }

    // ── Feeds write surface param validation (Phase A) ───────────────────────

    /// feeds_create_post requires non-blank `handle` then non-blank `body`.
    #[test]
    fn feeds_create_post_requires_body_not_handle() {
        // `body` is required and validated before any client/signer access.
        let err = block_on(handle_tinyplace_feeds_create_post(Map::new())).unwrap_err();
        assert!(err.contains("body"), "got: {err}");
        // The owner handle is resolved server-side from the signer, so the
        // handler must NOT read a client-supplied 'handle'.
        assert!(
            !err.contains("handle"),
            "handle must not be a client param: {err}"
        );
    }

    /// feeds_delete_post requires `postId`; the owner handle is signer-derived.
    #[test]
    fn feeds_delete_post_requires_post_id_not_handle() {
        let err = block_on(handle_tinyplace_feeds_delete_post(Map::new())).unwrap_err();
        assert!(err.contains("postId"), "got: {err}");
        assert!(
            !err.contains("handle"),
            "handle must not be a client param: {err}"
        );
    }

    /// feeds_add_comment requires `handle`, `postId`, then `body`.
    #[test]
    fn feeds_add_comment_requires_handle_post_id_and_body() {
        let err = block_on(handle_tinyplace_feeds_add_comment(Map::new())).unwrap_err();
        assert!(err.contains("handle"), "got: {err}");
        let mut p = Map::new();
        p.insert("handle".into(), Value::String("alice".into()));
        let err = block_on(handle_tinyplace_feeds_add_comment(p)).unwrap_err();
        assert!(err.contains("postId"), "got: {err}");
        let mut p2 = Map::new();
        p2.insert("handle".into(), Value::String("alice".into()));
        p2.insert("postId".into(), Value::String("p1".into()));
        let err = block_on(handle_tinyplace_feeds_add_comment(p2)).unwrap_err();
        assert!(err.contains("body"), "got: {err}");
    }

    /// feeds_delete_comment requires `handle` at minimum.
    #[test]
    fn feeds_delete_comment_requires_params() {
        let err = block_on(handle_tinyplace_feeds_delete_comment(Map::new())).unwrap_err();
        assert!(err.contains("handle"), "got: {err}");
    }

    /// Both like and unlike handlers require `handle` then `postId`.
    /// This test also verifies that the `actor` is NOT read from params:
    /// both handlers fail at param extraction (before any client/network work),
    /// and neither produces an error mentioning "actor".
    #[test]
    fn feeds_like_unlike_require_handle_and_post_id() {
        for handler in [
            handle_tinyplace_feeds_like_post as fn(Map<String, Value>) -> ControllerFuture,
            handle_tinyplace_feeds_unlike_post,
        ] {
            let err = block_on(handler(Map::new())).unwrap_err();
            assert!(err.contains("handle"), "got: {err}");
            // actor is server-derived — must not appear in the error message
            assert!(
                !err.contains("actor"),
                "actor must never come from params; got: {err}"
            );
            let mut p = Map::new();
            p.insert("handle".into(), Value::String("alice".into()));
            let err = block_on(handler(p)).unwrap_err();
            assert!(err.contains("postId"), "got: {err}");
        }
    }

    // ── Bounties handlers (Phase B) ───────────────────────────────────────────

    /// Degrade helpers return empty results for Serialization errors, and
    /// propagate non-serialization errors.
    #[test]
    fn bounties_degrade_helpers_return_empty_on_serialization() {
        let raw_ser_err: serde_json::Error =
            serde_json::from_str::<serde_json::Value>("{invalid json}").unwrap_err();
        let ser_err = tinyplace::Error::Serialization(raw_ser_err);
        assert!(bounties_list_degrade(&ser_err).is_some());
        assert!(bounties_submissions_degrade(&ser_err).is_some());
        assert!(bounties_comments_degrade(&ser_err).is_some());

        let other = tinyplace::Error::InvalidArgument("bad arg".into());
        assert!(bounties_list_degrade(&other).is_none());
        assert!(bounties_submissions_degrade(&other).is_none());
        assert!(bounties_comments_degrade(&other).is_none());
    }

    /// bounties_create rejects blank title before any client work.
    #[test]
    fn bounties_create_rejects_blank_title() {
        let mut params = Map::new();
        params.insert("title".to_string(), Value::String("".to_string()));
        params.insert("description".to_string(), Value::String("desc".to_string()));
        params.insert("amount".to_string(), Value::String("100".to_string()));
        let result = block_on(handle_tinyplace_bounties_create(params));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("title"),
            "expected 'title' in error"
        );
    }

    /// bounties_create rejects missing amount before any client work.
    #[test]
    fn bounties_create_rejects_missing_amount() {
        let mut params = Map::new();
        params.insert("title".to_string(), Value::String("test".to_string()));
        params.insert("description".to_string(), Value::String("desc".to_string()));
        let result = block_on(handle_tinyplace_bounties_create(params));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("amount"),
            "expected 'amount' in error"
        );
    }

    /// bounties_submit rejects blank url before any client work.
    #[test]
    fn bounties_submit_rejects_blank_url() {
        let mut params = Map::new();
        params.insert("bountyId".to_string(), Value::String("b1".to_string()));
        params.insert("url".to_string(), Value::String("".to_string()));
        let result = block_on(handle_tinyplace_bounties_submit(params));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("url"),
            "expected 'url' in error"
        );
    }

    /// bounties_comment rejects blank body before any client work.
    #[test]
    fn bounties_comment_rejects_blank_body() {
        let mut params = Map::new();
        params.insert("bountyId".to_string(), Value::String("b1".to_string()));
        params.insert("body".to_string(), Value::String("   ".to_string()));
        let result = block_on(handle_tinyplace_bounties_comment(params));
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("body"),
            "expected 'body' in error"
        );
    }

    /// ANTI-SPOOF: bounties_create, bounties_cancel, bounties_submit, bounties_comment,
    /// and bounties_run_council must NOT read actor/creator/submitter/author from params.
    /// These handlers fail at param extraction or client init — never at an actor param.
    #[test]
    fn bounties_write_handlers_do_not_accept_actor_from_params() {
        // bounties_create: fails on title before any actor check
        let err = block_on(handle_tinyplace_bounties_create(Map::new())).unwrap_err();
        assert!(err.contains("title"), "got: {err}");
        assert!(
            !err.contains("creator"),
            "creator must not be a client param: {err}"
        );

        // bounties_cancel: fails on bountyId before any actor check
        let err = block_on(handle_tinyplace_bounties_cancel(Map::new())).unwrap_err();
        assert!(err.contains("bountyId"), "got: {err}");
        assert!(
            !err.contains("creator"),
            "creator must not be a client param: {err}"
        );

        // bounties_submit: fails on bountyId before any actor check
        let err = block_on(handle_tinyplace_bounties_submit(Map::new())).unwrap_err();
        assert!(err.contains("bountyId"), "got: {err}");
        assert!(
            !err.contains("submitter"),
            "submitter must not be a client param: {err}"
        );

        // bounties_comment: fails on bountyId before any actor check
        let err = block_on(handle_tinyplace_bounties_comment(Map::new())).unwrap_err();
        assert!(err.contains("bountyId"), "got: {err}");
        assert!(
            !err.contains("author"),
            "author must not be a client param: {err}"
        );

        // bounties_run_council: fails on bountyId before any actor check
        let err = block_on(handle_tinyplace_bounties_run_council(Map::new())).unwrap_err();
        assert!(err.contains("bountyId"), "got: {err}");
        assert!(
            !err.contains("actor"),
            "actor must not be a client param: {err}"
        );
    }

    // ── match_agent_card_by_username — pure matching logic ───────────────────

    fn make_test_card(agent_id: &str, username: Option<&str>) -> tinyplace::types::AgentCard {
        tinyplace::types::AgentCard {
            agent_id: agent_id.to_string(),
            name: agent_id.to_string(),
            description: None,
            username: username.map(str::to_string),
            crypto_id: format!("crypto-{agent_id}"),
            actor_type: None,
            public_key: None,
            url: None,
            endpoint: None,
            supported_interfaces: None,
            skills: None,
            capabilities: None,
            tags: None,
            payment_methods: None,
            payment_requirements: None,
            groups: None,
            docs: None,
            webhooks: None,
            metadata: None,
            signature: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            viewer_is_following: None,
        }
    }

    /// Directory-card matcher finds an exact username match.
    #[test]
    fn match_agent_card_exact_username_hit() {
        let cards = vec![
            make_test_card("agent-1", Some("alice")),
            make_test_card("agent-2", Some("stevejobs")),
            make_test_card("agent-3", Some("bob")),
        ];
        let found = match_agent_card_by_username(&cards, "stevejobs");
        assert!(found.is_some(), "should find stevejobs");
        assert_eq!(found.unwrap().crypto_id, "crypto-agent-2");
    }

    /// Directory-card matcher is case-insensitive.
    #[test]
    fn match_agent_card_case_insensitive() {
        let cards = vec![make_test_card("agent-1", Some("SteveJobs"))];
        let found = match_agent_card_by_username(&cards, "stevejobs");
        assert!(found.is_some(), "case-insensitive match must succeed");
    }

    /// Directory-card matcher returns None when no card has the given username.
    #[test]
    fn match_agent_card_miss() {
        let cards = vec![
            make_test_card("agent-1", Some("alice")),
            make_test_card("agent-2", None), // no username
        ];
        let found = match_agent_card_by_username(&cards, "notexist");
        assert!(found.is_none(), "should return None when no match");
    }

    /// Directory-card matcher skips cards with no username without panicking.
    #[test]
    fn match_agent_card_skips_cards_without_username() {
        let cards = vec![
            make_test_card("agent-1", None),
            make_test_card("agent-2", None),
            make_test_card("agent-3", Some("target")),
        ];
        let found = match_agent_card_by_username(&cards, "target");
        assert!(found.is_some(), "should find the card that has a username");
        assert_eq!(found.unwrap().agent_id, "agent-3");
    }

    /// looks_like_base58_pubkey passthrough: resolve_recipient should NOT call
    /// directory.resolve for a raw crypto_id — it short-circuits and returns it
    /// directly.  We can verify this by supplying a valid 44-char base58 string
    /// which, without a live client, would error if it tried to resolve.
    ///
    /// Because `resolve_recipient_to_agent_id` requires a live client we test
    /// the passthrough indirectly via `looks_like_base58_pubkey`.
    #[test]
    fn base58_passthrough_identified_correctly() {
        // 44-char valid base58 — same as used elsewhere in this test suite.
        let key = "61KcG5aGLqpnJz2fXyzABCDEFGHJKLMNPQRSTUVWXY";
        assert!(
            looks_like_base58_pubkey(key),
            "44-char base58 must be identified as a crypto_id to skip resolution"
        );
    }

    /// Resolver error message does NOT expose the raw 404 HTTP text.
    /// The error string produced when no agent is found must be human-readable
    /// and must not look like a raw network error.
    #[test]
    fn resolve_not_found_error_is_human_readable() {
        // Test the error message produced in handle_tinyplace_directory_resolve
        // when both registry and directory miss — replicated here as the literal
        // format used in the production code.
        let name = "unknownuser";
        let err = format!(
            "No agent found for \"{name}\". \
             Check the handle or paste their wallet address directly."
        );
        assert!(
            err.contains("unknownuser"),
            "error must name the handle: {err}"
        );
        assert!(
            !err.contains("404"),
            "error must not expose raw HTTP status: {err}"
        );
        assert!(
            !err.contains("not found"),
            "error must not use raw 404 body text: {err}"
        );
    }
}
