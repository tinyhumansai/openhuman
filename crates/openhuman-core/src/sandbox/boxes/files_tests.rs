use super::*;
use crate::core::runtime::NoTenant;
use crate::sandbox::boxes::scripted_driver::{Call, ExecStep, ScriptedDriver};
use crate::sandbox::boxes::{BoxOutput, RegistryLimits};
use std::sync::Arc;
use std::time::Duration;

pub(super) fn embedded(id: &str) -> Tenant {
    Tenant {
        profile: None,
        agent: Some(id.into()),
    }
}

pub(super) fn registry(driver: &Arc<ScriptedDriver>) -> Arc<BoxRegistry<ScriptedDriver>> {
    BoxRegistry::new(
        Arc::clone(driver),
        RegistryLimits {
            idle_ttl: Duration::from_secs(900),
            max_live: 10,
            retain: None,
        },
    )
}

/// The settings of an OpenShell box with the default `[runtime.openshell]`.
pub(super) fn config() -> BoxSettings {
    BoxSettings::from_config(&crate::config::OpenShellRuntimeConfig::default())
        .expect("the default OpenShell settings are valid")
}

/// Every byte value, so NUL and bytes that are not UTF-8 are in the file.
pub(super) fn binary(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 256) as u8).collect()
}

pub(super) fn reply(exit_code: i32, stdout: &str, stderr: &str) -> ExecStep {
    ExecStep::Reply(BoxOutput {
        exit_code,
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    })
}

pub(super) fn encoded(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The part file a write's first command names: the quoted `./...part` word.
pub(super) fn part_in(command: &str) -> String {
    // The part file's name carries a UUID; the stale-file sweep names a glob.
    let marker = ".openhuman-transfer-";
    let name = command
        .match_indices(marker)
        .map(|(at, _)| at)
        .find(|at| !command[at + marker.len()..].starts_with('?'))
        .expect("a part file");
    let end = name + command[name..].find(".tmp'").expect("a part file") + ".tmp".len();
    let start = command[..end].rfind("'./").expect("a quoted path") + 1;
    command[start..end].to_owned()
}

/// The shell command of every exec the driver saw, in order.
pub(super) fn commands(driver: &ScriptedDriver) -> Vec<String> {
    driver
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Exec { exec, .. } => exec.argv.last().cloned(),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_call_with_no_tenant_is_refused_before_any_box_is_touched() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);

    let wrote = write_file_in(&boxes, Err(NoTenant), &config(), "a.txt", b"x").await;
    let read = read_file_in(&boxes, Err(NoTenant), &config(), "a.txt", 10).await;

    assert_eq!(wrote, Err(BoxError::NoTenant));
    assert_eq!(read, Err(BoxError::NoTenant));
    assert!(driver.calls().is_empty());
}

#[tokio::test]
async fn paths_outside_the_work_folder_are_refused_before_any_box_is_touched() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    let refused = [
        ("", "empty"),
        ("/etc/passwd", "absolute"),
        ("../escape.txt", "dotdot"),
        ("a/../../escape.txt", "nested dotdot"),
        ("a\0b", "NUL"),
        ("dir/", "trailing slash"),
        ("..", "bare dotdot"),
    ];

    for (path, why) in refused {
        let wrote = write_file_in(&boxes, Ok(embedded("u1")), &config(), path, b"x").await;
        let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), path, 10).await;
        assert!(matches!(wrote, Err(BoxError::InvalidPath(_))), "{why}");
        assert!(matches!(read, Err(BoxError::InvalidPath(_))), "{why}");
    }
    assert!(driver.calls().is_empty());
}

#[test]
fn a_refused_path_says_what_to_use_instead() {
    let message = BoxError::InvalidPath("it must be relative").to_string();
    assert_eq!(
        message,
        "the sandbox file path is not allowed: it must be relative"
    );
}

