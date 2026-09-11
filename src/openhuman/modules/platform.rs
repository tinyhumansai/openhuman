//! Choosing which published artifact belongs to this host.
//!
//! A target triple is not enough. tinybus admits a module only if its target,
//! pointer width, endianness and toolchain all match, and a `.so` built against
//! glibc 2.39 does not load on a host with glibc 2.35 — the failure is a
//! `dlopen` error about a missing symbol version, not something the ABI gate can
//! phrase helpfully. Releases therefore publish per-distro artifacts
//! (`ubuntu-22.04-x86_64` beside `ubuntu-24.04-x86_64`), and this module decides
//! which one to reach for.
//!
//! It returns an ordered list rather than a single answer. The newest build that
//! could work is tried first, and an admission failure falls through to the next
//! candidate: a host newer than every published artifact runs the newest one,
//! which is the case that works, while a host older than all of them gets the
//! oldest, which is the case most likely to.
//!
//! Every path here is pure — `std::env::consts` and, on Linux, the glibc version
//! string. That is what makes the whole table unit-testable on one machine.

/// Ordered artifact keys for this host, newest first.
///
/// Empty when no published artifact can run here, which is the honest answer for
/// a musl or BSD host: the alternative is downloading something that cannot
/// possibly `dlopen`.
#[must_use]
pub fn host_candidates() -> Vec<String> {
    candidates_for(
        std::env::consts::OS,
        std::env::consts::ARCH,
        glibc_version(),
    )
}

/// The pure half of [`host_candidates`], with the host facts injected.
///
/// `glibc` is `None` off Linux, and `None` on a Linux host whose libc is not
/// glibc — which is the distinction that decides whether any Linux artifact is
/// usable at all.
#[must_use]
pub fn candidates_for(os: &str, arch: &str, glibc: Option<(u32, u32)>) -> Vec<String> {
    let arch_key = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        _ => return Vec::new(),
    };

    match os {
        // Releases are cut on the two maintained LTS images. A host at or above
        // the newer one takes the newer build; anything older takes the older
        // build, which is the one whose glibc floor it can satisfy.
        "linux" => {
            let Some((major, minor)) = glibc else {
                // musl, or a libc we could not identify. No published artifact
                // targets it, and guessing produces a dlopen failure at first
                // use rather than a clear answer now.
                return Vec::new();
            };
            if (major, minor) >= (2, 39) {
                vec![
                    format!("ubuntu-24.04-{arch_key}"),
                    format!("ubuntu-22.04-{arch_key}"),
                ]
            } else if (major, minor) >= (2, 35) {
                vec![format!("ubuntu-22.04-{arch_key}")]
            } else {
                // Older than every published floor. Offering the oldest build
                // anyway would fail at dlopen with a symbol-version error.
                Vec::new()
            }
        }
        // macOS artifacts are forward-compatible, so the ordering is newest
        // first and an older host falls through to the older build.
        "macos" => vec![
            format!("macos-26-{arch_key}"),
            format!("macos-15-{arch_key}"),
        ],
        "windows" => match arch_key {
            // The arm64 build is published only against Windows 11.
            "arm64" => vec!["windows-11-arm64".to_string()],
            _ => vec![
                "windows-2025-x86_64".to_string(),
                "windows-2022-x86_64".to_string(),
            ],
        },
        _ => Vec::new(),
    }
}

/// The host's glibc version, or `None` if it is not glibc.
///
/// Gated on `target_env = "gnu"`, not merely on Linux: musl does not provide
/// this symbol, so a musl build would fail to *link* rather than fall through to
/// the `None` below. That fallback is what makes a musl host report "no
/// artifact for this platform" instead of downloading one it cannot load.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn glibc_version() -> Option<(u32, u32)> {
    // `gnu_get_libc_version` is the only reliable answer — parsing `ldd
    // --version` means spawning a process and reading localised output, and
    // reading a symlink target guesses. On musl the symbol is absent, which is
    // exactly the distinction that matters.
    unsafe extern "C" {
        fn gnu_get_libc_version() -> *const std::os::raw::c_char;
    }

    // SAFETY: `gnu_get_libc_version` takes no arguments and returns a pointer to
    // a static NUL-terminated string owned by libc, valid for the process
    // lifetime. On a non-glibc libc the symbol does not resolve and this code is
    // not reached, because the binary would fail to link against it — which is
    // why the musl case is handled by the returned `None` below rather than here.
    let raw = unsafe { gnu_get_libc_version() };
    if raw.is_null() {
        return None;
    }
    // SAFETY: non-null, NUL-terminated, static for the process lifetime.
    let version = unsafe { std::ffi::CStr::from_ptr(raw) }.to_str().ok()?;
    parse_glibc_version(version)
}

/// Every other target: not glibc, so there is no version to report.
///
/// Covers musl and every non-Linux host. Both want the same answer — `None`,
/// which `candidates_for` turns into an empty candidate list.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn glibc_version() -> Option<(u32, u32)> {
    None
}

/// Parse a `major.minor` glibc version string, ignoring any suffix.
fn parse_glibc_version(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    // A version like "2.39" has no third component; one like "2.39.1" does, and
    // the patch level never affects which artifact is usable.
    let minor_raw = parts.next()?;
    let minor = minor_raw
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()?;
    Some((major, minor))
}

#[cfg(test)]
#[path = "platform_tests.rs"]
mod tests;
