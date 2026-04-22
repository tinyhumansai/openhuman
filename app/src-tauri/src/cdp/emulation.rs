//! CDP-based UA override — replaces the old `ua_spoof.js` init script.
//!
//! `Emulation.setUserAgentOverride` applies both at the network layer (UA
//! HTTP header + service workers) AND in the renderer (navigator.userAgent,
//! navigator.userAgentData via Client Hints). Tauri's `builder.user_agent()`
//! only covers the HTTP header; this lets us match full Chromium fingerprints
//! without injecting JavaScript.
//!
//! What this does NOT cover (vs the old ua_spoof.js):
//!   * `window.chrome = {...}` stub — pages gating on `window.chrome`
//!     presence will still fail on WKWebView. Rare in practice.
//!   * `delete window.safari` — Safari-specific API removal.
//!
//! If a provider regresses on either of those we'll revisit.

use serde_json::json;

use super::CdpConn;

/// Full UA metadata we present to the renderer. Matches the strings the old
/// `ua_spoof.js` produced so providers that fingerprinted on (UA, brands)
/// see the same values.
#[derive(Debug, Clone)]
pub struct UaSpec {
    pub user_agent: String,
    pub chrome_major: String,
    pub chrome_full: String,
    pub platform: String,
    pub platform_version: String,
    pub architecture: String,
    pub bitness: String,
    pub mobile: bool,
    pub accept_language: Option<String>,
}

impl UaSpec {
    /// TODO(maintenance): bump these strings when the headline Chrome
    /// version drifts far enough that fingerprinters flag us as stale.
    /// Fields to refresh in lock-step: `user_agent`, `chrome_major`,
    /// `chrome_full`, `platform_version`. Target cadence: quarterly, or
    /// whenever a provider starts rejecting the current UA. The previous
    /// JS shim (`ua_spoof.js`) shipped these same values so behaviour is
    /// preserved until we touch them.
    pub fn chrome_mac() -> Self {
        Self {
            user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/124.0.6367.118 Safari/537.36"
                .to_string(),
            chrome_major: "124".to_string(),
            chrome_full: "124.0.6367.118".to_string(),
            platform: "macOS".to_string(),
            platform_version: "14.0.0".to_string(),
            architecture: "x86".to_string(),
            bitness: "64".to_string(),
            mobile: false,
            accept_language: Some("en-US,en".to_string()),
        }
    }
}

/// Apply full UA override to an attached session. Safe to call repeatedly;
/// the browser keeps the latest override per target.
pub async fn set_user_agent_override(
    cdp: &mut CdpConn,
    session_id: &str,
    spec: &UaSpec,
) -> Result<(), String> {
    let metadata = json!({
        "brands": [
            { "brand": "Chromium", "version": spec.chrome_major },
            { "brand": "Google Chrome", "version": spec.chrome_major },
            { "brand": "Not-A.Brand", "version": "99" },
        ],
        "fullVersionList": [
            { "brand": "Chromium", "version": spec.chrome_full },
            { "brand": "Google Chrome", "version": spec.chrome_full },
            { "brand": "Not-A.Brand", "version": "99.0.0.0" },
        ],
        "platform": spec.platform,
        "platformVersion": spec.platform_version,
        "architecture": spec.architecture,
        "bitness": spec.bitness,
        "model": "",
        "mobile": spec.mobile,
        "wow64": false,
    });
    let mut params = json!({
        "userAgent": spec.user_agent,
        "userAgentMetadata": metadata,
    });
    if let Some(lang) = spec.accept_language.as_deref() {
        params["acceptLanguage"] = json!(lang);
    }
    cdp.call("Emulation.setUserAgentOverride", params, Some(session_id))
        .await?;
    Ok(())
}
