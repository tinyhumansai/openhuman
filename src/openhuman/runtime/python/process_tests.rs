use super::*;
use crate::openhuman::runtime::python::bootstrap::PythonSource;

// `apply_no_window` is a no-op off Windows, but exercising the spawn path
// end-to-end keeps the GH-4814 CREATE_NO_WINDOW hook covered. `/bin/cat
// <file>` prints the file and exits, so it stands in for the python child.
#[cfg(unix)]
#[tokio::test]
async fn spawn_stdio_process_launches_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("payload.txt");
    std::fs::write(&script, b"ok").expect("write payload");

    let resolved = ResolvedPython {
        bin_dir: PathBuf::from("/bin"),
        python_bin: PathBuf::from("/bin/cat"),
        version: "test".to_string(),
        source: PythonSource::System,
    };
    let mut spec = PythonLaunchSpec::new(script);
    spec.unbuffered = false; // `-u` is python-only; plain `cat <file>` here

    let mut child = spawn_stdio_process(&resolved, &spec).expect("spawn cat");
    let status = child.wait().await.expect("wait cat");
    assert!(status.success());
}
