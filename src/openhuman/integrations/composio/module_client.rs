//! Calling the connector module, or explaining why it is not there.
//!
//! Every Composio operation now lives in the `tinyconnectors` module, which is
//! loaded through [`crate::openhuman::modules`] — and that is behind the
//! `modules` feature. A build with gates off has no loader, so it has no
//! connectors either.
//!
//! The `#[cfg]` lives here rather than on each handler. Twelve handlers with a
//! feature gate each is twelve chances to gate one of them differently, and the
//! gates-off build only fails on whichever is compiled first. One switch, one
//! message, and the handlers read the same either way.
//!
//! # What the member names do *not* depend on
//!
//! [`methods`] comes from `tinyconnectors_bus`, an ordinary dependency with no
//! feature gate. A gates-off build can still name a member and match on the
//! contract; it just cannot call one.

pub use tinyconnectors_bus::names::methods;

/// Whether a member failure is the module refusing an operation the live route
/// does not offer.
///
/// The two routes are not equivalent — direct mode has no per-user toolkit
/// allowlist, and no webhook endpoint for triggers — and the module says so by
/// name rather than by returning an empty result. A caller that wants to render
/// its own answer for that case has to tell the refusal apart from a real
/// failure, and getting it backwards would show "no curated allowlist" over an
/// outage, leaving the user unaware their integration had broken.
///
/// Matched on the message because that is what crosses the bus: `TinyBus`
/// carries an error name and a string, so the structure of the module's error
/// is flattened by the time it arrives.
#[must_use]
pub fn is_unsupported_by_route(error: &str) -> bool {
    error.contains("is not available over the") && error.contains("route")
}

/// Keep the structured Composio classification at the start of the error.
///
/// TinyBus prefixes a member failure with the member name (`Execute: `) and,
/// since the module extraction, with its wire error name
/// (`ai.tinyhumans.tinybus.Error.<Kind>: `) — but the frontend parser
/// intentionally requires the classification at byte zero, so every transport
/// layer is peeled before checking. Errors that are not classified retain their
/// full context unchanged.
///
/// # Why the wire layer is peeled in a loop
///
/// It used to be peeled once, with a second `strip_prefix` afterwards as a
/// fallback for `Error.Failed: ` specifically. That fallback could not fire for
/// the shape its comment described: the single peel above it already consumed
/// `Error.Failed: `, so the branch was reachable only by a *doubly* wire-wrapped
/// error — which nothing documented, and which the fallback's own comment did
/// not claim to handle. Nesting depth is a property of the transport, not
/// something this function should hardcode a guess at, so it now strips as many
/// layers as are present. Output is unchanged for every shape the old form
/// handled, including the double wrap the fallback did catch; a triple or deeper
/// wrap, which the old form silently passed through unnormalised (and so
/// reported to the user as an unclassified outage), now normalises too.
fn normalize_error(member: &str, error: String) -> String {
    const CLASSIFIED: &str = "[composio:error:";
    const WIRE_ERROR_PREFIX: &str = "ai.tinyhumans.tinybus.Error.";
    if let Some(remainder) = error
        .strip_prefix(member)
        .and_then(|remainder| remainder.strip_prefix(": "))
    {
        // Peel each `ai.tinyhumans.tinybus.Error.<Kind>: ` layer. `<Kind>` is a
        // bare identifier, so the next `: ` ends it; anything shaped differently
        // stops the loop and is left alone. An embedded marker that does NOT
        // follow a wire prefix is never promoted — the `starts_with` below is
        // what decides, and it runs on the fully-peeled remainder only.
        let mut module_error = remainder;
        while let Some(tail) = module_error
            .strip_prefix(WIRE_ERROR_PREFIX)
            .and_then(|rest| rest.split_once(": ").map(|(_, tail)| tail))
        {
            module_error = tail;
        }
        if module_error.starts_with(CLASSIFIED) {
            return module_error.to_string();
        }
    }
    error
}

/// The message a gates-off build answers every connector call with.
#[cfg(not(feature = "modules"))]
const WITHOUT_MODULES: &str =
    "composio is unavailable in this build: connectors run in the `tinyconnectors` module, \
     which needs the `modules` feature";

/// Call one member with an argument and decode its reply.
///
/// # Errors
///
/// Returns the member's failure as the module rendered it, or an explanation
/// when this build has no module loader.
///
/// Note what is *not* an error: a Composio action the provider refused comes
/// back as a successful reply carrying `successful: false`. A caller that
/// checks only for `Err` will report a failed send as a success.
#[cfg(feature = "modules")]
pub async fn call<Request, Reply>(
    config: &crate::openhuman::config::Config,
    member: &str,
    request: Request,
) -> Result<Reply, String>
where
    Request: serde::Serialize + Send,
    Reply: serde::de::DeserializeOwned,
{
    crate::openhuman::modules::connectors::call(config, member, request)
        .await
        .map_err(|error| normalize_error(member, error))
}

