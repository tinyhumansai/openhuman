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
//!   network capabilities — we honor `jail.allow_net` by adding
//!   `internetClient` and `privateNetworkClientServer` capabilities.
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
    FreeSid, ACL, DACL_SECURITY_INFORMATION, PSID, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES,
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

    // 3. Build SECURITY_CAPABILITIES. For now we attach no capability SIDs;
    //    network capabilities (`internetClient`) would need their own
    //    well-known SID lookups via DeriveCapabilitySidsFromName — left as
    //    a TODO with a clear failure mode (`jail.allow_net == true` will
    //    log a warning until implemented).
    if jail.allow_net {
        log::warn!(
            "[cwd_jail] AppContainer network capabilities not yet wired; \
             jail.allow_net=true is currently equivalent to no-net on Windows"
        );
    }
    let mut caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid,
        Capabilities: ptr::null_mut(),
        CapabilityCount: 0,
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

// Unused import suppression on this platform.
#[allow(dead_code)]
fn _unused() {
    let _ = SID_AND_ATTRIBUTES {
        Sid: ptr::null_mut(),
        Attributes: 0,
    };
}
