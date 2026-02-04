pub mod session_service;
pub mod socket_service;
pub mod tdlib;
pub mod tdlib_v8;

#[cfg(desktop)]
pub mod notification_service;

// Local LLM inference - desktop only (llama.cpp requires native C++ compilation)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod llama;
