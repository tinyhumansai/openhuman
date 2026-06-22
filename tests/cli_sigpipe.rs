#[cfg(unix)]
#[test]
fn help_output_closed_pipe_does_not_panic() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .arg("--help")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn openhuman-core --help");

    // Mirror the issue repro (`openhuman-core --help | head -n 1`): read a
    // single line, then drop the read end mid-stream so the child's next write
    // lands on a closed pipe. Closing before reading (as a naive test does)
    // lets the child buffer its entire few-KB `--help` output in one successful
    // write well under the 64 KB pipe buffer and exit cleanly, never exercising
    // the broken-pipe path — a false green that passes even without the fix.
    let stdout = child.stdout.take().expect("capture stdout");
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .expect("read first help line");
    drop(reader);

    let output = child.wait_with_output().expect("wait for openhuman-core");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !stderr.contains("Broken pipe"),
        "stderr must not include a broken-pipe panic: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "stderr must not include a panic report: {stderr}"
    );

    // Acceptance criterion #1: a closed downstream reader must yield a clean
    // exit — either the child finished before the pipe closed, or the restored
    // default disposition let SIGPIPE terminate it. A normal-code crash (e.g.
    // the panic exit code 101) is precisely the regression this guards against.
    assert!(
        output.status.success() || output.status.signal() == Some(libc::SIGPIPE),
        "process must exit cleanly or via SIGPIPE, got {:?}",
        output.status
    );
}
