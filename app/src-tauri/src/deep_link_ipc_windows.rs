//! Pre-CEF deep-link forwarding for Windows.
//!
//! `openhuman://` OAuth callbacks launch a second `OpenHuman.exe` with the
//! URL in argv. The Windows pre-CEF mutex guard exits secondaries before Tauri's
//! single-instance/deep-link plugins can run, so the URL must be forwarded here.

#![cfg(target_os = "windows")]

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_PIPE_CONNECTED, HANDLE,
        INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_GENERIC_WRITE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
    },
    System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    },
};

const PIPE_NAME: &str = r"\\.\pipe\com.openhuman.app-deeplink";
const FORWARD_RETRY_ATTEMPTS: usize = 40;
const FORWARD_RETRY_DELAY: Duration = Duration::from_millis(50);

pub(crate) enum ForwardResult {
    Forwarded,
    NoPrimary,
    NoUrls,
}

pub(crate) fn collect_deep_link_urls_from_args<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .skip(1)
        .filter_map(|arg| {
            let arg = arg.as_ref();
            arg.starts_with("openhuman://").then(|| arg.to_string())
        })
        .collect()
}

pub(crate) fn extract_deep_link_urls() -> Vec<String> {
    collect_deep_link_urls_from_args(std::env::args())
}

pub(crate) fn try_forward_deep_links() -> ForwardResult {
    let urls = extract_deep_link_urls();
    if urls.is_empty() {
        return ForwardResult::NoUrls;
    }

    log::info!(
        "[deep-link-ipc] secondary: found {} deep-link URL(s), forwarding to primary",
        urls.len()
    );

    for attempt in 1..=FORWARD_RETRY_ATTEMPTS {
        match open_pipe_for_write() {
            Some(handle) => {
                let result = write_urls(handle, &urls);
                unsafe {
                    CloseHandle(handle);
                }
                if result {
                    log::info!(
                        "[deep-link-ipc] secondary: forwarded {} deep-link URL(s)",
                        urls.len()
                    );
                    return ForwardResult::Forwarded;
                }
            }
            None if attempt < FORWARD_RETRY_ATTEMPTS => {
                std::thread::sleep(FORWARD_RETRY_DELAY);
            }
            None => {}
        }
    }

    log::warn!(
        "[deep-link-ipc] secondary: primary pipe was unavailable; deep-link URL was not forwarded"
    );
    ForwardResult::NoPrimary
}

static PENDING_URLS: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
static LIVE_HANDLER: OnceLock<Mutex<Option<Box<dyn Fn(String) + Send + Sync>>>> = OnceLock::new();

fn pending_queue() -> &'static Arc<Mutex<Vec<String>>> {
    PENDING_URLS.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

fn live_handler() -> &'static Mutex<Option<Box<dyn Fn(String) + Send + Sync>>> {
    LIVE_HANDLER.get_or_init(|| Mutex::new(None))
}

pub(crate) fn redact_url_for_log(url: &str) -> String {
    url.parse::<url::Url>()
        .map(|mut parsed| {
            parsed.set_query(None);
            parsed.set_fragment(None);
            parsed.to_string()
        })
        .unwrap_or_else(|_| "<invalid deep link>".to_string())
}

fn dispatch_url(url: String) {
    if let Ok(guard) = live_handler().lock() {
        if let Some(ref handler) = *guard {
            handler(url);
            return;
        }
    }

    if let Ok(mut queue) = pending_queue().lock() {
        log::debug!(
            "[deep-link-ipc] queued URL before setup: {}",
            redact_url_for_log(&url)
        );
        queue.push(url);
    }
}

pub(crate) struct DeepLinkPipeGuard {
    stop: Arc<AtomicBool>,
}

impl Drop for DeepLinkPipeGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = open_pipe_for_write() {
            unsafe {
                CloseHandle(handle);
            }
        }
    }
}

pub(crate) fn bind_and_listen() -> Option<DeepLinkPipeGuard> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);

    match std::thread::Builder::new()
        .name("deep-link-ipc-windows".into())
        .spawn(move || listener_loop(thread_stop))
    {
        Ok(_) => {
            log::info!("[deep-link-ipc] primary: named pipe listener started");
            Some(DeepLinkPipeGuard { stop })
        }
        Err(err) => {
            log::warn!("[deep-link-ipc] failed to spawn listener thread: {err}");
            None
        }
    }
}