#[tokio::test]
async fn a_name_reserved_for_transfers_is_refused_before_any_box_is_touched() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    let path = "out/.openhuman-transfer-0123.tmp";

    let wrote = write_file_in(&boxes, Ok(embedded("u1")), &config(), path, b"x").await;
    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), path, 10).await;

    let refusal = Err(BoxError::InvalidPath(
        "that file name is reserved for transfers",
    ));
    assert_eq!(wrote, refusal);
    assert_eq!(
        read,
        Err(BoxError::InvalidPath(
            "that file name is reserved for transfers"
        ))
    );
    assert!(driver.calls().is_empty());
}

#[tokio::test]
async fn a_file_over_the_host_limit_is_refused_before_any_box_is_touched() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    let too_big = vec![0u8; MAX_FILE_BYTES as usize + 1];

    let wrote = write_file_in(&boxes, Ok(embedded("u1")), &config(), "big.bin", &too_big).await;

    assert_eq!(
        wrote,
        Err(BoxError::FileTooLarge {
            size: MAX_FILE_BYTES + 1,
            limit: MAX_FILE_BYTES,
        })
    );
    assert!(driver.calls().is_empty());
}

#[tokio::test]
async fn a_small_file_is_one_command_that_writes_a_part_file_and_renames_it() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);

    write_file_in(&boxes, Ok(embedded("u1")), &config(), "out/a.txt", b"hi")
        .await
        .unwrap();

    let sent = commands(&driver);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("mkdir -p -- './out'"), "{}", sent[0]);
    assert!(
        sent[0].contains(&format!("printf %s '{}' | base64 -d", encoded(b"hi"))),
        "{}",
        sent[0]
    );
    let part = part_in(&sent[0]);
    assert!(
        part.starts_with("./out/.openhuman-transfer-") && part.ends_with(".tmp"),
        "{part}"
    );
    assert!(
        sent[0].contains(&format!("mv -f -- '{part}' './out/a.txt'")),
        "{}",
        sent[0]
    );
}

#[tokio::test]
async fn a_large_file_is_sent_in_chunks_that_fit_one_argument() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    let bytes = binary(WRITE_CHUNK_BYTES * 2 + 5);

    write_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", &bytes)
        .await
        .unwrap();

    let sent = commands(&driver);
    assert_eq!(sent.len(), 3);
    assert!(sent.iter().all(|command| command.len() < 128 * 1024));
    // The first chunk starts the part file; the rest append; only the last renames.
    let part = part_in(&sent[0]);
    assert!(
        sent[0].contains(&format!("base64 -d > '{part}'")),
        "{}",
        sent[0]
    );
    assert!(sent[1].contains(&format!("base64 -d >> '{part}'")));
    assert!(!sent[0].contains("mv -f") && !sent[1].contains("mv -f"));
    assert!(sent[2].contains("mv -f"));
}

#[tokio::test]
async fn a_failed_chunk_removes_the_part_file_and_reports_what_the_box_said() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(0, "", ""));
    driver.then(reply(1, "", "sh: write error: No space left on device\n"));
    let bytes = binary(WRITE_CHUNK_BYTES * 2);

    let wrote = write_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", &bytes).await;

    assert_eq!(
        wrote,
        Err(BoxError::FileTransfer {
            action: "write",
            path: "a.bin".into(),
            detail: "sh: write error: No space left on device".into(),
        })
    );
    let sent = commands(&driver);
    assert_eq!(sent.len(), 3, "{sent:?}");
    assert_eq!(sent[2], format!("rm -f -- '{}'", part_in(&sent[0])));
}

#[test]
fn a_failed_transfer_reads_well_as_a_tool_result() {
    let error = BoxError::FileTransfer {
        action: "write",
        path: "a.bin".into(),
        detail: "No space left on device".into(),
    };
    assert_eq!(
        error.to_string(),
        "could not write `a.bin` in the sandbox: No space left on device"
    );
    assert_eq!(
        BoxError::FileNotFound("a.bin".into()).to_string(),
        "there is no file `a.bin` in the sandbox"
    );
    assert_eq!(
        BoxError::FileTooLarge { size: 10, limit: 4 }.to_string(),
        "the file is 10 bytes, over the 4 byte limit"
    );
    assert_eq!(
        BoxError::ShortRead {
            expected: 10,
            got: 4
        }
        .to_string(),
        "the sandbox returned 4 of the file's 10 bytes"
    );
}

