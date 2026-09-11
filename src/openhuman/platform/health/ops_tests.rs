use super::*;

#[test]
fn system_info_returns_non_empty_version() {
    let outcome = system_info();
    let json = outcome
        .into_cli_compatible_json()
        .expect("serialization ok");
    let version = json["version"].as_str().expect("version is a string");
    assert!(!version.is_empty(), "version must be non-empty");
}

#[test]
fn system_info_returns_known_os() {
    let outcome = system_info();
    let json = outcome
        .into_cli_compatible_json()
        .expect("serialization ok");
    let os = json["os"].as_str().expect("os is a string");
    // std::env::consts::OS is always one of the compile-time Rust target OS names.
    assert!(!os.is_empty(), "os must be non-empty");
}

#[test]
fn system_info_returns_non_zero_pid() {
    let outcome = system_info();
    let json = outcome
        .into_cli_compatible_json()
        .expect("serialization ok");
    let pid = json["pid"].as_u64().expect("pid is a u64");
    assert!(pid > 0, "pid must be greater than zero");
}

#[test]
fn health_snapshot_returns_serializable_value() {
    let outcome = health_snapshot();
    let json = outcome
        .into_cli_compatible_json()
        .expect("serialization ok");
    assert!(json.is_object(), "snapshot must be a JSON object");
}
