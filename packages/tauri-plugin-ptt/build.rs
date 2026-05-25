// Standard Tauri v2 plugin build script.
// Generates iOS Swift package metadata consumed by `tauri-plugin`.
const COMMANDS: &[&str] = &[
    "start_listening",
    "stop_listening",
    "speak",
    "cancel_speech",
    "list_voices",
];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .ios_path("ios")
        .try_build()
        .expect("failed to run tauri-plugin build");
}