fn listener_loop(stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        let pipe = match create_pipe_for_read() {
            Some(pipe) => pipe,
            None => {
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
        };

        let connected = unsafe { ConnectNamedPipe(pipe, std::ptr::null_mut()) != 0 }
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;

        if connected {
            for url in read_urls(pipe) {
                log::info!(
                    "[deep-link-ipc] primary: received deep-link URL: {}",
                    redact_url_for_log(&url)
                );
                dispatch_url(url);
            }
        }

        unsafe {
            CloseHandle(pipe);
        }
    }
}

fn pipe_name_wide() -> Vec<u16> {
    PIPE_NAME.encode_utf16().chain(std::iter::once(0)).collect()
}

fn create_pipe_for_read() -> Option<HANDLE> {
    let name = pipe_name_wide();
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_INBOUND,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            4096,
            4096,
            0,
            std::ptr::null(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        log::warn!(
            "[deep-link-ipc] CreateNamedPipeW failed with os error {}",
            unsafe { GetLastError() }
        );
        None
    } else {
        Some(handle)
    }
}

fn open_pipe_for_write() -> Option<HANDLE> {
    let name = pipe_name_wide();
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };

    (handle != INVALID_HANDLE_VALUE).then_some(handle)
}

fn write_urls(handle: HANDLE, urls: &[String]) -> bool {
    let payload = urls.join("\n") + "\n";
    let bytes = payload.as_bytes();
    let mut written = 0u32;
    let ok = unsafe {
        WriteFile(
            handle,
            bytes.as_ptr(),
            bytes.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        )
    } != 0;

    ok && written == bytes.len() as u32
}

fn read_urls(handle: HANDLE) -> Vec<String> {
    let mut all = Vec::new();
    let mut buf = [0u8; 1024];

    loop {
        let mut read = 0u32;
        let ok = unsafe {
            ReadFile(
                handle,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut read,
                std::ptr::null_mut(),
            )
        } != 0;

        if !ok {
            let err = unsafe { GetLastError() };
            if err != ERROR_BROKEN_PIPE {
                log::debug!("[deep-link-ipc] ReadFile stopped with os error {err}");
            }
            break;
        }

        if read == 0 {
            break;
        }

        all.extend_from_slice(&buf[..read as usize]);
    }

    String::from_utf8_lossy(&all)
        .lines()
        .filter(|line| line.starts_with("openhuman://"))
        .map(ToOwned::to_owned)
        .collect()
}

pub(crate) fn drain_pending_urls<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    use tauri::Emitter;

    let app_clone = app.clone();
    if let Ok(mut guard) = live_handler().lock() {
        *guard = Some(Box::new(move |url: String| {
            if let Ok(parsed) = url.parse::<url::Url>() {
                if let Err(err) = app_clone.emit("deep-link://new-url", &vec![parsed]) {
                    log::warn!("[deep-link-ipc] failed to emit deep-link event: {err}");
                }
            } else {
                log::warn!("[deep-link-ipc] received malformed deep-link URL");
            }
        }));
    }

    let pending = pending_queue()
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default();

    if !pending.is_empty() {
        log::info!(
            "[deep-link-ipc] draining {} queued deep-link URL(s)",
            pending.len()
        );
    }

    for url in pending {
        if let Ok(parsed) = url.parse::<url::Url>() {
            if let Err(err) = app.emit("deep-link://new-url", &vec![parsed]) {
                log::warn!("[deep-link-ipc] failed to emit queued deep-link URL: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_deep_link_urls_filters_args() {
        let urls = collect_deep_link_urls_from_args([
            "OpenHuman.exe",
            "openhuman://auth?token=secret&key=auth",
            "--flag",
            "https://example.test",
            "openhuman://oauth/success?integrationId=abc",
        ]);

        assert_eq!(
            urls,
            vec![
                "openhuman://auth?token=secret&key=auth",
                "openhuman://oauth/success?integrationId=abc"
            ]
        );
    }

    #[test]
    fn redact_url_removes_query_and_fragment() {
        assert_eq!(
            redact_url_for_log("openhuman://auth?token=secret&key=auth#frag"),
            "openhuman://auth"
        );
    }

    #[test]
    fn pipe_name_is_stable_and_app_scoped() {
        assert_eq!(PIPE_NAME, r"\\.\pipe\com.openhuman.app-deeplink");
    }
}
