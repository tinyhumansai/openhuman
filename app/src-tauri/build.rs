use std::env;

fn main() {
    // Ensure Tauri ACL is regenerated when permissions or capabilities change.
    // Without this, cargo incremental builds may skip tauri-build and embed
    // stale ACL tables that miss newly added permission entries.
    println!("cargo:rerun-if-changed=permissions");
    println!("cargo:rerun-if-changed=capabilities");

    maybe_override_tauri_config_for_local_builds();
    tauri_build::build();
}

fn maybe_override_tauri_config_for_local_builds() {
    let profile = env::var("PROFILE").unwrap_or_default();
    let skip_resources = env::var("TAURI_SKIP_RESOURCES").is_ok() || profile != "release";

    if !skip_resources {
        return;
    }

    let mut merge_config = serde_json::json!({});
    if skip_resources {
        merge_config["bundle"]["resources"] = serde_json::json!([]);
        // Keep sidecars enabled for local/debug builds so the desktop host can
        // exercise the same core process launch path as packaged builds.
    }

    match serde_json::to_string(&merge_config) {
        Ok(json) => {
            env::set_var("TAURI_CONFIG", json);
            if skip_resources {
                println!("cargo:warning=TAURI resources disabled for local build");
            }
        }
        Err(err) => {
            println!("cargo:warning=Failed to serialize TAURI_CONFIG override: {err}");
        }
    }
}
