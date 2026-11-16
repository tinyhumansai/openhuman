//! Per-call workspace scoping of a [`SecurityPolicy`].

use std::path::Path;

use super::{SecurityPolicy, TrustedAccess, TrustedRoot};

/// Clone `security` and scope it to `root`, a run's workspace descriptor root.
///
/// The root becomes both the relative-path resolution root (`action_dir`)
/// **and** a `ReadWrite` trusted root. The grant is the load-bearing half:
/// `action_dir` only decides where a relative path lands, while the allow/deny
/// decision reads `workspace_dir` + `trusted_roots`
/// (TinySecurity's scoped path authorization). Without the grant a
/// descriptor rooted outside `workspace_dir` moved the cwd but refused every
/// read and write in it.
///
/// The grant is *additive and per-call*: it is pushed onto a clone, so nothing
/// process-global is mutated and concurrent turns cannot race each other. It
/// also cannot widen the hard invariants: TinySecurity's protected-path floor
/// (credential stores, core OS dirs) and internal-state reservations (core-managed
/// state under `workspace_dir`) are both evaluated *before* any trusted-root
/// shortcut, so a granted root can never expose them.
///
/// The root always originates from trusted in-process code (the session
/// builder, the sub-agent runner, or the `cwd` RPC parameter), never from
/// model-supplied text.
pub fn scoped_to_workspace(security: &SecurityPolicy, root: &Path) -> SecurityPolicy {
    let mut scoped = security.clone();
    scoped.action_dir = root.to_path_buf();
    scoped.trusted_roots.push(TrustedRoot {
        path: root.to_string_lossy().to_string(),
        access: TrustedAccess::ReadWrite,
    });
    scoped
}

#[cfg(test)]
#[path = "workspace_scope_tests.rs"]
mod tests;
