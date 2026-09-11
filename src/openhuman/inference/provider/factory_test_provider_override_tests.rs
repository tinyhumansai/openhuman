use std::sync::{Arc, Mutex, OnceLock};
use tinyinference::model::ChatModel;

static OVERRIDE: OnceLock<Mutex<Option<Arc<dyn ChatModel<()>>>>> = OnceLock::new();
fn cell() -> &'static Mutex<Option<Arc<dyn ChatModel<()>>>> {
    OVERRIDE.get_or_init(|| Mutex::new(None))
}

pub(crate) fn current() -> Option<Arc<dyn ChatModel<()>>> {
    cell().lock().unwrap().clone()
}

/// Install a crate-native mock model; the returned guard clears it on drop.
#[must_use]
pub fn install_model(model: Arc<dyn ChatModel<()>>) -> InstallGuard {
    *cell().lock().unwrap() = Some(model);
    InstallGuard
}
pub struct InstallGuard;
impl Drop for InstallGuard {
    fn drop(&mut self) {
        *cell().lock().unwrap() = None;
    }
}
