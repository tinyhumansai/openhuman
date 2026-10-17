use std::sync::{Arc, Mutex, OnceLock};
use tinyinference_llm::model::ChatModel;

static OVERRIDE: OnceLock<Mutex<Option<Arc<dyn ChatModel<()>>>>> = OnceLock::new();
// The override is process-wide, so tests using it must never replace or clear
// one another's model while an inference call is in flight.
static OVERRIDE_TEST_LOCK: Mutex<()> = Mutex::new(());
fn cell() -> &'static Mutex<Option<Arc<dyn ChatModel<()>>>> {
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub(crate) fn current() -> Option<Arc<dyn ChatModel<()>>> {
    cell().lock().unwrap().clone()
}

/// Install a crate-native mock model; the returned guard clears it on drop.
#[must_use]
pub fn install_model(model: Arc<dyn ChatModel<()>>) -> InstallGuard {
    let serial = OVERRIDE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *cell().lock().unwrap() = Some(model);
    InstallGuard { _serial: serial }
}
/// Hold the override serial lock without installing a model.
///
/// A test that asserts what the factory really constructs (a route's provider,
/// the resolved model id) must not run while another test has a mock
/// installed: the factory would answer with `mock-model` instead. Holding this
/// guard waits out any installer and keeps new ones out until it drops.
#[must_use]
pub fn exclusive() -> ExclusiveGuard {
    ExclusiveGuard {
        _serial: OVERRIDE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    }
}
/// Proof that no test override can be installed; see [`exclusive`].
pub struct ExclusiveGuard {
    _serial: std::sync::MutexGuard<'static, ()>,
}
pub struct InstallGuard {
    _serial: std::sync::MutexGuard<'static, ()>,
}
impl Drop for InstallGuard {
    fn drop(&mut self) {
        *cell().lock().unwrap() = None;
    }
}
