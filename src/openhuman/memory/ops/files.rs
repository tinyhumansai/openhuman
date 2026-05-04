//! File-based memory RPC handlers (`ai_list_memory_files`,
//! `ai_read_memory_file`, `ai_write_memory_file`).
//!
//! All filesystem I/O here is performed via `tokio::fs` so the handlers stay
//! async-friendly and never block the executor.

use crate::openhuman::memory::{
    ApiEnvelope, ListMemoryFilesRequest, ListMemoryFilesResponse, ReadMemoryFileRequest,
    ReadMemoryFileResponse, WriteMemoryFileRequest, WriteMemoryFileResponse,
};
use crate::rpc::RpcOutcome;

use super::envelope::{envelope, memory_counts};
use super::helpers::{
    resolve_existing_memory_path, resolve_writable_memory_path, validate_memory_relative_path,
};

/// Lists files in a memory directory.
pub async fn ai_list_memory_files(
    request: ListMemoryFilesRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListMemoryFilesResponse>>, String> {
    validate_memory_relative_path(&request.relative_dir)?;
    let directory = resolve_existing_memory_path(&request.relative_dir).await?;
    if !directory.is_dir() {
        return Err(format!(
            "memory directory not found: {}",
            directory.display()
        ));
    }
    let mut files = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&directory)
        .await
        .map_err(|e| format!("read memory directory {}: {e}", directory.display()))?;
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| format!("read memory directory entry: {e}"))?
    {
        // Skip subdirectories and symlinks — `ai_read_memory_file` only
        // consumes regular file entries, and surfacing other entry kinds
        // here would just produce confusing follow-up read errors.
        let file_type = entry
            .file_type()
            .await
            .map_err(|e| format!("read memory directory entry type: {e}"))?;
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !file_name.is_empty() {
            files.push(file_name.to_string());
        }
    }
    files.sort();
    let count = files.len();
    Ok(envelope(
        ListMemoryFilesResponse {
            relative_dir: request.relative_dir,
            files,
            count,
        },
        Some(memory_counts([("num_files", count)])),
        None,
    ))
}

/// Reads the contents of a memory file.
pub async fn ai_read_memory_file(
    request: ReadMemoryFileRequest,
) -> Result<RpcOutcome<ApiEnvelope<ReadMemoryFileResponse>>, String> {
    let path = resolve_existing_memory_path(&request.relative_path).await?;
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| format!("read memory file {}: {e}", path.display()))?;
    Ok(envelope(
        ReadMemoryFileResponse {
            relative_path: request.relative_path,
            content,
        },
        None,
        None,
    ))
}

/// Writes content to a memory file.
pub async fn ai_write_memory_file(
    request: WriteMemoryFileRequest,
) -> Result<RpcOutcome<ApiEnvelope<WriteMemoryFileResponse>>, String> {
    let path = resolve_writable_memory_path(&request.relative_path).await?;
    tokio::fs::write(&path, request.content.as_bytes())
        .await
        .map_err(|e| format!("write memory file {}: {e}", path.display()))?;
    let bytes_written = request.content.len();
    Ok(envelope(
        WriteMemoryFileResponse {
            relative_path: request.relative_path,
            written: true,
            bytes_written,
        },
        None,
        None,
    ))
}
