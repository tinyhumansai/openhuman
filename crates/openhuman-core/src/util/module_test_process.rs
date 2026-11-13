//! Process isolation for tests that load a native module onto a runtime-owned bus.
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Serialises every test in this process that spawns a native-module child.
///
/// Six test names across `agent::artifacts`, `tools::impl::presentation`,
/// `tools::impl::document` and `memory::convert` spawn fresh test binaries
/// that all load the same checksum-pinned `tinydocs_module.dll` under
/// `%LOCALAPPDATA%/openhuman/modules`. Under the parallel default the
/// children race the fixture's engine state inside the DLL (which imports
/// `CreateMutexA`/`WaitForSingleObject`) and the interbred `is_error:true`
/// result trips the tests' `is_error == false` asserts. Holding one process-wide
/// guard across the listing-plus-child lifetime makes the children sequential,
/// which is exactly how these tests would behave under `--test-threads=1`
/// without disabling parallelism for everything else.
#[cfg(test)]
static NATIVE_MODULE_LOCK: Mutex<()> = Mutex::new(());

/// Run the calling test once in a fresh process; return true in its parent.
///
/// The selected test must exist, and its real assertions must finish successfully.
/// Child processes inherit the checksum-pinned module fixtures from the test runner.
pub(crate) fn run(module: &str, name: &str) -> bool {
    let module = module.split_once("::").expect("test has a crate prefix").1;
    let name = format!("{module}::{name}");
    if std::env::var("OPENHUMAN_NATIVE_TEST_CHILD").as_deref() == Ok(name.as_str()) {
        return false;
    }
    #[cfg(test)]
    let _guard = NATIVE_MODULE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let executable = std::env::current_exe().expect("test executable");
    let listing = Command::new(&executable)
        .args(["--exact", &name, "--list"])
        .output()
        .expect("list the isolated test");
    assert!(listing.status.success(), "could not list {name}");
    assert!(
        String::from_utf8_lossy(&listing.stdout)
            .lines()
            .any(|line| line == format!("{name}: test")),
        "isolated test {name} must exist; refusing a zero-test pass"
    );
    let mut child = Command::new(executable)
        .args(["--exact", &name, "--nocapture"])
        .env("OPENHUMAN_NATIVE_TEST_CHILD", &name)
        .stdin(Stdio::null())
        // Without inheriting stdio the child's panic message is lost and the
        // parent only sees "exit code: 101" with no context.
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start the isolated native-module test");
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait().expect("wait for native-module test") {
            assert!(
                status.success(),
                "isolated native-module test {name} failed: {status}"
            );
            return true;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated native-module test {name} exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
