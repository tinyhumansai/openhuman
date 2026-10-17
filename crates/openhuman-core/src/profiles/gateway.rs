//! How a gateway request picks the context it runs under.
//!
//! The gateway authenticates users and calls the core with the service bearer
//! plus, for work on a user's behalf, the user's id:
//!
//! ```text
//! Authorization: Bearer <service token>
//! X-OpenHuman-User: <gateway user id>
//! X-OpenHuman-User-Sig: t=<unix secs>,v1=<hex hmac-sha256(service token, "<t>.<user id>")>
//! ```
//!
//! No user header means the operator plane. With one, the request runs under
//! that user's profile — which must already be provisioned — and nothing else.
//! The signature, required unless the operator turns it off, binds the user id
//! to the bearer holder and a ±60 s window: a gateway that forwards a client's
//! headers by mistake cannot be talked into acting as another user.
//!
//! The core never verifies the user's own credential; the gateway did that.
//!
//! A profile another node hosts (its lease is live there) is refused with
//! `409`: the refusal carries the holder ([`HeldBy`]) so the gateway can
//! route the user there, or retry after the lease runs out.

use std::sync::Arc;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::host::{self, Profile, ProfileHost};
use super::lease::OpenError;
use super::types::ProfileId;

/// The gateway's user header.
pub const USER_HEADER: &str = "x-openhuman-user";
/// The gateway's signature over the user header.
pub const USER_SIG_HEADER: &str = "x-openhuman-user-sig";
/// How far a signature's timestamp may be from the core's clock.
pub const SIGNATURE_WINDOW_SECS: u64 = 60;

type HmacSha256 = Hmac<Sha256>;

fn mac(secret: &str, user_id: &str, ts: u64) -> HmacSha256 {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(format!("{ts}.{user_id}").as_bytes());
    mac
}

/// The `X-OpenHuman-User-Sig` value for `user_id` at `ts` (unix seconds).
pub fn sign(secret: &str, user_id: &str, ts: u64) -> String {
    let tag = mac(secret, user_id, ts).finalize().into_bytes();
    format!("t={ts},v1={}", hex::encode(tag))
}

/// Check a signature header for `user_id` against `now` (unix seconds).
pub fn verify(secret: &str, user_id: &str, header: &str, now: u64) -> Result<(), String> {
    let mut ts = None;
    let mut tag = None;
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", value)) => ts = value.parse::<u64>().ok(),
            Some(("v1", value)) => tag = hex::decode(value).ok(),
            _ => {}
        }
    }
    let (Some(ts), Some(tag)) = (ts, tag) else {
        return Err("malformed user signature".to_string());
    };
    if ts.abs_diff(now) > SIGNATURE_WINDOW_SECS {
        return Err("user signature is outside the allowed clock window".to_string());
    }
    mac(secret, user_id, ts)
        .verify_slice(&tag)
        .map_err(|_| "user signature does not match".to_string())
}

/// The `error` code of a refusal for a profile another node hosts.
pub const PROFILE_HELD: &str = "profile_held";

/// The response header naming the node that hosts a refused profile.
pub const PROFILE_OWNER_HEADER: &str = "x-openhuman-profile-owner";

/// The node that hosts a profile this one was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldBy {
    /// The holder's node id.
    pub owner: String,
    /// Where the holder can be reached, when it advertises an endpoint.
    pub endpoint: Option<String>,
    /// A polling hint in milliseconds: the time until its lease lapses unless
    /// renewed, capped at 60 seconds (a lease that never expires, such as a
    /// file lock, reports the cap). Retry after this, not necessarily after
    /// expiry.
    pub retry_after_ms: u64,
}

/// Why a gateway request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRefusal {
    /// The HTTP status to answer with.
    pub status: u16,
    pub message: String,
    /// Set on a `409`: who hosts the profile instead.
    pub held_by: Option<HeldBy>,
}

impl GatewayRefusal {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            held_by: None,
        }
    }

    /// The refusal for a profile's [`OpenError`] at `now_ms`.
    pub fn from_open_error(error: OpenError, now_ms: u64) -> Self {
        match error {
            OpenError::NotProvisioned(_) => Self::new(403, error.to_string()),
            OpenError::Full { .. } | OpenError::Storage(_) => Self::new(503, error.to_string()),
            OpenError::HeldElsewhere(record) => {
                log::debug!(
                    "[profiles][gateway] the profile is held by node={} epoch={}",
                    record.owner,
                    record.epoch
                );
                // A file-lock lease never expires (`u64::MAX`), so the raw
                // value is no retry hint (and overflows a JSON consumer's
                // safe integers); cap it.
                Self {
                    status: 409,
                    message: PROFILE_HELD.to_string(),
                    held_by: Some(HeldBy {
                        retry_after_ms: record.retry_after_ms(now_ms).min(MAX_RETRY_AFTER_MS),
                        owner: record.owner,
                        endpoint: record.endpoint,
                    }),
                }
            }
        }
    }
}

/// The longest `retry_after_ms` a `409` reports.
const MAX_RETRY_AFTER_MS: u64 = 60_000;

/// Where a gateway request runs.
#[derive(Debug)]
pub enum GatewayScope {
    /// No user header: the operator plane.
    Operator,
    /// One user's profile.
    User(Arc<Profile>),
}

/// Pick the scope for a request that already presented the service bearer.
///
/// `secret` is the service bearer; `now` is unix seconds.
pub async fn resolve_scope(
    user_id: Option<&str>,
    signature: Option<&str>,
    secret: &str,
    now: u64,
) -> Result<GatewayScope, GatewayRefusal> {
    let Some(user_id) = user_id else {
        return Ok(GatewayScope::Operator);
    };
    let host = host::host().ok_or_else(|| GatewayRefusal::new(503, "this core serves no users"))?;
    resolve_user_on(&host, user_id, signature, secret, now)
        .await
        .map(GatewayScope::User)
}

/// [`resolve_scope`] for a request that names `user_id`, on an explicit
/// `host` rather than the process's: the profile that serves that user, or
/// the refusal.
pub async fn resolve_user_on(
    host: &ProfileHost,
    user_id: &str,
    signature: Option<&str>,
    secret: &str,
    now: u64,
) -> Result<Arc<Profile>, GatewayRefusal> {
    let profile = ProfileId::for_user(user_id, host.saas().profile_ids)
        .map_err(|e| GatewayRefusal::new(400, e))?;
    if host.saas().require_user_signature {
        let signature = signature
            .ok_or_else(|| GatewayRefusal::new(401, format!("missing {USER_SIG_HEADER}")))?;
        verify(secret, user_id, signature, now).map_err(|e| {
            log::warn!("[profiles][gateway] refused a scoped request: {e}");
            GatewayRefusal::new(401, e)
        })?;
    }
    match host.open(&profile).await {
        Ok(state) => {
            log::debug!("[profiles][gateway] scoped request to an open profile");
            Ok(state)
        }
        Err(error) => Err(GatewayRefusal::from_open_error(
            error,
            super::lease::now_ms(),
        )),
    }
}

#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "gateway_proptest_tests.rs"]
mod proptest_tests;
