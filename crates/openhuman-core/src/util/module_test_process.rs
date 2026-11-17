//! Process isolation for tests that load a native module onto a runtime-owned bus.
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
