use tauri_plugin_deep_link::DeepLinkExt;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[derive(serde::Deserialize)]
struct ExchangeResponse {
    #[serde(rename = "sessionToken")]
    session_token: Option<String>,
    user: Option<serde_json::Value>,
    error: Option<String>,
}

#[tauri::command]
async fn exchange_token(backend_url: String, token: String) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/auth/desktop-exchange", backend_url);

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("ngrok-skip-browser-warning", "true")
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status().as_u16();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if status != 200 {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("Unknown error");
        return Err(format!("Exchange failed ({}): {}", status, error));
    }

    Ok(body)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            #[cfg(any(windows, target_os = "linux"))]
            {
                app.deep_link().register_all()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet, exchange_token])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
