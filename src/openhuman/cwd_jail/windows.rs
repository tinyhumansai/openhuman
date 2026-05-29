//! Windows backend: AppContainer.
//!
//! AppContainer is Microsoft's strict process-isolation model — the same one
//! UWP apps and Edge's renderer use. Each container gets a unique SID; the
//! process can only touch filesystem objects whose DACL explicitly grants
//! that SID access. To jail an agent in `jail.root`:
//!
//! 1. `CreateAppContainerProfile` → derive a per-jail SID.
//! 2. Grant the SID `GENERIC_READ | GENERIC_WRITE | DELETE` on `jail.root`
//!    via `SetNamedSecurityInfoW` (additive ACE on the existing DACL).
//! 3. Build `STARTUPINFOEXW` with `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`.
//! 4. `CreateProcessW` with `EXTENDED_STARTUPINFO_PRESENT`.
//!
//! Things this backend deliberately does not do:
//!
//! - Network capability requests. By default an AppContainer has *no*
//!   network capabilities — we honor `jail.allow_net` by adding the
//!   `internetClient` capability (outbound client only, matching the
//!   `jail.allow_net` docstring of "Allow outbound network"). LAN /
//!   inbound capabilities are intentionally not granted; see
//!   `NET_CAPABILITY_NAMES` for the rationale.
//! - Persistent profile cleanup. We use a deterministic profile name
//!   derived from `jail.label` so reruns reuse the same SID. The host
//!   should call `DeleteAppContainerProfile` on uninstall — out of scope.
//!
//! Validated paths: this implementation has been compile-checked on
//! Windows targets but **must be tested on real Windows hardware** before
//! shipping. Win32 has many subtle wrong-trees you can bark up.

#![cfg(target_os = "windows")]

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use std::process::{Child, Command};
use std::ptr;

use windows_sys::core::PWSTR;
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, SET_ACCESS,
    SE_FILE_OBJECT, TRUSTEE_IS_GROUP, TRUSTEE_IS_SID, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{
    CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows_sys::Win32::Security::{
    DeriveCapabilitySidsFromName, FreeSid, ACL, DACL_SECURITY_INFORMATION, PSID,
    SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
};

// Well-known Win32 access masks. `windows-sys` has moved these constants
// between modules across releases (0.59 had GENERIC_READ in
// `Win32::Storage::FileSystem`, 0.60+ shuffled them around). Inlining the
// canonical values is both stable and avoids guessing which module the
// installed minor version exposes them through. Source: WinNT.h.
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const DELETE: u32 = 0x0001_0000;
const NO_INHERITANCE: u32 = 0;
/// SE_GROUP_ENABLED — marks a SID in a `SID_AND_ATTRIBUTES` entry as active.
/// Required for capability SIDs passed to AppContainer SECURITY_CAPABILITIES.
/// Source: WinNT.h.
const SE_GROUP_ENABLED: u32 = 0x0000_0004;

/// AppContainer capability names granted when `jail.allow_net == true`.
///
/// Per `jail.rs`, `allow_net` is documented as **"Allow outbound network"**
/// — so the capability set here is strictly outbound-client only:
///
/// - `internetClient` — outbound TCP/UDP to the public internet (client only).
///
/// Intentionally NOT granted (each would over-grant relative to the
/// documented `allow_net` contract):
///
/// - `internetClientServer` — would let the child `bind()` and accept
///   inbound public-internet connections.
/// - `privateNetworkClientServer` — would let the child `bind()` on the
///   LAN, granting inbound LAN reachability beyond the documented
///   outbound-only semantics. Excluding it keeps this backend faithful to
///   the `Jail.allow_net` doc-string contract ("Allow outbound network"),
///   which is the invariant being enforced here — not the actual network
///   posture of the other backends (macOS Seatbelt grants `allow default`,
///   i.e. full network, on `allow_net == true`, and the Linux Landlock
///   backend does not gate network at all).
///
/// If a future caller needs LAN inbound or public server roles, add a
/// dedicated `Jail` flag (e.g. `allow_private_lan_server`) and extend
/// this list conditionally — do NOT silently expand `allow_net`.
const NET_CAPABILITY_NAMES: &[&str] = &["internetClient"];
use windows_sys::Win32::System::Memory::{LocalAlloc, LPTR};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    UpdateProcThreadAttribute, EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTUPINFOEXW, STARTUPINFOW,
};