/// Call a long-running member with a deadline sized for it (see
/// `modules::connectors::call_slow` for why `Sync` needs one).
#[cfg(feature = "modules")]
pub async fn call_slow<Request, Reply>(
    config: &crate::openhuman::config::Config,
    member: &str,
    request: Request,
) -> Result<Reply, String>
where
    Request: serde::Serialize + Send,
    Reply: serde::de::DeserializeOwned,
{
    crate::openhuman::modules::connectors::call_slow(config, member, request)
        .await
        .map_err(|error| normalize_error(member, error))
}

/// Call one member with an argument. Always fails without the `modules` feature.
///
/// # Errors
///
/// Always, explaining that this build has no module loader.
#[cfg(not(feature = "modules"))]
pub async fn call<Request, Reply>(
    _config: &crate::openhuman::config::Config,
    member: &str,
    _request: Request,
) -> Result<Reply, String>
where
    Request: serde::Serialize + Send,
    Reply: serde::de::DeserializeOwned,
{
    Err(format!("{member}: {WITHOUT_MODULES}"))
}

/// Call a long-running member. Always fails without the `modules` feature.
///
/// # Errors
///
/// Always, explaining that this build has no module loader.
#[cfg(not(feature = "modules"))]
pub async fn call_slow<Request, Reply>(
    _config: &crate::openhuman::config::Config,
    member: &str,
    _request: Request,
) -> Result<Reply, String>
where
    Request: serde::Serialize + Send,
    Reply: serde::de::DeserializeOwned,
{
    Err(format!("{member}: {WITHOUT_MODULES}"))
}

/// Give an already-serving connector module the route the current config
/// selects — for a signed-out user, that means dropping the credential it was
/// configured with while signed in — without loading a module that is not
/// serving. See `modules::connectors::reconcile_route_if_loaded`.
///
/// # Errors
///
/// As the module function: the module is serving but could not be reached or
/// refused the reconfiguration.
#[cfg(feature = "modules")]
pub async fn reconcile_route_if_loaded(
    config: &crate::openhuman::config::Config,
) -> Result<(), String> {
    crate::openhuman::modules::connectors::reconcile_route_if_loaded(config).await
}

/// Without the module loader there is no module holding a route. Always `Ok`.
#[cfg(not(feature = "modules"))]
pub async fn reconcile_route_if_loaded(
    _config: &crate::openhuman::config::Config,
) -> Result<(), String> {
    Ok(())
}

/// Call a member that takes no arguments.
///
/// # Errors
///
/// As [`call`].
#[cfg(feature = "modules")]
pub async fn call_bare<Reply: serde::de::DeserializeOwned>(
    config: &crate::openhuman::config::Config,
    member: &str,
) -> Result<Reply, String> {
    crate::openhuman::modules::connectors::call_bare(config, member)
        .await
        .map_err(|error| normalize_error(member, error))
}

/// Call a member that takes no arguments. Always fails without `modules`.
///
/// # Errors
///
/// Always, explaining that this build has no module loader.
#[cfg(not(feature = "modules"))]
pub async fn call_bare<Reply: serde::de::DeserializeOwned>(
    _config: &crate::openhuman::config::Config,
    member: &str,
) -> Result<Reply, String> {
    Err(format!("{member}: {WITHOUT_MODULES}"))
}

/// Serializes every test that reaches the connector module.
///
/// The module is loaded once per process and holds exactly one route, but each
/// test that exercises it stands up its own mock backend on its own port and
/// points the module at it. Run in parallel they reconfigure each other
/// mid-call, and the failure surfaces as a 404 from somebody else's mock —
/// which reads like a bug in the code under test rather than as a race.
///
/// This did not exist before the connector extraction, because each test built
/// its own client. Sharing one module instance is the price of the module
/// being real in these tests rather than mocked, and they are worth more for
/// it: they exercise the loader, the bus, and the module's own dispatch.
///
/// It lives here rather than in one test module because the tests that need it
/// are spread across `ops_tests` and the flow adapter's own suite, and two
/// locks would serialize neither against the other.
#[cfg(test)]
pub(crate) static MODULE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Take the module lock for the rest of the test.
#[cfg(test)]
pub(crate) async fn module_guard() -> tokio::sync::MutexGuard<'static, ()> {
    MODULE_LOCK.lock().await
}

#[cfg(test)]
#[path = "module_client_tests.rs"]
mod tests;
