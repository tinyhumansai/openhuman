//! Only host-observed tool outcomes count as verification activity.
use once_cell::sync::Lazy;
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Mutex,
};
#[derive(Default, Clone, Debug)]
pub struct Observed {
    pub successful: u32,
    pub shell: u32,
    pub failed: u32,
    pub fingerprint: u64,
}
static RUNS: Lazy<Mutex<HashMap<String, Observed>>> = Lazy::new(|| Mutex::new(HashMap::new()));
pub struct Guard(String);
impl Guard {
    pub fn new(request: &str) -> Self {
        RUNS.lock()
            .unwrap()
            .insert(request.into(), Observed::default());
        Self(request.into())
    }
    pub fn snapshot(&self) -> Observed {
        RUNS.lock()
            .unwrap()
            .get(&self.0)
            .cloned()
            .unwrap_or_default()
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        RUNS.lock().unwrap().remove(&self.0);
    }
}
pub fn record(
    request: &str,
    tool: &str,
    success: bool,
    args: &Option<serde_json::Value>,
    output: &str,
) {
    let mut runs = RUNS.lock().unwrap();
    let Some(run) = runs.get_mut(request) else {
        return;
    };
    if matches!(
        tool,
        "autonomous_work" | "todo" | "goal_get" | "goal_set" | "goal_complete" | "tool_search"
    ) {
        return;
    }
    // A code edit makes earlier execution evidence stale. The final review must
    // inspect and execute checks after the last observed edit, not before it.
    if matches!(tool, "file_write" | "apply_patch") {
        run.successful = 0;
        run.shell = 0;
    }
    if success {
        run.successful += 1;
        if matches!(tool, "shell" | "run_tests" | "run_linter") {
            run.shell += 1;
        }
    } else {
        run.failed += 1;
    }
    let mut hash = DefaultHasher::new();
    run.fingerprint.hash(&mut hash);
    tool.hash(&mut hash);
    success.hash(&mut hash);
    args.as_ref()
        .map(serde_json::Value::to_string)
        .hash(&mut hash);
    output.hash(&mut hash);
    run.fingerprint = hash.finish();
}
pub fn verified(observed: &Observed) -> bool {
    observed.successful >= 2 && observed.shell > 0 && observed.failed == 0
}
