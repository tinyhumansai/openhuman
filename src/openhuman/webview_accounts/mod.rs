//! Webview account login detection for the core sidecar.
//!
//! The Tauri shell hosts CEF-backed webviews for third-party accounts
//! (Gmail, WhatsApp, Telegram, Slack, Discord, LinkedIn, Zoom, Google
//! Messages). Their HTTP cookies live in a single shared Chromium
//! cookie store at `{CEF_USER_DATA_DIR}/Default/Cookies` — a SQLite
//! database. The core runs as a child sidecar and has no direct handle
//! to CEF, so the Tauri shell exports `OPENHUMAN_CEF_COOKIES_DB`
//! pointing at that file before spawning core.
//!
//! The `ops` submodule opens the DB read-only and asks a simple
//! question per provider: "is there a row whose `host_key` matches our
//! expected host suffix and whose `name` matches a known session-cookie
//! name?" If so, we report `logged_in: true` for that provider. If the
//! env var is missing, the DB can't be opened (locked, corrupt,
//! nonexistent), or no matching rows exist, we report
//! `logged_in: false` for every provider — never return an error, the
//! welcome-agent snapshot must always build.
//!
//! This is a heuristic. Chromium prunes expired cookies at startup, so
//! any row with a known session-cookie name is a strong signal the
//! user has an active session for that provider.

mod ops;

pub use ops::detect_webview_logins;
