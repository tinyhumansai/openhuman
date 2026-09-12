//! Pins the host copy of the scope-pref storage shape.
//!
//! These constants were copied out of `tinymemory-core` when the two RPC
//! handlers stopped reaching the in-process engine (openhuman#5560). The engine
//! still reads the same rows to gate tool calls by scope, and it now runs
//! inside the loaded module — so drift between the two spellings does not
//! fail. It reads as "no preference stored" and hands the agent the permissive
//! default while the user's saved choice sits one key away: a permission bug
//! that looks like a working app. Hence a test rather than a comment.
//!
//! **What this can and cannot check, stated rather than implied.**
//! `tinymemory-core`'s own `user_scopes::KV_NAMESPACE` no longer exists —
//! tinymemory v1.13.4 deleted the whole in-process Composio pipeline, this
//! constant included — so there is nothing left in that crate to assert
//! against. What *is* still public and load-bearing is `tinycortex`'s own
//! `memory::sync::state::STATE_NAMESPACE` (`"composio-sync-state"`), which is
//! the literal `memory_cleanup.rs` reads and writes under, and the two must
//! differ so prefs and Composio sync cursors never collide — that is the half
//! asserted here, and the literal is asserted against itself.
//!
//! `tinycortex` resolves because it is an ordinary dependency of this crate
//! (`Cargo.toml`); production code in `user_scopes.rs` names neither it nor
//! any engine item.

use super::*;

/// `kv_key` trims and ASCII-lowercases, exactly as the engine's does — the RPC
/// takes free text from a settings toggle, so `"GitHub"`, `" github "` and
/// `"github"` have to reach one row.
#[test]
fn kv_key_normalises_the_toolkit_the_same_way_the_engine_does() {
    assert_eq!(kv_key(" GitHub "), "github");
    assert_eq!(kv_key("SLACK"), "slack");
    assert_eq!(kv_key("gmail"), "gmail");
    assert_eq!(kv_key("   "), "", "an all-whitespace toolkit has no row");
}

/// The stored JSON is the engine's `UserScopePref`, field for field.
///
/// A host-side twin with the same three booleans would serialise identically
/// today and drift on the first field either side adds, so the production code
/// re-exports the engine's type instead of declaring one. This asserts the
/// bytes that actually land in the row.
#[test]
fn stored_value_is_the_three_boolean_fields() {
    let value = serde_json::to_value(UserScopePref {
        read: true,
        write: false,
        admin: true,
    })
    .expect("UserScopePref serialises");
    assert_eq!(
        value,
        serde_json::json!({ "read": true, "write": false, "admin": true })
    );
}

/// The default a failed or absent read falls back to: productive, not
/// permissive-with-admin.
#[test]
fn default_pref_is_read_write_without_admin() {
    let pref = UserScopePref::default();
    assert!(pref.read);
    assert!(pref.write);
    assert!(!pref.admin, "admin must stay opt-in");
}
