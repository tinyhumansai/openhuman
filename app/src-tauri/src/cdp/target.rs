//! CDP target discovery. Replaces the four hand-rolled copies in the
//! per-provider scanners.

use std::time::Duration;

use serde_json::{json, Value};

use super::{CdpConn, CDP_HOST, CDP_PORT};

#[derive(Debug, Clone)]
pub struct CdpTarget {
    pub id: String,
    pub kind: String,
    pub url: String,
    pub title: String,
}

/// Discover the browser-level WebSocket endpoint via `/json/version`. All
/// CDP sessions in the app tunnel through this one ws once `flatten: true`
/// is set on attach.
pub async fn browser_ws_url() -> Result<String, String> {
    let url = format!("http://{CDP_HOST}:{CDP_PORT}/json/version");
    let resp = reqwest::Client::builder()
        .user_agent("openhuman-cdp/1.0")
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("reqwest build: {e}"))?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    let v: Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;
    v.get("webSocketDebuggerUrl")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no webSocketDebuggerUrl in /json/version".to_string())
}

pub fn parse_targets(v: &Value) -> Vec<CdpTarget> {
    v.get("targetInfos")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    Some(CdpTarget {
                        id: t.get("targetId")?.as_str()?.to_string(),
                        kind: t.get("type")?.as_str()?.to_string(),
                        url: t
                            .get("url")
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string(),
                        title: t
                            .get("title")
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Full short-lived attach sequence: connect to the browser, find the
/// matching page target, attach with `flatten: true`. Caller gets a ready
/// CdpConn + session id for issuing commands. Caller MUST `detach_session`
/// (or drop the CdpConn entirely) when done so we don't leak sessions.
///
/// The predicate must match on per-account fragment + URL prefix so
/// multi-account webviews on the same origin resolve uniquely.
pub async fn connect_and_attach_matching<F>(pred: F) -> Result<(CdpConn, String), String>
where
    F: Fn(&CdpTarget) -> bool,
{
    let ws = browser_ws_url().await?;
    let mut cdp = CdpConn::open(&ws).await?;
    let target = find_page_target_where(&mut cdp, pred).await?;
    let attach = cdp
        .call(
            "Target.attachToTarget",
            json!({ "targetId": target.id, "flatten": true }),
            None,
        )
        .await?;
    let session = attach
        .get("sessionId")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "attach missing sessionId".to_string())?
        .to_string();
    Ok((cdp, session))
}

pub async fn detach_session(cdp: &mut CdpConn, session_id: &str) {
    let _ = cdp
        .call(
            "Target.detachFromTarget",
            json!({ "sessionId": session_id }),
            None,
        )
        .await;
}

/// Generalised variant — caller supplies the predicate (url-hash marker,
/// title marker, etc). Used by the per-account session opener, which matches
/// on `#openhuman-account-{id}` so multiple webviews on the same origin
/// don't collide.
pub async fn find_page_target_where<F>(cdp: &mut CdpConn, pred: F) -> Result<CdpTarget, String>
where
    F: Fn(&CdpTarget) -> bool,
{
    let targets_v = cdp.call("Target.getTargets", json!({}), None).await?;
    let targets = parse_targets(&targets_v);
    targets
        .into_iter()
        .find(|t| t.kind == "page" && pred(t))
        .ok_or_else(|| "no matching page target".to_string())
}
