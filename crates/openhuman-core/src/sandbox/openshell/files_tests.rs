use super::*;
use crate::core::runtime::NoTenant;
use crate::sandbox::openshell::scripted_driver::{Call, ExecStep, ScriptedDriver};
use crate::sandbox::openshell::{BoxOutput, RegistryLimits};
use std::sync::Arc;
use std::time::Duration;

fn embedded(id: &str) -> Tenant {
    Tenant {
        profile: None,
        agent: Some(id.into()),
    }
}

fn registry(driver: &Arc<ScriptedDriver>) -> Arc<BoxRegistry<ScriptedDriver>> {
    BoxRegistry::new(
        Arc::clone(driver),
        RegistryLimits {
            idle_ttl: Duration::from_secs(900),
            max_live: 10,
        },
    )
}

fn config() -> OpenShellRuntimeConfig {
    OpenShellRuntimeConfig::default()
}

/// Every byte value, so NUL and bytes that are not UTF-8 are in the file.
fn binary(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 256) as u8).collect()
}

fn reply(exit_code: i32, stdout: &str, stderr: &str) -> ExecStep {
    ExecStep::Reply(BoxOutput {
        exit_code,
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    })
}

fn encoded(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The part file a write's first command names: the quoted `./...part` word.
fn part_in(command: &str) -> String {
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
fn commands(driver: &ScriptedDriver) -> Vec<String> {
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

/// Tests that run the commands for real, in a directory standing in for each
/// box, so the quoting and the encoding are checked against a shell.
#[cfg(target_os = "linux")]
mod on_a_real_shell {
    use super::*;

    fn host_boxes() -> (
        tempfile::TempDir,
        Arc<ScriptedDriver>,
        Arc<BoxRegistry<ScriptedDriver>>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let driver = ScriptedDriver::new();
        driver.keep_boxes_in(root.path());
        let boxes = registry(&driver);
        (root, driver, boxes)
    }

    #[tokio::test]
    async fn binary_bytes_spanning_several_chunks_round_trip() {
        let (_root, driver, boxes) = host_boxes();
        let bytes = binary(READ_CHUNK_BYTES * 2 + WRITE_CHUNK_BYTES + 17);
        assert!(bytes.contains(&0) && std::str::from_utf8(&bytes).is_err());

        write_file_in(
            &boxes,
            Ok(embedded("u1")),
            &config(),
            "work/out.bin",
            &bytes,
        )
        .await
        .unwrap();
        let back = read_file_in(
            &boxes,
            Ok(embedded("u1")),
            &config(),
            "work/out.bin",
            MAX_FILE_BYTES,
        )
        .await
        .unwrap();

        assert_eq!(back, bytes);
        let on_disk = driver.box_dir("box-0").unwrap().join("work/out.bin");
        assert_eq!(std::fs::read(on_disk).unwrap(), bytes);
        let left: Vec<_> = std::fs::read_dir(driver.box_dir("box-0").unwrap().join("work"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["out.bin"], "no part file is left behind");
    }

    #[tokio::test]
    async fn a_path_with_a_space_and_a_quote_is_one_file() {
        let (_root, driver, boxes) = host_boxes();
        let path = "my files/it's; $(touch pwned) here.txt";

        write_file_in(&boxes, Ok(embedded("u1")), &config(), path, b"hello")
            .await
            .unwrap();
        let back = read_file_in(&boxes, Ok(embedded("u1")), &config(), path, 100)
            .await
            .unwrap();

        assert_eq!(back, b"hello");
        let dir = driver.box_dir("box-0").unwrap();
        assert!(dir.join(path).is_file());
        assert!(!dir.join("pwned").exists());
    }

    #[tokio::test]
    async fn a_name_that_looks_like_an_option_is_a_file() {
        let (_root, driver, boxes) = host_boxes();

        write_file_in(&boxes, Ok(embedded("u1")), &config(), "-rf", b"x")
            .await
            .unwrap();
        let back = read_file_in(&boxes, Ok(embedded("u1")), &config(), "-rf", 10)
            .await
            .unwrap();

        assert_eq!(back, b"x");
        assert!(driver.box_dir("box-0").unwrap().join("-rf").is_file());
    }

    #[tokio::test]
    async fn an_empty_file_round_trips() {
        let (_root, _driver, boxes) = host_boxes();

        write_file_in(&boxes, Ok(embedded("u1")), &config(), "empty", b"")
            .await
            .unwrap();
        let back = read_file_in(&boxes, Ok(embedded("u1")), &config(), "empty", 10)
            .await
            .unwrap();

        assert!(back.is_empty());
    }

    #[tokio::test]
    async fn writing_again_replaces_the_file() {
        let (_root, _driver, boxes) = host_boxes();

        for body in [&b"first and longer"[..], b"second"] {
            write_file_in(&boxes, Ok(embedded("u1")), &config(), "a.txt", body)
                .await
                .unwrap();
        }
        let back = read_file_in(&boxes, Ok(embedded("u1")), &config(), "a.txt", 100)
            .await
            .unwrap();

        assert_eq!(back, b"second");
    }

    #[tokio::test]
    async fn a_directory_is_not_overwritten_and_is_not_a_readable_file() {
        let (_root, _driver, boxes) = host_boxes();
        write_file_in(&boxes, Ok(embedded("u1")), &config(), "d/a.txt", b"x")
            .await
            .unwrap();

        let wrote = write_file_in(&boxes, Ok(embedded("u1")), &config(), "d", b"y").await;
        let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "d", 10).await;

        assert!(
            matches!(wrote, Err(BoxError::FileTransfer { .. })),
            "{wrote:?}"
        );
        assert_eq!(read, Err(BoxError::FileNotFound("d".into())));
    }

    #[tokio::test]
    async fn a_missing_file_is_not_found() {
        let (_root, _driver, boxes) = host_boxes();

        let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "nope.txt", 10).await;

        assert_eq!(read, Err(BoxError::FileNotFound("nope.txt".into())));
    }

    #[tokio::test]
    async fn tenants_never_share_a_box_or_its_files() {
        let (_root, driver, boxes) = host_boxes();
        write_file_in(&boxes, Ok(embedded("u1")), &config(), "secret.txt", b"mine")
            .await
            .unwrap();

        let theirs = read_file_in(&boxes, Ok(embedded("u2")), &config(), "secret.txt", 10).await;
        let mine = read_file_in(&boxes, Ok(embedded("u1")), &config(), "secret.txt", 10).await;

        assert_eq!(theirs, Err(BoxError::FileNotFound("secret.txt".into())));
        assert_eq!(mine, Ok(b"mine".to_vec()));
        assert_eq!(driver.created(), 2);
        assert!(driver
            .box_dir("box-0")
            .unwrap()
            .join("secret.txt")
            .is_file());
        assert!(!driver.box_dir("box-1").unwrap().join("secret.txt").exists());
    }

    #[tokio::test]
    async fn a_box_replaced_mid_write_never_leaves_a_truncated_file() {
        let (_root, driver, boxes) = host_boxes();
        // The box is lost after the first chunk: the registry retries the
        // second in a fresh box, which has none of the first.
        driver.then(reply(0, "", ""));
        driver.then(ExecStep::Fail(BoxError::UnknownResource));
        let bytes = binary(WRITE_CHUNK_BYTES * 2);

        let wrote = write_file_in(&boxes, Ok(embedded("u1")), &config(), "a.bin", &bytes).await;

        assert_eq!(
            wrote,
            Err(BoxError::FileTransfer {
                action: "write",
                path: "a.bin".into(),
                detail: "the file was not written in full".into(),
            })
        );
        assert_eq!(driver.created(), 2);
        assert!(!driver.box_dir("box-1").unwrap().join("a.bin").exists());
    }

    #[tokio::test]
    async fn a_name_at_the_filesystem_limit_still_stages() {
        let (_root, _driver, boxes) = host_boxes();
        let name = "n".repeat(255);

        write_file_in(&boxes, Ok(embedded("u1")), &config(), &name, b"long")
            .await
            .unwrap();
        let back = read_file_in(&boxes, Ok(embedded("u1")), &config(), &name, 10).await;

        assert_eq!(back, Ok(b"long".to_vec()));
    }

    #[tokio::test]
    async fn a_link_to_a_file_inside_the_work_folder_reads_the_files_bytes() {
        let (_root, driver, boxes) = host_boxes();
        write_file_in(
            &boxes,
            Ok(embedded("u1")),
            &config(),
            "target.bin",
            b"the real bytes of the target",
        )
        .await
        .unwrap();
        let dir = driver.box_dir("box-0").unwrap();
        std::os::unix::fs::symlink("target.bin", dir.join("alias.bin")).unwrap();

        let back = read_file_in(&boxes, Ok(embedded("u1")), &config(), "alias.bin", 100).await;

        assert_eq!(back, Ok(b"the real bytes of the target".to_vec()));
    }

    #[tokio::test]
    async fn a_part_file_abandoned_by_a_cancelled_upload_is_reclaimed_by_the_next_write() {
        let (_root, driver, boxes) = host_boxes();
        write_file_in(&boxes, Ok(embedded("u1")), &config(), "out/seed.txt", b"x")
            .await
            .unwrap();
        let dir = driver.box_dir("box-0").unwrap().join("out");
        let (old, fresh) = (
            dir.join(".openhuman-transfer-00000000000000000000000000000001.tmp"),
            dir.join(".openhuman-transfer-00000000000000000000000000000002.tmp"),
        );
        std::fs::write(&old, b"half").unwrap();
        std::fs::write(&fresh, b"in flight").unwrap();
        let (_, _, code) = run_in(
            &dir,
            "touch -d '3 hours ago' .openhuman-transfer-00000000000000000000000000000001.tmp",
        );
        assert_eq!(code, 0);

        write_file_in(&boxes, Ok(embedded("u1")), &config(), "out/a.txt", b"y")
            .await
            .unwrap();

        assert!(!old.exists(), "an hour-old part file is removed");
        assert!(fresh.exists(), "a recent one may be an upload in flight");
        assert!(dir.join("a.txt").is_file());
    }

    #[tokio::test]
    async fn a_user_file_that_only_looks_like_a_part_file_is_never_swept() {
        let (_root, driver, boxes) = host_boxes();
        write_file_in(&boxes, Ok(embedded("u1")), &config(), "out/seed.txt", b"x")
            .await
            .unwrap();
        let dir = driver.box_dir("box-0").unwrap().join("out");
        std::fs::write(dir.join(".openhuman-report.tmp"), b"mine").unwrap();
        run_in(&dir, "touch -d '3 hours ago' .openhuman-report.tmp");

        write_file_in(&boxes, Ok(embedded("u1")), &config(), "out/new.txt", b"y")
            .await
            .unwrap();

        assert!(dir.join(".openhuman-report.tmp").exists());
    }

    #[tokio::test]
    async fn a_file_named_dash_is_a_file_and_not_standard_input() {
        let (_root, _driver, boxes) = host_boxes();

        write_file_in(&boxes, Ok(embedded("u1")), &config(), "-", b"dash")
            .await
            .unwrap();
        let back = read_file_in(&boxes, Ok(embedded("u1")), &config(), "-", 10).await;

        assert_eq!(back, Ok(b"dash".to_vec()));
    }

    #[tokio::test]
    async fn each_write_stages_in_a_part_file_of_its_own() {
        let driver = ScriptedDriver::new();
        let recorded = registry(&driver);
        for _ in 0..2 {
            write_file_in(&recorded, Ok(embedded("u1")), &config(), "a.txt", b"x")
                .await
                .unwrap();
        }
        let sent = commands(&driver);
        assert_ne!(part_in(&sent[0]), part_in(&sent[1]));
    }

    #[tokio::test]
    async fn a_file_that_is_named_like_the_old_part_file_is_left_alone() {
        let (_root, driver, boxes) = host_boxes();
        write_file_in(&boxes, Ok(embedded("u1")), &config(), "a.txt.part", b"mine")
            .await
            .unwrap();

        write_file_in(&boxes, Ok(embedded("u1")), &config(), "a.txt", b"new")
            .await
            .unwrap();

        let dir = driver.box_dir("box-0").unwrap();
        assert_eq!(std::fs::read(dir.join("a.txt.part")).unwrap(), b"mine");
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"new");
    }

    #[tokio::test]
    async fn interleaved_uploads_of_one_path_never_publish_a_mix() {
        let (_root, _driver, boxes) = host_boxes();
        let a = binary(WRITE_CHUNK_BYTES * 3);
        let b: Vec<u8> = a.iter().map(|byte| byte.wrapping_add(1)).collect();

        let cfg = config();
        let (first, second) = tokio::join!(
            write_file_in(&boxes, Ok(embedded("u1")), &cfg, "a.bin", &a),
            write_file_in(&boxes, Ok(embedded("u1")), &cfg, "a.bin", &b),
        );
        first.unwrap();
        second.unwrap();
        let back = read_file_in(
            &boxes,
            Ok(embedded("u1")),
            &config(),
            "a.bin",
            MAX_FILE_BYTES,
        )
        .await
        .unwrap();

        assert!(
            back == a || back == b,
            "the file is one upload or the other"
        );
    }

    #[tokio::test]
    async fn a_link_out_of_the_work_folder_is_not_followed_to_read_or_write() {
        let (root, driver, boxes) = host_boxes();
        write_file_in(&boxes, Ok(embedded("u1")), &config(), "seed", b"x")
            .await
            .unwrap();
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("passwd"), b"secret").unwrap();
        let dir = driver.box_dir("box-0").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("out")).unwrap();

        let read = read_file_in(&boxes, Ok(embedded("u1")), &config(), "out/passwd", 100).await;
        let wrote = write_file_in(&boxes, Ok(embedded("u1")), &config(), "out/new.txt", b"x").await;
        let nested = write_file_in(
            &boxes,
            Ok(embedded("u1")),
            &config(),
            "out/deeper/new.txt",
            b"x",
        )
        .await;

        for result in [read.map(|_| ()), wrote, nested] {
            let Err(BoxError::FileTransfer { detail, .. }) = result else {
                panic!("expected a refusal, got {result:?}");
            };
            assert!(
                detail.contains("outside the sandbox work folder"),
                "{detail}"
            );
        }
        assert!(!outside.join("new.txt").exists());
        assert!(!outside.join("deeper").exists());
    }

    #[test]
    fn a_file_replaced_between_chunks_fails_the_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"abcd").unwrap();
        let identity = run_in(dir.path(), "stat -L -c '%d:%i:%y:%z' -- ./a.bin").0;
        let command = read_command("'./a.bin'", identity.trim(), 1, 4);
        assert_eq!(run_in(dir.path(), &command).0.trim(), encoded(b"abcd"));

        // Same length, new file: every size check would pass.
        std::fs::write(dir.path().join("new"), b"wxyz").unwrap();
        std::fs::rename(dir.path().join("new"), dir.path().join("a.bin")).unwrap();
        let (_, stderr, code) = run_in(dir.path(), &command);

        assert_eq!(code, 1);
        assert!(stderr.contains("changed while it was read"), "{stderr}");
    }

    #[test]
    fn a_file_rewritten_in_place_at_the_same_size_fails_the_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.bin"), b"abcd").unwrap();
        let identity = run_in(dir.path(), "stat -L -c '%d:%i:%y:%z' -- ./a.bin").0;
        let command = read_command("'./a.bin'", identity.trim(), 1, 4);
        assert_eq!(run_in(dir.path(), &command).2, 0);

        // The same inode, the same size and the same second.
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(dir.path().join("a.bin"), b"wxyz").unwrap();
        let (_, stderr, code) = run_in(dir.path(), &command);

        assert_eq!(code, 1);
        assert!(stderr.contains("changed while it was read"), "{stderr}");
    }

    /// Run `command` under `sh -c` in `dir`: its stdout, stderr and exit code.
    fn run_in(dir: &std::path::Path, command: &str) -> (String, String, i32) {
        let output = std::process::Command::new("sh")
            .args(["-c", command])
            .current_dir(dir)
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code().unwrap_or(-1),
        )
    }
}
