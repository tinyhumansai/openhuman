//! Orders request registration and its approval surfaces after workspace resolution.

use super::ApprovalScope;

/// Resolve the workspace before registering and publishing any approval surface.
/// Removal during resolution refuses registration; accepted registration and
/// publication finish together before removal. No barrier is held across the await.
pub(super) async fn register_after_workspace<W, T>(
    scope: Option<&ApprovalScope>,
    workspace: impl std::future::Future<Output = W>,
    register: impl FnOnce(W) -> T,
) -> Result<T, String> {
    let workspace = workspace.await;
    match scope {
        Some(scope) => scope.with_open(|| register(workspace)),
        None => Ok(register(workspace)),
    }
}

#[cfg(test)]
#[path = "flow_surface_tests.rs"]
mod tests;
