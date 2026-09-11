use super::{candidates_for, host_candidates, parse_glibc_version};

#[test]
fn a_modern_linux_host_prefers_the_newer_build_but_can_fall_back() {
    assert_eq!(
        candidates_for("linux", "x86_64", Some((2, 39))),
        vec!["ubuntu-24.04-x86_64", "ubuntu-22.04-x86_64"]
    );
    assert_eq!(
        candidates_for("linux", "aarch64", Some((2, 41))),
        vec!["ubuntu-24.04-arm64", "ubuntu-22.04-arm64"]
    );
}

#[test]
fn an_older_linux_host_is_offered_only_what_its_glibc_can_load() {
    // Offering the 24.04 build here would fail at dlopen with a symbol
    // version error, which is a worse outcome than not offering it.
    assert_eq!(
        candidates_for("linux", "x86_64", Some((2, 35))),
        vec!["ubuntu-22.04-x86_64"]
    );
}

#[test]
fn a_linux_host_below_every_published_floor_gets_nothing() {
    assert!(candidates_for("linux", "x86_64", Some((2, 31))).is_empty());
}

#[test]
fn a_non_glibc_linux_host_gets_nothing() {
    // musl. No published artifact targets it; guessing produces a dlopen
    // failure at first use rather than a clear answer now.
    assert!(candidates_for("linux", "x86_64", None).is_empty());
}

#[test]
fn macos_prefers_the_newer_build_and_falls_back() {
    assert_eq!(
        candidates_for("macos", "aarch64", None),
        vec!["macos-26-arm64", "macos-15-arm64"]
    );
    assert_eq!(
        candidates_for("macos", "x86_64", None),
        vec!["macos-26-x86_64", "macos-15-x86_64"]
    );
}

#[test]
fn windows_arm64_has_only_the_windows_11_build() {
    assert_eq!(
        candidates_for("windows", "aarch64", None),
        vec!["windows-11-arm64"]
    );
    assert_eq!(
        candidates_for("windows", "x86_64", None),
        vec!["windows-2025-x86_64", "windows-2022-x86_64"]
    );
}

#[test]
fn an_unsupported_os_or_architecture_gets_nothing() {
    assert!(candidates_for("freebsd", "x86_64", Some((2, 39))).is_empty());
    assert!(candidates_for("linux", "riscv64", Some((2, 39))).is_empty());
    assert!(candidates_for("macos", "powerpc", None).is_empty());
}

#[test]
fn glibc_versions_parse_with_and_without_a_patch_level() {
    assert_eq!(parse_glibc_version("2.39"), Some((2, 39)));
    assert_eq!(parse_glibc_version("2.39.1"), Some((2, 39)));
    assert_eq!(parse_glibc_version(" 2.35 "), Some((2, 35)));
    // Ubuntu ships versions like "2.39-0ubuntu8.3".
    assert_eq!(parse_glibc_version("2.39-0ubuntu8.3"), Some((2, 39)));
    assert_eq!(parse_glibc_version("garbage"), None);
    assert_eq!(parse_glibc_version("2"), None);
}

#[test]
fn the_running_host_resolves_without_panicking() {
    // Whatever this machine is, asking must be safe and must agree with the
    // pure table for its own os/arch.
    let live = host_candidates();
    let expected = candidates_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        super::glibc_version(),
    );
    assert_eq!(live, expected);
}
