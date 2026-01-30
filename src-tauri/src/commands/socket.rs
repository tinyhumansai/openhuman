use crate::models::socket::{ConnectionStatus, SocketState};
use crate::services::socket_service::SOCKET_SERVICE;
use tauri::AppHandle;

/// Request the frontend to connect to the socket server
#[tauri::command]
pub fn socket_connect(
    app: AppHandle,
    backend_url: String,
    token: String,
) -> Result<(), String> {
    // Set app handle for event emission
    SOCKET_SERVICE.set_app_handle(app);

    // Request frontend to connect
    SOCKET_SERVICE.request_connect(&backend_url, &token)
}

/// Request the frontend to disconnect from the socket server
#[tauri::command]
pub fn socket_disconnect() -> Result<(), String> {
    SOCKET_SERVICE.request_disconnect()
}

/// Get current socket state
#[tauri::command]
pub fn get_socket_state() -> SocketState {
    SOCKET_SERVICE.get_state()
}

/// Check if socket is connected
#[tauri::command]
pub fn is_socket_connected() -> bool {
    SOCKET_SERVICE.is_connected()
}

/// Report socket connected (called by frontend)
#[tauri::command]
pub fn report_socket_connected(socket_id: Option<String>) {
    SOCKET_SERVICE.report_connected(socket_id);
}

/// Report socket disconnected (called by frontend)
#[tauri::command]
pub fn report_socket_disconnected() {
    SOCKET_SERVICE.report_disconnected();
}

/// Report socket error (called by frontend)
#[tauri::command]
pub fn report_socket_error(error: String) {
    SOCKET_SERVICE.report_error(&error);
}

/// Update socket status (called by frontend)
#[tauri::command]
pub fn update_socket_status(status: String, socket_id: Option<String>) {
    let status = match status.as_str() {
        "connected" => ConnectionStatus::Connected,
        "connecting" => ConnectionStatus::Connecting,
        "reconnecting" => ConnectionStatus::Reconnecting,
        "error" => ConnectionStatus::Error,
        _ => ConnectionStatus::Disconnected,
    };
    SOCKET_SERVICE.update_status(status, socket_id);
}
