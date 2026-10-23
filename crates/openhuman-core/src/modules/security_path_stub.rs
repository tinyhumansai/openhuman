//! Test-only path-validation port for host adapters without a TinySecurity runtime.
//!
//! Production path authorization always uses the TinySecurity bus. This thread-local
//! fixture grants only paths below one explicitly selected action directory, so
//! filesystem adapter tests can exercise policy ordering without a native module.

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

use tinysecurity_bus::{PathErrorCategory, PathPolicy, PathValidationResult};

thread_local! {
    static ACTION_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Restores the previous action root when dropped.
pub(crate) struct PortGuard(Option<PathBuf>);

impl Drop for PortGuard {
    fn drop(&mut self) {
        ACTION_ROOT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// Allow path validation only for policies rooted at `action_dir`.
pub(crate) fn install_port(action_dir: PathBuf) -> PortGuard {
    PortGuard(ACTION_ROOT.with(|slot| slot.borrow_mut().replace(action_dir)))
}

/// Answer a scoped test request, or defer to the production TinySecurity client.
pub(crate) fn validate_host_path(
    policy: &PathPolicy,
    path: &str,
    _parent: bool,
) -> Option<Result<PathValidationResult, String>> {
    let action_root = ACTION_ROOT.with(|slot| slot.borrow().clone())?;
    if policy.action_dir != action_root {
        return None;
    }

    let requested = Path::new(path);
    let resolved = if requested.is_absolute() {
        if !requested.starts_with(&action_root) {
            return Some(Ok(PathValidationResult::Denied {
                category: PathErrorCategory::PolicyDenied,
            }));
        }
        requested.to_path_buf()
    } else {
        if requested.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Some(Ok(PathValidationResult::Denied {
                category: PathErrorCategory::PolicyDenied,
            }));
        }
        action_root.join(requested)
    };

    Some(Ok(PathValidationResult::Allowed { resolved }))
}
