
/// Owns the cpal stream for the process lifetime.
///
/// Each callback converts the device's sample format to `f32` and forwards the
/// interleaved buffer untouched. Downmixing and resampling used to happen here;
/// they now happen in the async processor, because this runs on a realtime
/// audio thread where the right amount of work is the least possible.
fn capture_on_thread(
    tx: tokio::sync::mpsc::Sender<RawChunk>,
    setup_tx: &std::sync::mpsc::SyncSender<Result<CaptureFormat, String>>,
) -> Result<(), String> {
    use crate::openhuman::desktop::accessibility::{detect_microphone_permission, PermissionState};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, StreamConfig};

    // Surface the mic permission state explicitly — a denied/Unknown state is the
    // most common reason always-on "does nothing" and it differs per OS (macOS TCC
    // prompt, Windows privacy settings), so log it on every test build.
    let permission = detect_microphone_permission();
    log::info!("{LOG_PREFIX} microphone permission: {permission:?}");
    if matches!(permission, PermissionState::Denied) {
        log::warn!("{LOG_PREFIX} microphone permission denied — always-on cannot capture audio");
        return Err("microphone permission denied".to_string());
    }

    let host = cpal::default_host();
    log::info!("{LOG_PREFIX} audio host: {:?}", host.id());
    let device = host
        .default_input_device()
        .ok_or_else(|| "no default audio input device".to_string())?;
    let device_name = device.name().unwrap_or_else(|e| format!("<unknown: {e}>"));
    let supported = device
        .default_input_config()
        .map_err(|e| format!("no default input config: {e}"))?;
    let source_rate = supported.sample_rate().0;
    let channels = supported.channels();
    let sample_format = supported.sample_format();
    let stream_config: StreamConfig = supported.into();
    // Name + source rate/channels/format vary across M-chip, Intel, and Windows
    // mics; capturing them makes a "wrong device" or "unsupported format" failure
    // obvious from the log alone. We resample everything to 16 kHz mono downstream.
    log::info!(
        "{LOG_PREFIX} capture device ready name='{device_name}' rate={source_rate}->{TARGET_SAMPLE_RATE} channels={channels} format={sample_format:?}"
    );

    // Forward one raw interleaved chunk per callback.
    //
    // `try_send`, never `send`: this runs on a realtime audio thread where
    // blocking is a dropout, so a full queue drops the chunk rather than
    // waiting for the processor to catch up. Dropping the newest chunk is the
    // right end to lose — the queue ahead of it is older speech that is closer
    // to being transcribed.
    //
    // A send error also covers the processor being gone (shutdown), which is
    // why neither case is fatal here.
    let forward = move |samples: Vec<f32>| {
        if tx.try_send(RawChunk { samples }).is_err() {
            let dropped = DROPPED_CHUNKS.fetch_add(1, Ordering::Relaxed) + 1;
            // Log on a power-of-two schedule: a persistently overloaded
            // processor should be visible without logging inside every
            // callback once it starts.
            if dropped.is_power_of_two() {
                log::warn!("{LOG_PREFIX} capture queue full; dropped {dropped} chunk(s) so far");
            }
        }
    };

    let err_fn = |e| log::warn!("{LOG_PREFIX} cpal stream error: {e}");
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| forward(data.to_vec()),
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                forward(data.iter().map(|&s| f32::from(s) / 32768.0).collect());
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _| {
                forward(data.iter().map(|&s| f32::from(s) / 32768.0 - 1.0).collect());
            },
            err_fn,
            None,
        ),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("failed to build input stream: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("failed to start stream: {e}"))?;
    let _ = setup_tx.send(Ok(CaptureFormat {
        source_rate,
        channels,
    }));
    log::info!("{LOG_PREFIX} microphone stream live");

    // Keep the stream (and thus this thread) alive for the process lifetime.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// Poll the screen-lock state and drive [`PAUSED`] so always-on never captures
/// what is spoken at the lock screen. macOS-only for now (uses the Quartz
/// session dictionary); other platforms never pause (no lock signal yet).
fn spawn_lock_watcher() {
    #[cfg(target_os = "macos")]
    tokio::spawn(async move {
        let mut last = false;
        loop {
            let locked = macos_lock::is_screen_locked();
            if locked != last {
                log::info!(
                    "{LOG_PREFIX} screen {} → {}",
                    if locked { "locked" } else { "unlocked" },
                    if locked { "pausing" } else { "resuming" }
                );
                PAUSED.store(locked, Ordering::Relaxed);
                last = locked;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
    #[cfg(not(target_os = "macos"))]
    {
        log::info!("{LOG_PREFIX} screen-lock watcher unavailable on this platform");
    }
}

/// macOS screen-lock detection via the Quartz session dictionary.
///
/// `CGSessionCopyCurrentDictionary` exposes `CGSSessionScreenIsLocked`; we read
/// it defensively (null dict ⇒ no session, treated as locked; missing/odd value
/// ⇒ unlocked) and never assume the CF value's concrete type without checking.
#[cfg(target_os = "macos")]
mod macos_lock {
    use std::ffi::{c_void, CString};

    type CFTypeRef = *const c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSessionCopyCurrentDictionary() -> CFTypeRef;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFStringCreateWithCString(alloc: CFTypeRef, c: *const i8, enc: u32) -> CFTypeRef;
        fn CFGetTypeID(v: CFTypeRef) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(b: CFTypeRef) -> u8;
        fn CFNumberGetTypeID() -> usize;
        fn CFNumberGetValue(n: CFTypeRef, the_type: i64, out: *mut c_void) -> u8;
        fn CFRelease(v: CFTypeRef);
    }
    const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const KCF_NUMBER_SINT32: i64 = 3;

    /// True when the screen is locked (or there is no active GUI session).
    pub fn is_screen_locked() -> bool {
        // SAFETY: standard Quartz/CoreFoundation calls. Ownership: the session
        // dict and the key string are +1 (Create/Copy) and released here; the
        // dictionary value is borrowed and must not be released.
        unsafe {
            let dict = CGSessionCopyCurrentDictionary();
            if dict.is_null() {
                return true; // no session (loginwindow) — treat as locked
            }
            let Ok(key_c) = CString::new("CGSSessionScreenIsLocked") else {
                CFRelease(dict);
                return false;
            };
            let key = CFStringCreateWithCString(
                std::ptr::null(),
                key_c.as_ptr(),
                KCF_STRING_ENCODING_UTF8,
            );
            if key.is_null() {
                CFRelease(dict);
                return false;
            }
            let value = CFDictionaryGetValue(dict, key); // borrowed
            let locked = if value.is_null() {
                false
            } else {
                let tid = CFGetTypeID(value);
                if tid == CFBooleanGetTypeID() {
                    CFBooleanGetValue(value) != 0
                } else if tid == CFNumberGetTypeID() {
                    let mut n: i32 = 0;
                    CFNumberGetValue(value, KCF_NUMBER_SINT32, &mut n as *mut i32 as *mut c_void);
                    n != 0
                } else {
                    false
                }
            };
            CFRelease(key);
            CFRelease(dict);
            locked
        }
    }
}
