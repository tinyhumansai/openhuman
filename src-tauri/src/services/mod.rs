pub mod session_service;
pub mod socket_service;

// TDLib modules - desktop only (requires tdlib-rs which isn't available on mobile)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod tdlib;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod tdlib_v8;

#[cfg(desktop)]
pub mod notification_service;

// Local LLM inference - desktop only (llama.cpp requires native C++ compilation)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod llama;