use super::jail::{Jail, JailBackend};

pub struct AppContainerBackend;

impl AppContainerBackend {
    pub fn new() -> Self {
        Self
    }
}

impl JailBackend for AppContainerBackend {
    fn name(&self) -> &'static str {
        "appcontainer"
    }

    fn is_available(&self) -> bool {
        // AppContainer is available on Windows 8+ and Server 2012+.
        // We could probe `CreateAppContainerProfile`'s availability via
        // GetProcAddress, but treating the target_os = "windows" cfg as
        // good enough — supported Windows versions are all 10+.
        true
    }

    fn spawn(&self, jail: &Jail, cmd: Command) -> io::Result<Child> {
        unsafe { spawn_in_container(jail, cmd) }
    }
}

unsafe fn spawn_in_container(jail: &Jail, cmd: Command) -> io::Result<Child> {
    // 1. Create or open the AppContainer profile and obtain its SID.
    let profile_name = sanitize_profile_name(&jail.label);
    let wide_name = to_wide(&profile_name);
    let display = to_wide(&format!("openhuman {}", jail.label));
    let desc = to_wide("openhuman spawnd agent process");

    let mut sid: PSID = ptr::null_mut();
    let hr = CreateAppContainerProfile(
        wide_name.as_ptr(),
        display.as_ptr(),
        desc.as_ptr(),
        ptr::null_mut(),
        0,
        &mut sid,
    );
    if hr != 0 {
        // 0x800700B7 = HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS).
        // In that case derive the SID from the existing profile.
        if hr as u32 == 0x800700B7 {
            let mut existing: PSID = ptr::null_mut();
            let drv = DeriveAppContainerSidFromAppContainerName(wide_name.as_ptr(), &mut existing);
            if drv != 0 {
                return Err(io::Error::from_raw_os_error(drv));
            }
            sid = existing;
        } else {
            return Err(io::Error::from_raw_os_error(hr));
        }
    }
    let _sid_guard = SidGuard(sid);

    // 2. Grant the container SID access to the root + read-only paths.
    grant_sid_access(&jail.root, sid, GENERIC_READ | GENERIC_WRITE | DELETE)?;
    for ro in &jail.read_only {
        grant_sid_access(ro, sid, GENERIC_READ)?;
    }

    // 3. Build SECURITY_CAPABILITIES. AppContainers start with no network
    //    access by default; honour `jail.allow_net` by deriving the
    //    capability SIDs in `NET_CAPABILITY_NAMES` via
    //    `DeriveCapabilitySidsFromName` and attaching them as
    //    SE_GROUP_ENABLED entries. `_cap_derivations` owns the
    //    OS-allocated SID buffers and frees them via Drop after
    //    CreateProcessW has captured the values into the child process.
    //    `cap_attrs` must outlive CreateProcessW (the attribute list
    //    references it by pointer) — it stays in scope until the end of
    //    `spawn_in_container`.
    let mut cap_attrs: Vec<SID_AND_ATTRIBUTES> = Vec::new();
    let mut _cap_derivations: Vec<CapabilityDerivation> = Vec::new();
    if jail.allow_net {
        let mut any_derived = false;
        for &name in NET_CAPABILITY_NAMES {
            match derive_capability(name) {
                Ok(deriv) => {
                    for &cap_sid in &deriv.capability_sids {
                        cap_attrs.push(SID_AND_ATTRIBUTES {
                            Sid: cap_sid,
                            Attributes: SE_GROUP_ENABLED,
                        });
                        any_derived = true;
                    }
                    _cap_derivations.push(deriv);
                }
                Err(e) => log::warn!(
                    "[cwd_jail] DeriveCapabilitySidsFromName({name}) failed: \
                     {e}; this capability will not be granted"
                ),
            }
        }
        if !any_derived {
            log::error!(
                "[cwd_jail] jail.allow_net=true but NO network capability \
                 could be derived; the AppContainer child will have no \
                 network access. Check Userenv.dll availability."
            );
        }
    }
    let mut caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        Capabilities: if cap_attrs.is_empty() {
            ptr::null_mut()
        } else {
            cap_attrs.as_mut_ptr()
        },
        CapabilityCount: cap_attrs.len() as u32,
        Reserved: 0,
    };

    // 4. Build STARTUPINFOEXW with the security-capabilities attribute.
    let mut size: usize = 0;
    InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size);
    let attr_buf = LocalAlloc(LPTR, size) as LPPROC_THREAD_ATTRIBUTE_LIST;
    if attr_buf.is_null() {
        return Err(io::Error::last_os_error());
    }
    let _attr_guard = AttrListGuard(attr_buf);
    if InitializeProcThreadAttributeList(attr_buf, 1, 0, &mut size) == 0 {
        return Err(io::Error::last_os_error());
    }
    if UpdateProcThreadAttribute(
        attr_buf,
        0,
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
        &mut caps as *mut _ as *mut _,
        std::mem::size_of::<SECURITY_CAPABILITIES>(),
        ptr::null_mut(),
        ptr::null_mut(),
    ) == 0
    {
        return Err(io::Error::last_os_error());
    }

    let mut si: STARTUPINFOEXW = std::mem::zeroed();
    si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    si.lpAttributeList = attr_buf;

    // 5. Build the command line and current directory.
    let cmdline = build_command_line(&cmd);
    let mut cmdline_w = to_wide(&cmdline);
    let cwd_w = cmd.get_current_dir().map(|p| to_wide(&p.to_string_lossy()));

    let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
    let ok = CreateProcessW(
        ptr::null(),
        cmdline_w.as_mut_ptr() as PWSTR,
        ptr::null_mut(),
        ptr::null_mut(),
        0, // bInheritHandles
        EXTENDED_STARTUPINFO_PRESENT,
        ptr::null_mut(),
        cwd_w.as_ref().map(|s| s.as_ptr()).unwrap_or(ptr::null()),
        &mut si as *mut _ as *mut STARTUPINFOW,
        &mut pi,
    );
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    // 6. Wrap the raw process handle in a std `Child`.
    //
    // std::process::Child has no public constructor from a raw HANDLE on
    // Windows. We close the thread handle (we don't need it) and leak the
    // process handle into an OwnedHandle so the caller can at least wait
    // via the OS. Returning a `Child` requires nightly's `FromRawHandle for
    // Child`, which is unstable. For now we close the process handle too
    // and return an error explaining the limitation — see TODO below.
    CloseHandle(pi.hThread);

    let _process_handle = OwnedHandle::from_raw_handle(pi.hProcess as _);

    // TODO: bridge OwnedHandle -> std::process::Child once
    // `std::os::windows::process::ChildExt::from_raw_handle` lands, OR
    // expose a custom `OpenhumanChild` from the cwd_jail module that
    // mirrors the bits of `Child` callers actually need (id, wait, kill).
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "AppContainer spawn succeeded but cannot yet be returned as std::process::Child; \
         see TODO in src/openhuman/cwd_jail/windows.rs",
    ))
}

