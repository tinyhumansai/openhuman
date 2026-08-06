//! Platform auto-detection. Picks the strongest available backend.

use std::sync::Arc;

use super::jail::JailBackend;
use super::noop::NoopBackend;

pub fn pick_backend() -> Arc<dyn JailBackend> {
    #[cfg(target_os = "linux")]
    {
        let lb = super::linux::LandlockBackend::new();
        if lb.is_available() {
            log::info!("[cwd_jail] backend=landlock");
            return Arc::new(lb);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let sb = super::macos::SeatbeltBackend::new();
        if sb.is_available() {
            log::info!("[cwd_jail] backend=seatbelt");
            return Arc::new(sb);
        }
    }
    #[cfg(target_os = "windows")]
    {
        let ac = super::windows::AppContainerBackend::new();
        if ac.is_available() {
            log::info!("[cwd_jail] backend=appcontainer");
            return Arc::new(ac);
        }
    }
    log::warn!("[cwd_jail] no OS sandbox available, falling back to noop");
    Arc::new(NoopBackend)
}
