use super::*;

fn operation<'a>(command: &'a str, cwd: &'a Path) -> SpawnOperation<'a> {
    SpawnOperation {
        command,
        cwd,
        env: vec![("A".into(), "1".into()), ("B".into(), "2".into())],
        stdin: None,
        preserve_pipeline_status: false,
    }
}

fn canonical(operation: &SpawnOperation<'_>) -> String {
    operation.canonical().expect("canonical operation")
}

#[test]
fn the_document_is_deterministic_and_independent_of_environment_order() {
    let cwd = Path::new("/work");
    let first = operation("ls", cwd);
    let mut reordered = operation("ls", cwd);
    reordered.env.reverse();
    assert_eq!(canonical(&first), canonical(&reordered));
    assert_eq!(canonical(&first), canonical(&operation("ls", cwd)));
}

#[test]
fn any_change_to_the_concrete_spawn_changes_the_document() {
    let cwd = Path::new("/work");
    let base = canonical(&operation("ls", cwd));
    let mut changed_env = operation("ls", cwd);
    changed_env.env[0].1 = "other".into();
    let mut renamed_env = operation("ls", cwd);
    renamed_env.env[0].0 = "C".into();
    let mut with_stdin = operation("ls", cwd);
    with_stdin.stdin = Some(b"input");
    let mut other_stdin = operation("ls", cwd);
    other_stdin.stdin = Some(b"inpuT");
    let mut piped = operation("ls", cwd);
    piped.preserve_pipeline_status = true;
    let documents = [
        canonical(&operation("ls -a", cwd)),
        canonical(&operation("ls", Path::new("/other"))),
        canonical(&changed_env),
        canonical(&renamed_env),
        canonical(&with_stdin),
        canonical(&other_stdin),
        canonical(&piped),
        base.clone(),
    ];
    let mut unique = documents.to_vec();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), documents.len());
}

#[test]
fn environment_pairs_cannot_be_resplit_into_another_environment() {
    let cwd = Path::new("/work");
    let mut joined = operation("ls", cwd);
    joined.env = vec![("AB".into(), "C".into())];
    let mut split = operation("ls", cwd);
    split.env = vec![("A".into(), "BC".into())];
    assert_ne!(canonical(&joined), canonical(&split));
}

#[test]
fn environment_values_never_appear_in_the_document() {
    let cwd = Path::new("/work");
    let mut secret = operation("ls", cwd);
    secret.env = vec![("API_KEY".into(), "sk-very-secret".into())];
    secret.stdin = Some(b"stdin-secret");
    let text = canonical(&secret);
    assert!(!text.contains("sk-very-secret") && !text.contains("stdin-secret"));
    assert!(text.contains("API_KEY"));
}

#[test]
fn a_short_command_is_readable_and_a_long_one_is_bound_by_digest() {
    let cwd = Path::new("/work");
    let short: serde_json::Value =
        serde_json::from_str(&canonical(&operation("echo hi", cwd))).unwrap();
    assert_eq!(short["command"], "echo hi");
    let long_text = "x".repeat(INLINE_COMMAND_BYTES + 1);
    let long: serde_json::Value =
        serde_json::from_str(&canonical(&operation(&long_text, cwd))).unwrap();
    assert!(long["command"].is_null());
    assert_eq!(long["command_len"], long_text.len());
    assert_eq!(long["command_sha256"], hex(long_text.as_bytes()));
    // The document stays far below the module's limit whatever the command size.
    let huge = "y".repeat(1 << 20);
    assert!(canonical(&operation(&huge, cwd)).len() < 4096);
    // A longer command still differs from another one of the same length.
    let other = "z".repeat(INLINE_COMMAND_BYTES + 1);
    assert_ne!(
        canonical(&operation(&long_text, cwd)),
        canonical(&operation(&other, cwd))
    );
}

#[cfg(unix)]
#[test]
fn a_working_directory_that_is_not_text_has_no_document() {
    use std::os::unix::ffi::OsStrExt;
    let cwd = Path::new(std::ffi::OsStr::from_bytes(b"/bad/\xff"));
    assert!(operation("ls", cwd).canonical().is_none());
}