unsafe fn grant_sid_access(path: &Path, sid: PSID, access: u32) -> io::Result<()> {
    let path_w = to_wide(&path.to_string_lossy());

    // First, fetch the *existing* DACL from the path. Passing
    // `ptr::null_mut()` as the old-ACL argument to `SetEntriesInAclW`
    // would build a fresh DACL containing only the AppContainer ACE,
    // and `SetNamedSecurityInfoW` would then replace the entire DACL —
    // locking out the owner / SYSTEM / Administrators. Merge instead.
    let mut sd_ptr: *mut std::ffi::c_void = ptr::null_mut();
    let mut existing_dacl: *mut ACL = ptr::null_mut();
    let mut dacl_present: i32 = 0;
    let rc = GetNamedSecurityInfoW(
        path_w.as_ptr(),
        SE_FILE_OBJECT,
        DACL_SECURITY_INFORMATION,
        ptr::null_mut(),
        ptr::null_mut(),
        &mut existing_dacl,
        ptr::null_mut(),
        &mut sd_ptr,
    );
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    let _sd_guard = LocalGuard(sd_ptr as HLOCAL);
    let _ = dacl_present;

    let mut ea: EXPLICIT_ACCESS_W = std::mem::zeroed();
    ea.grfAccessPermissions = access;
    ea.grfAccessMode = SET_ACCESS;
    ea.grfInheritance = NO_INHERITANCE;
    ea.Trustee = TRUSTEE_W {
        pMultipleTrustee: ptr::null_mut(),
        MultipleTrusteeOperation: 0,
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_GROUP,
        ptstrName: sid as *mut _,
    };

    let mut new_acl: *mut ACL = ptr::null_mut();
    let rc = SetEntriesInAclW(1, &mut ea, existing_dacl, &mut new_acl);
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    let _acl_guard = LocalGuard(new_acl as HLOCAL);

    let rc = SetNamedSecurityInfoW(
        path_w.as_ptr() as PWSTR,
        SE_FILE_OBJECT,
        DACL_SECURITY_INFORMATION,
        ptr::null_mut(),
        ptr::null_mut(),
        new_acl,
        ptr::null_mut(),
    );
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    Ok(())
}

