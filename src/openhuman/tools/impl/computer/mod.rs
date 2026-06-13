mod automate;
mod ax_interact;
mod human_path;
// `pub(crate)` so the automate backend can reuse the shared input helpers
// (`run_hotkey`/`run_key`/`run_type_text`) without going through the tool.
pub(crate) mod keyboard;
mod main_thread;
mod mouse;

pub use automate::AutomateTool;
pub use ax_interact::AxInteractTool;
pub use keyboard::KeyboardTool;
pub use main_thread::{run_input_on_main, MainThreadInputOp, INPUT_ON_MAIN_THREAD_METHOD};
pub use mouse::MouseTool;
