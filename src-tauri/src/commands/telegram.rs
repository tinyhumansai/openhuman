use crate::utils::config::get_telegram_widget_url;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

/// Start Telegram login flow by opening the widget in system browser
/// The widget will redirect back to the app via deep link after auth
#[tauri::command]
pub async fn start_telegram_login(app: AppHandle) -> Result<(), String> {
    let url = get_telegram_widget_url();

    // Open in system browser using the opener plugin
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    Ok(())
}

/// Start Telegram login with a custom backend URL
#[tauri::command]
pub async fn start_telegram_login_with_url(
    app: AppHandle,
    backend_url: String,
) -> Result<(), String> {
    let url = format!(
        "{}/auth/telegram-widget?redirect=alphahuman://auth",
        backend_url
    );

    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    Ok(())
}