#[tokio::test]
async fn a_file_over_the_callers_limit_is_refused_before_any_chunk_is_read() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(0, "1000 1:2:3\n", ""));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", 999).await;

    assert_eq!(
        read,
        Err(BoxError::FileTooLarge {
            size: 1000,
            limit: 999
        })
    );
    // Only the size check ran.
    assert_eq!(commands(&driver).len(), 1);
}

#[tokio::test]
async fn the_host_limit_caps_a_callers_larger_limit() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(0, &format!("{} 1:2:3\n", MAX_FILE_BYTES + 1), ""));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", u64::MAX).await;

    assert_eq!(
        read,
        Err(BoxError::FileTooLarge {
            size: MAX_FILE_BYTES + 1,
            limit: MAX_FILE_BYTES
        })
    );
    assert_eq!(commands(&driver).len(), 1);
}

#[tokio::test]
async fn a_missing_file_is_reported_by_name() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(66, "", ""));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "gone.pdf", 100).await;

    assert_eq!(read, Err(BoxError::FileNotFound("gone.pdf".into())));
    assert!(commands(&driver)[0].starts_with("test -f './gone.pdf' || exit 66"));
}

#[tokio::test]
async fn a_short_chunk_fails_the_read_instead_of_returning_a_short_file() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(0, "10 1:2:3\n", ""));
    driver.then(reply(0, &encoded(b"1234"), ""));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", 100).await;

    assert_eq!(
        read,
        Err(BoxError::ShortRead {
            expected: 10,
            got: 4
        })
    );
}

#[tokio::test]
async fn a_chunk_that_is_not_base64_fails_the_read() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(0, "4 1:2:3\n", ""));
    driver.then(reply(0, "!!!!", ""));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", 100).await;

    assert!(
        matches!(read, Err(BoxError::FileTransfer { .. })),
        "{read:?}"
    );
}

#[tokio::test]
async fn a_failing_chunk_command_fails_the_read_with_the_boxs_message() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(0, "4 1:2:3\n", ""));
    driver.then(reply(
        1,
        "",
        "tail: error reading 'a.bin': Input/output error\n",
    ));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", 100).await;

    assert_eq!(
        read,
        Err(BoxError::FileTransfer {
            action: "read",
            path: "a.bin".into(),
            detail: "tail: error reading 'a.bin': Input/output error".into(),
        })
    );
}

#[tokio::test]
async fn a_size_the_box_does_not_report_as_a_number_fails_the_read() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(0, "lots", ""));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", 100).await;

    assert!(
        matches!(read, Err(BoxError::FileTransfer { .. })),
        "{read:?}"
    );
}

#[tokio::test]
async fn an_empty_file_reads_as_no_bytes_without_a_chunk() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(0, "0 1:2:3\n", ""));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", 100).await;

    assert_eq!(read, Ok(Vec::new()));
    assert_eq!(commands(&driver).len(), 1);
}

#[tokio::test]
async fn a_command_that_times_out_is_a_failed_transfer() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(124, "", ""));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", 100).await;

    assert_eq!(
        read,
        Err(BoxError::FileTransfer {
            action: "read",
            path: "a.bin".into(),
            detail: "the sandbox took too long".into(),
        })
    );
}

#[tokio::test]
async fn a_failure_with_no_message_still_names_the_exit_status() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(reply(2, "", "  \n"));

    let wrote = write_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", b"x").await;

    assert_eq!(
        wrote,
        Err(BoxError::FileTransfer {
            action: "write",
            path: "a.bin".into(),
            detail: "the command exited with status 2".into(),
        })
    );
}

#[tokio::test]
async fn a_driver_failure_is_passed_through() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver);
    driver.then(ExecStep::Fail(BoxError::Driver("gateway down".into())));

    let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", 100).await;

    assert_eq!(read, Err(BoxError::Driver("gateway down".into())));
}
