//! Domain RPC handlers for curated memory. Adapter handlers in `schemas.rs`
//! deserialise params and call into these functions.

use crate::openhuman::curated_memory::{runtime::CuratedMemoryRuntime, MemoryStore};
use crate::rpc::RpcOutcome;
use serde_json::{json, Value};

/// Resolve `"memory"` / `"user"` strings to one of the runtime's stores.
fn pick<'a>(file: &str, rt: &'a CuratedMemoryRuntime) -> Result<&'a MemoryStore, String> {
    match file {
        "memory" => Ok(rt.memory.as_ref()),
        "user" => Ok(rt.user.as_ref()),
        other => Err(format!(
            "unknown curated memory file '{other}': expected 'memory' or 'user'"
        )),
    }
}

pub async fn handle_read(
    rt: &CuratedMemoryRuntime,
    file: String,
) -> Result<RpcOutcome<Value>, String> {
    let store = pick(&file, rt)?;
    let body = store.read().await.map_err(|e| format!("read: {e}"))?;
    Ok(RpcOutcome::new(
        json!({ "file": file, "body": body }),
        vec![],
    ))
}

pub async fn handle_add(
    rt: &CuratedMemoryRuntime,
    file: String,
    entry: String,
) -> Result<RpcOutcome<Value>, String> {
    let store = pick(&file, rt)?;
    store.add(&entry).await.map_err(|e| format!("add: {e}"))?;
    Ok(RpcOutcome::new(json!({ "file": file, "ok": true }), vec![]))
}

pub async fn handle_replace(
    rt: &CuratedMemoryRuntime,
    file: String,
    needle: String,
    replacement: String,
) -> Result<RpcOutcome<Value>, String> {
    if needle.is_empty() {
        return Err("needle must not be empty".into());
    }
    let store = pick(&file, rt)?;
    store
        .replace(&needle, &replacement)
        .await
        .map_err(|e| format!("replace: {e}"))?;
    Ok(RpcOutcome::new(json!({ "file": file, "ok": true }), vec![]))
}

pub async fn handle_remove(
    rt: &CuratedMemoryRuntime,
    file: String,
    needle: String,
) -> Result<RpcOutcome<Value>, String> {
    if needle.is_empty() {
        return Err("needle must not be empty".into());
    }
    let store = pick(&file, rt)?;
    store
        .remove(&needle)
        .await
        .map_err(|e| format!("remove: {e}"))?;
    Ok(RpcOutcome::new(json!({ "file": file, "ok": true }), vec![]))
}
