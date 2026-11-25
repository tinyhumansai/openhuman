//! Platform-specific descriptor/handle-bound attachment opens.

#[cfg(unix)]
pub(crate) fn secure_open(
    path: &std::path::Path,
    root: &std::path::Path,
) -> std::io::Result<std::fs::File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};

    let (base_path, relative) = if let Ok(relative) = path.strip_prefix(root) {
        (root, relative)
    } else {
        let relative = path.strip_prefix("/").map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path is not absolute")
        })?;
        (std::path::Path::new("/"), relative)
    };
    let filesystem_root = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open("/")?;
    let mut current = filesystem_root;
    for component in base_path
        .strip_prefix("/")
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "root is not absolute"))?
        .components()
    {
        let std::path::Component::Normal(name) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "attachment root contains an invalid component",
            ));
        };
        let name = CString::new(name.as_bytes()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "attachment root contains NUL",
            )
        })?;
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        current = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "attachment path names a directory",
        ));
    }
    for (index, component) in components.iter().enumerate() {
        let std::path::Component::Normal(name) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "attachment path contains an invalid component",
            ));
        };
        let name = CString::new(name.as_bytes()).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "attachment path contains NUL",
            )
        })?;
        let final_component = index + 1 == components.len();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if final_component { libc::O_NONBLOCK } else { 0 }
            | if final_component {
                0
            } else {
                libc::O_DIRECTORY
            };
        // Each lookup is relative to an already-open directory descriptor and
        // refuses reparse/symlink traversal, so a path swap cannot redirect the
        // eventual read after policy validation.
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        current = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    Ok(current)
}

#[cfg(windows)]
pub(crate) fn secure_open(
    path: &std::path::Path,
    _root: &std::path::Path,
) -> std::io::Result<std::fs::File> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, VOLUME_NAME_DOS,
    };

    use std::os::windows::ffi::OsStringExt;
    let expected = path.to_path_buf();
    let file = std::fs::File::open(path)?;
    let handle = file.as_raw_handle();
    let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_DOS;
    let required =
        unsafe { GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, flags) } as usize;
    if required == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut buffer = vec![0u16; required + 1];
    let written = unsafe {
        GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, flags)
    } as usize;
    if written == 0 || written >= buffer.len() {
        return Err(std::io::Error::last_os_error());
    }
    let opened = std::ffi::OsString::from_wide(&buffer[..written]);
    if !normalize_windows_path_for_comparison(&opened.to_string_lossy()).eq_ignore_ascii_case(
        &normalize_windows_path_for_comparison(&expected.to_string_lossy()),
    ) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "attachment path changed during secure open",
        ));
    }
    Ok(file)
}

#[cfg(any(windows, test))]
pub(crate) fn normalize_windows_path_for_comparison(value: &str) -> String {
    value
        .strip_prefix("\\\\?\\UNC\\")
        .map(|unc| format!("\\\\{unc}"))
        .or_else(|| value.strip_prefix("\\\\?\\").map(str::to_owned))
        .unwrap_or_else(|| value.to_owned())
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn secure_open(
    _path: &std::path::Path,
    _root: &std::path::Path,
) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "secure attachment reads are unsupported on this platform",
    ))
}
