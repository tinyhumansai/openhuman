// Sign-out invalidation for the two current-user caches.
//
// Both `CURRENT_USER_CACHE` and `CURRENT_USER_FAILURE` are keyed on
// `(api_base, token)`, so clearing them is not enough on its own:
// `fetch_current_user_cached` awaits the network between reading them and
// writing them, and a refresh already in flight when sign-out lands would
// re-publish exactly the state the sign-out removed. A generation counter,
// read when the token is read and re-checked under each cache's own lock,
// is what closes that window (#5758).

/// Bumped by [`forget_current_user_caches`]; read by `snapshot` before it
/// loads the session token, threaded into `fetch_current_user_cached`, and
/// checked again under the lock before either cache is written.
///
/// Counting rather than flagging keeps two overlapping sign-outs from
/// cancelling each other out.
static CURRENT_USER_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The failure write itself, taking the guard rather than the lock, so a caller
/// that must decide *under* the lock can do so without re-entering it.
fn record_current_user_failure_locked(
    failure: &mut Option<CurrentUserFailure>,
    api_base: &str,
    token: &str,
    error: CurrentUserFetchError,
) {
    let consecutive = match failure.as_ref() {
        Some(entry) if entry.api_base == api_base && entry.token == token => {
            entry.consecutive.saturating_add(1)
        }
        _ => 1,
    };
    *failure = Some(CurrentUserFailure {
        api_base: api_base.to_string(),
        token: token.to_string(),
        failed_at: Instant::now(),
        consecutive,
        error,
    });
}

/// Drop both current-user caches.
///
/// Sign-out must forget the positive snapshot *and* the negative failure record
/// together: signing back in with the same JWT inside their windows would
/// otherwise replay pre-logout state — the old `/auth/me` snapshot for the rest
/// of [`CURRENT_USER_REFRESH_TTL`], and the old error for up to
/// [`CURRENT_USER_BACKOFF_MAX`].
///
/// Public because the canonical sign-out path lives in
/// `security::credentials::ops::clear_session`, outside this module, and these
/// statics are private to it.
pub fn forget_current_user_caches() {
    // Bump BEFORE taking either lock. That order is what makes the checks in
    // `publish_current_user_unless_stale` and
    // `record_current_user_failure_unless_stale` sufficient: a writer holding
    // one of these locks either observes the bump and stands down, or read the
    // generation before it — in which case its write necessarily completed and
    // released the lock before the clear below could acquire it, so the clear
    // lands second and wins. There is no interleaving that leaves pre-logout
    // state behind.
    CURRENT_USER_GENERATION.fetch_add(1, Ordering::SeqCst);
    *CURRENT_USER_CACHE.lock() = None;
    clear_current_user_failure();
}

/// The sign-out generation a refresh must still be running under to publish.
fn current_user_generation() -> u64 {
    CURRENT_USER_GENERATION.load(Ordering::SeqCst)
}

/// Record an availability failure only if `generation` is still current,
/// **checked while holding the failure lock**.
///
/// Checking before the lock would be a check-then-act: sign-out could land in
/// the gap and this write would then restore the negative cache it had just
/// cleared.
fn record_current_user_failure_unless_stale(
    generation: u64,
    api_base: &str,
    token: &str,
    error: CurrentUserFetchError,
) -> bool {
    if !error.is_availability_failure() {
        return true;
    }
    let mut failure = CURRENT_USER_FAILURE.lock();
    if current_user_generation() != generation {
        return false;
    }
    record_current_user_failure_locked(&mut failure, api_base, token, error);
    true
}

/// Publish a fresh `/auth/me` answer to the positive cache, and drop any
/// recorded outage, only if `generation` is still current — each check taken
/// under the lock that guards the record it gates.
///
/// Returns `false` when sign-out won, in which case nothing was written.
fn publish_current_user_unless_stale(
    generation: u64,
    api_base: &str,
    token: &str,
    fetched: Option<Value>,
) -> bool {
    {
        let mut cache = CURRENT_USER_CACHE.lock();
        if current_user_generation() != generation {
            return false;
        }
        match fetched {
            Some(user) => {
                debug!("{LOG_PREFIX} refreshed current user from backend");
                *cache = Some(CachedCurrentUser {
                    api_base: api_base.to_string(),
                    token: token.to_string(),
                    fetched_at: Instant::now(),
                    user,
                });
            }
            None => {
                debug!("{LOG_PREFIX} backend returned empty current user; clearing cache");
                *cache = None;
            }
        }
    }

    // Clearing only ever removes state, so it cannot replay a pre-logout answer
    // — but it could erase an outage a *new* session recorded after signing back
    // in, so it takes the same guard.
    let mut failure = CURRENT_USER_FAILURE.lock();
    if current_user_generation() == generation {
        *failure = None;
    }
    true
}