fn build_command_line(cmd: &Command) -> String {
    // Minimal CommandLineToArgvW-compatible quoting. std's internal
    // `make_command_line` is private; this mirrors the same rules.
    let mut out = String::new();
    let prog = cmd.get_program().to_string_lossy().into_owned();
    push_arg(&mut out, &prog);
    for a in cmd.get_args() {
        out.push(' ');
        push_arg(&mut out, &a.to_string_lossy());
    }
    out
}

fn push_arg(out: &mut String, a: &str) {
    let needs_quotes = a.is_empty() || a.contains([' ', '\t', '"']);
    if !needs_quotes {
        out.push_str(a);
        return;
    }
    out.push('"');
    let mut backslashes = 0;
    for c in a.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                for _ in 0..(backslashes * 2 + 1) {
                    out.push('\\');
                }
                out.push('"');
                backslashes = 0;
            }
            _ => {
                for _ in 0..backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push(c);
            }
        }
    }
    for _ in 0..(backslashes * 2) {
        out.push('\\');
    }
    out.push('"');
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn sanitize_profile_name(label: &str) -> String {
    // AppContainer profile names must be ≤ 64 chars and use a restricted
    // charset. Keep ASCII alnum + dot; map everything else to `_`.
    let mut s: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    s.truncate(60);
    format!("openhuman.{s}")
}

struct SidGuard(PSID);
impl Drop for SidGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // MSDN: SIDs returned by both `CreateAppContainerProfile`
            // and `DeriveAppContainerSidFromAppContainerName` must be
            // freed with `FreeSid`. The older comment that said
            // `LocalFree` is "safer" was wrong — the buffers are not
            // necessarily LocalAlloc-backed, and using the wrong free
            // function corrupts the heap on some Windows builds.
            unsafe {
                FreeSid(self.0);
            }
        }
    }
}

struct AttrListGuard(LPPROC_THREAD_ATTRIBUTE_LIST);
impl Drop for AttrListGuard {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.0);
            LocalFree(self.0 as HLOCAL);
        }
    }
}

