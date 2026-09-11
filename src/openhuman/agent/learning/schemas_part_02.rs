// ── list_facets ───────────────────────────────────────────────────────────────

fn handle_list_facets(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use tinymemory_api::provider::FacetState;

        tracing::debug!("[learning.list_facets] called");

        let class_filter = params
            .get("class")
            .and_then(Value::as_str)
            .map(str::to_string);

        // Reject an unknown class before touching the store, so a filter that
        // could never match a facet is surfaced as an error instead of an
        // empty result the caller cannot distinguish from "nothing learned".
        if let Some(cls) = &class_filter {
            crate::openhuman::agent::learning::cache::parse_facet_class_name(cls)?;
        }

        let cache = get_cache().await?;

        // list_all returns all states (active + provisional + candidate + dropped).
        let all = cache
            .list_all()
            .await
            .map_err(|e| format!("list_all failed: {e:#}"))?;

        let facets: Vec<serde_json::Value> = all
            .iter()
            .filter(|f| {
                // Expose Active and Provisional rows to the user.
                f.state == FacetState::Active || f.state == FacetState::Provisional
            })
            .filter(|f| {
                // Match on the class column when it is set — that stays
                // authoritative, so a row explicitly tagged with another class
                // can never match `cls` via its key prefix (the #6077 leak
                // stays closed). A row with no class column falls back to its
                // key prefix, which is where a canonical key like
                // `style/verbosity` carries the class the column omits — the
                // behaviour the dropped `|| key.starts_with(...)` arm provided
                // for legitimate classless rows.
                match &class_filter {
                    Some(cls) => {
                        f.class.as_deref() == Some(cls.as_str())
                            || (f.class.is_none() && f.key.starts_with(&format!("{cls}/")))
                    }
                    None => true,
                }
            })
            .map(facet_to_json)
            .collect();

        let count = facets.len();
        let log = vec![format!(
            "learning.list_facets: returned {count} facets (class_filter={:?})",
            class_filter
        )];

        let payload = serde_json::json!({ "facets": facets, "count": count });
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

// ── get_facet ─────────────────────────────────────────────────────────────────

fn handle_get_facet(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // These refusals are reachable from a dispatched call, not only from a
        // direct one. An *absent* `class`/`key` is refused earlier by
        // `core::all::validate_params`, in its own wording — but an explicit
        // `{"class": null}` passes that gate (the required check tests key
        // presence, and its type check returns early on null), so on that path
        // this is the only guard. See the `validate_params` docs (#6073).
        let class_str = params
            .get("class")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `class`".to_string())?
            .to_string();
        let key_suffix = params
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `key`".to_string())?
            .to_string();

        crate::openhuman::agent::learning::cache::parse_facet_class_name(&class_str)?;

        let fk = full_key(&class_str, &key_suffix);
        tracing::debug!("[learning.get_facet] key={fk}");

        let cache = get_cache().await?;
        let facet = cache
            .get(&fk)
            .await
            .map_err(|e| format!("get failed: {e:#}"))?;

        let (found, facet_val) = match &facet {
            Some(f) => (true, facet_to_json(f)),
            None => (false, serde_json::Value::Null),
        };

        let log = vec![format!("learning.get_facet: key={fk} found={found}")];
        let payload = serde_json::json!({ "facet": facet_val, "found": found });
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

// ── update_facet ──────────────────────────────────────────────────────────────

fn handle_update_facet(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use tinymemory_api::provider::UserState;

        let class_str = params
            .get("class")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `class`".to_string())?
            .to_string();
        let key_suffix = params
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `key`".to_string())?
            .to_string();
        let new_value = params
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `value`".to_string())?
            .to_string();

        crate::openhuman::agent::learning::cache::parse_facet_class_name(&class_str)?;

        let fk = full_key(&class_str, &key_suffix);
        tracing::debug!("[learning.update_facet] key={fk} value={new_value}");

        let cache = get_cache().await?;

        let mut facet = cache
            .get(&fk)
            .await
            .map_err(|e| format!("get failed: {e:#}"))?
            .ok_or_else(|| format!("facet not found: {fk}"))?;

        // Update value and pin so this survives future rebuilds.
        facet.value = new_value.clone();
        facet.user_state = UserState::Pinned;

        cache
            .upsert(&facet)
            .await
            .map_err(|e| format!("upsert failed: {e:#}"))?;

        let updated = cache
            .get(&fk)
            .await
            .map_err(|e| format!("re-read failed: {e:#}"))?
            .ok_or_else(|| "facet disappeared after upsert".to_string())?;

        let log = vec![format!(
            "learning.update_facet: key={fk} new_value={new_value} user_state=pinned"
        )];
        let payload = serde_json::json!({ "facet": facet_to_json(&updated) });
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

// ── pin_facet ─────────────────────────────────────────────────────────────────

fn handle_pin_facet(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use tinymemory_api::provider::UserState;

        let class_str = params
            .get("class")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `class`".to_string())?
            .to_string();
        let key_suffix = params
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `key`".to_string())?
            .to_string();

        crate::openhuman::agent::learning::cache::parse_facet_class_name(&class_str)?;

        let fk = full_key(&class_str, &key_suffix);
        tracing::debug!("[learning.pin_facet] key={fk}");

        let cache = get_cache().await?;
        let updated = cache
            .set_user_state(&fk, UserState::Pinned)
            .await
            .map_err(|e| format!("set_user_state failed: {e:#}"))?;

        if !updated {
            return Err(format!("facet not found: {fk}"));
        }

        let facet = cache
            .get(&fk)
            .await
            .map_err(|e| format!("re-read failed: {e:#}"))?
            .ok_or_else(|| "facet disappeared after update".to_string())?;

        let log = vec![format!("learning.pin_facet: key={fk} user_state=pinned")];
        let payload = serde_json::json!({ "facet": facet_to_json(&facet) });
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

// ── unpin_facet ───────────────────────────────────────────────────────────────

fn handle_unpin_facet(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use tinymemory_api::provider::UserState;

        let class_str = params
            .get("class")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `class`".to_string())?
            .to_string();
        let key_suffix = params
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `key`".to_string())?
            .to_string();

        crate::openhuman::agent::learning::cache::parse_facet_class_name(&class_str)?;

        let fk = full_key(&class_str, &key_suffix);
        tracing::debug!("[learning.unpin_facet] key={fk}");

        let cache = get_cache().await?;
        let updated = cache
            .set_user_state(&fk, UserState::Auto)
            .await
            .map_err(|e| format!("set_user_state failed: {e:#}"))?;

        if !updated {
            return Err(format!("facet not found: {fk}"));
        }

        let facet = cache
            .get(&fk)
            .await
            .map_err(|e| format!("re-read failed: {e:#}"))?
            .ok_or_else(|| "facet disappeared after update".to_string())?;

        let log = vec![format!("learning.unpin_facet: key={fk} user_state=auto")];
        let payload = serde_json::json!({ "facet": facet_to_json(&facet) });
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

// ── forget_facet ──────────────────────────────────────────────────────────────

/// The log line `learning.forget_facet` emits, given whether a row was actually
/// written.
///
/// Split out so the claim can be unit-tested without a cache: the defect this
/// replaces built the "state=dropped user_state=forgotten" line unconditionally,
/// *before* the read told it whether there was anything to drop, so an absent key
/// produced a log asserting a state change that never happened (#6108).
fn forget_facet_log(full_key: &str, dropped: bool) -> Vec<String> {
    vec![if dropped {
        format!("learning.forget_facet: key={full_key} state=dropped user_state=forgotten")
    } else {
        format!("learning.forget_facet: key={full_key} not present — no change")
    }]
}

fn handle_forget_facet(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        use tinymemory_api::provider::{FacetState, UserState};

        let class_str = params
            .get("class")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `class`".to_string())?
            .to_string();
        let key_suffix = params
            .get("key")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing required `key`".to_string())?
            .to_string();

        crate::openhuman::agent::learning::cache::parse_facet_class_name(&class_str)?;

        let fk = full_key(&class_str, &key_suffix);
        tracing::debug!("[learning.forget_facet] key={fk}");

        let cache = get_cache().await?;

        let facet_before = cache
            .get(&fk)
            .await
            .map_err(|e| format!("get failed: {e:#}"))?;

        // Absent is not an error: forgetting is idempotent, and erroring would
        // leak whether the facet existed — the wrong direction for a privacy
        // operation. The agent tool (`tools.rs`) has always behaved this way;
        // what was wrong here was the *log*, built before the branch was known,
        // so a typo'd key produced a line claiming a row had been dropped.
        let (facet_json, dropped) = if let Some(mut f) = facet_before {
            // Mark Forgotten + Dropped so it doesn't resurface.
            f.user_state = UserState::Forgotten;
            f.state = FacetState::Dropped;
            cache
                .upsert(&f)
                .await
                .map_err(|e| format!("upsert failed: {e:#}"))?;
            let updated = cache
                .get(&fk)
                .await
                .map_err(|e| format!("re-read failed: {e:#}"))?
                .unwrap_or(f);
            (facet_to_json(&updated), true)
        } else {
            (serde_json::Value::Null, false)
        };

        let log = forget_facet_log(&fk, dropped);
        let payload = serde_json::json!({ "facet": facet_json });
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}

// ── reset_cache ───────────────────────────────────────────────────────────────

fn handle_reset_cache(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        tracing::debug!("[learning.reset_cache] called");

        let cache = get_cache().await?;

        let (deleted, pinned_preserved) =
            crate::openhuman::agent::learning::cache::reset_non_pinned(&cache)
                .await
                .map_err(|e| format!("reset_cache failed: {e:#}"))?;

        tracing::info!(
            "[learning.reset_cache] deleted={deleted} pinned_preserved={pinned_preserved}"
        );

        let log = vec![format!(
            "learning.reset_cache: deleted={deleted} pinned_preserved={pinned_preserved}"
        )];
        let payload = serde_json::json!({
            "deleted": deleted,
            "pinned_preserved": pinned_preserved,
        });
        RpcOutcome::new(payload, log).into_cli_compatible_json()
    })
}
