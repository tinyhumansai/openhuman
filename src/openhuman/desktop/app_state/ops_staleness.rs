// How stale the `current_user` the snapshot is serving has become.
//
// Split out of `ops_part_01.rs` (layout gate) and kept together because the
// three pieces are one contract: the stamp is written on every successful
// `auth_get_me`, cleared when the identity goes away, and read back keyed on
// the same `(api_base, token)` both caches use — so one identity's freshness
// is never reported as another's (#5930).
//
// `include!`d into `ops.rs`, so everything here shares that module's scope.

/// When the backend last returned an actual user for `auth_get_me` in this
// process — i.e. when the data the snapshot displays was last replaced.
//
// Deliberately not "when the backend was last healthy": a 200 carrying no user
// leaves the caller on `stored_user`, so counting it as a success would report
// a ~0s age for data that never changed.
///
/// Neither existing cache can answer "how old is the user data we are showing":
/// [`CURRENT_USER_FAILURE`] only knows about failures, and
/// [`CURRENT_USER_CACHE`] is set to `None` whenever the backend returns an
/// empty user, which erases the timestamp on a path that is not an outage.
/// `None` here means "no success yet this process" — the snapshot then reports
/// staleness with an unknown age rather than inventing one (#5930).
///
/// Keyed on `(api_base, token)` like both caches. An unkeyed timestamp would
/// outlive the identity that earned it: the ordinary
/// `credentials::clear_session` logout does not run the deferred-rejection
/// cleanup, so a logout and re-login in the same process — or simply a second
/// identity failing after a first succeeded — would report the previous
/// session's age as this one's.
static LAST_CURRENT_USER_SUCCESS: Lazy<Mutex<Option<CurrentUserSuccess>>> =
    Lazy::new(|| Mutex::new(None));

/// The last successful `auth_get_me`, with the identity it belongs to.
#[derive(Debug, Clone)]
struct CurrentUserSuccess {
    api_base: String,
    token: String,
    at: Instant,
}

/// Stamp a refreshed user, so the snapshot can report how old the data it is
/// serving has become. Callers must not invoke this for an answer that carried
/// no user — see [`LAST_CURRENT_USER_SUCCESS`].
fn note_current_user_success(api_base: &str, token: &str) {
    *LAST_CURRENT_USER_SUCCESS.lock() = Some(CurrentUserSuccess {
        api_base: api_base.to_string(),
        token: token.to_string(),
        at: Instant::now(),
    });
}

/// Forget the success stamp on sign-out, so the next account does not inherit
/// this one's freshness.
fn clear_current_user_success() {
    *LAST_CURRENT_USER_SUCCESS.lock() = None;
}

/// Whether the snapshot is being served from data the backend could not
/// refresh, and how long since it last did.
///
/// Keyed like both caches: a recorded failure only makes *this* `(api_base,
/// token)` stale. Without the key check, switching environment or signing in as
/// someone else would inherit the previous identity's outage and report the
/// fresh data it is about to fetch as stale.
///
/// The age is `None` when *this identity's* backend has not answered at all in
/// this process — the stored snapshot then came off disk, and its true age is
/// not knowable from here. A success recorded against a different `(api_base,
/// token)` is somebody else's freshness and is not reported as this one's.
fn current_user_staleness(api_base: &str, token: &str) -> (bool, Option<u64>) {
    let stale = CURRENT_USER_FAILURE
        .lock()
        .as_ref()
        .is_some_and(|entry| entry.api_base == api_base && entry.token == token);
    let age = LAST_CURRENT_USER_SUCCESS
        .lock()
        .as_ref()
        .filter(|entry| entry.api_base == api_base && entry.token == token)
        .map(|entry| entry.at.elapsed().as_secs());
    (stale, age)
}