struct LocalGuard(HLOCAL);
impl Drop for LocalGuard {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

/// Owned wrapper around the SID arrays returned by
/// `DeriveCapabilitySidsFromName`. MSDN: each inner SID AND each array
/// itself is `LocalAlloc`-backed; `Drop` releases all of them in the
/// reverse order they were allocated.
///
/// The `capability_sids` are the ones we actually want for an
/// AppContainer's `SECURITY_CAPABILITIES.Capabilities`; the
/// `group_sids` come along because the Win32 API returns both, and we
/// hold them here purely so we can free them.
struct CapabilityDerivation {
    capability_sids: Vec<PSID>,
    capability_sids_ptr: *mut PSID,
    group_sids: Vec<PSID>,
    group_sids_ptr: *mut PSID,
}

impl Drop for CapabilityDerivation {
    fn drop(&mut self) {
        unsafe {
            for &sid in &self.capability_sids {
                if !sid.is_null() {
                    LocalFree(sid as HLOCAL);
                }
            }
            if !self.capability_sids_ptr.is_null() {
                LocalFree(self.capability_sids_ptr as HLOCAL);
            }
            for &sid in &self.group_sids {
                if !sid.is_null() {
                    LocalFree(sid as HLOCAL);
                }
            }
            if !self.group_sids_ptr.is_null() {
                LocalFree(self.group_sids_ptr as HLOCAL);
            }
        }
    }
}

/// Resolves a capability name (e.g. `"internetClient"`) to its SIDs via
/// `DeriveCapabilitySidsFromName`. The returned [`CapabilityDerivation`]
/// owns the OS allocations; drop it to free them. Use the
/// `capability_sids` slice as the SID values for
/// `SECURITY_CAPABILITIES.Capabilities` entries.
///
/// # Safety
/// FFI into `userenv.dll`. Safe to call from any thread on Windows.
unsafe fn derive_capability(name: &str) -> io::Result<CapabilityDerivation> {
    let name_w = to_wide(name);
    let mut group_sids: *mut PSID = ptr::null_mut();
    let mut group_count: u32 = 0;
    let mut cap_sids: *mut PSID = ptr::null_mut();
    let mut cap_count: u32 = 0;
    let ok = DeriveCapabilitySidsFromName(
        name_w.as_ptr(),
        &mut group_sids,
        &mut group_count,
        &mut cap_sids,
        &mut cap_count,
    );
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let capability_sids: Vec<PSID> = (0..cap_count as usize).map(|i| *cap_sids.add(i)).collect();
    let group_sids_vec: Vec<PSID> = (0..group_count as usize)
        .map(|i| *group_sids.add(i))
        .collect();
    Ok(CapabilityDerivation {
        capability_sids,
        capability_sids_ptr: cap_sids,
        group_sids: group_sids_vec,
        group_sids_ptr: group_sids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_capability_names_is_outbound_client_only() {
        // `Jail.allow_net` is documented as "Allow outbound network" in
        // `jail.rs`, so the granted capability set must be strictly
        // outbound-client.
        assert!(NET_CAPABILITY_NAMES.contains(&"internetClient"));
        // The following would over-grant inbound bind() rights and are
        // intentionally NOT exposed via the coarse `allow_net` switch.
        // A future caller that needs them should add a separate `Jail`
        // flag (e.g. `allow_private_lan_server`) and gate them on it.
        assert!(!NET_CAPABILITY_NAMES.contains(&"privateNetworkClientServer"));
        assert!(!NET_CAPABILITY_NAMES.contains(&"internetClientServer"));
    }

    #[test]
    fn derive_capability_resolves_well_known_internet_client() {
        // `internetClient` is one of the Windows manifest-defined
        // capabilities present on every supported Windows version
        // (10+). DeriveCapabilitySidsFromName must succeed and return
        // at least one non-null capability SID.
        let deriv = unsafe { derive_capability("internetClient") }
            .expect("internetClient must resolve on any supported Windows");
        assert!(
            !deriv.capability_sids.is_empty(),
            "expected at least one capability SID for internetClient"
        );
        for &sid in &deriv.capability_sids {
            assert!(!sid.is_null(), "capability SID must not be null");
        }
        // Dropping `deriv` here exercises the Drop impl that LocalFrees
        // every SID + array — a no-op for the test runner unless it
        // double-frees, which would crash the process.
    }

    #[test]
    fn sanitize_profile_name_keeps_alnum_and_dot() {
        let s = sanitize_profile_name("agent.delegate-7");
        assert!(s.starts_with("openhuman."));
        // hyphen mapped to underscore
        assert!(s.contains("agent.delegate_7"));
        assert!(s.len() <= 70); // "openhuman." (10) + label (≤60)
    }

    #[test]
    fn sanitize_profile_name_truncates_long_labels() {
        let long: String = "a".repeat(200);
        let s = sanitize_profile_name(&long);
        // Per the in-function comment: truncate to 60 then prefix.
        assert!(s.len() <= 70);
        assert!(s.starts_with("openhuman."));
    }
}
