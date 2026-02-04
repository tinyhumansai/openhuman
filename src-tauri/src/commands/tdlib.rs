//! TDLib Tauri Commands
//!
//! These commands provide TDLib access via Tauri's invoke() system.
//! On desktop, they delegate to the TdLibManager singleton.
//! On Android, they use JNI to call the TDLib Android library.

use serde_json::Value;

/// Create a TDLib client with the given data directory.
///
/// # Arguments
/// * `data_dir` - Path to store TDLib data files
///
/// # Returns
/// Client ID (always 1 for singleton pattern)
#[tauri::command]
pub async fn tdlib_create_client(data_dir: String) -> Result<i32, String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use crate::services::tdlib::TDLIB_MANAGER;
        let path = std::path::PathBuf::from(data_dir);
        TDLIB_MANAGER.create_client(path)
    }

    #[cfg(target_os = "android")]
    {
        // Android: Use JNI bridge (to be implemented)
        log::info!("[tdlib-android] Creating client with data_dir: {}", data_dir);
        // TODO: Call TdLibBridge.createClient() via JNI
        Err("TDLib Android bridge not yet implemented".to_string())
    }

    #[cfg(target_os = "ios")]
    {
        let _ = data_dir;
        Err("TDLib is not supported on iOS".to_string())
    }
}

/// Send a request to TDLib and wait for the response.
///
/// # Arguments
/// * `request` - TDLib API request object with @type field
///
/// # Returns
/// TDLib response object
#[tauri::command]
pub async fn tdlib_send(request: Value) -> Result<Value, String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use crate::services::tdlib::TDLIB_MANAGER;
        TDLIB_MANAGER.send(request).await
    }

    #[cfg(target_os = "android")]
    {
        // Android: Use JNI bridge (to be implemented)
        log::info!("[tdlib-android] Sending request: {:?}", request);
        // TODO: Call TdLibBridge.send() via JNI
        Err("TDLib Android bridge not yet implemented".to_string())
    }

    #[cfg(target_os = "ios")]
    {
        let _ = request;
        Err("TDLib is not supported on iOS".to_string())
    }
}

/// Receive the next update from TDLib (with timeout).
///
/// # Arguments
/// * `timeout_ms` - Timeout in milliseconds
///
/// # Returns
/// Update object or null if timeout
#[tauri::command]
pub async fn tdlib_receive(timeout_ms: u32) -> Result<Option<Value>, String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use crate::services::tdlib::TDLIB_MANAGER;
        Ok(TDLIB_MANAGER.receive(timeout_ms).await)
    }

    #[cfg(target_os = "android")]
    {
        // Android: Use JNI bridge (to be implemented)
        log::debug!("[tdlib-android] Receiving with timeout: {}ms", timeout_ms);
        // TODO: Call TdLibBridge.receive() via JNI
        Err("TDLib Android bridge not yet implemented".to_string())
    }

    #[cfg(target_os = "ios")]
    {
        let _ = timeout_ms;
        Err("TDLib is not supported on iOS".to_string())
    }
}

/// Destroy the TDLib client and clean up resources.
#[tauri::command]
pub async fn tdlib_destroy() -> Result<(), String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use crate::services::tdlib::TDLIB_MANAGER;
        TDLIB_MANAGER.destroy().await
    }

    #[cfg(target_os = "android")]
    {
        // Android: Use JNI bridge (to be implemented)
        log::info!("[tdlib-android] Destroying client");
        // TODO: Call TdLibBridge.destroy() via JNI
        Err("TDLib Android bridge not yet implemented".to_string())
    }

    #[cfg(target_os = "ios")]
    {
        Err("TDLib is not supported on iOS".to_string())
    }
}

/// Check if TDLib is available on the current platform.
#[tauri::command]
pub fn tdlib_is_available() -> bool {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        true
    }

    #[cfg(target_os = "android")]
    {
        // Android: TDLib is available via JNI bridge
        true
    }

    #[cfg(target_os = "ios")]
    {
        false
    }
}
