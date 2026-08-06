//! WeChat webview-scan ingest normalization.
//!
//! The Tauri shell's `wechat_scanner` scrapes the embedded WeChat CEF
//! webview via CDP and hands the raw DOM snapshot to this module, which
//! validates it and normalizes it into the memory-doc ingest envelope the
//! rest of the system consumes. The shell owns capture; core owns the
//! normalization + persistence contract.
//!
//! (The former webview cookie-login heuristic — `ops::detect_webview_logins`
//! — was removed as unused; only the WeChat ingest surface remains, kept in
//! core because it produces the shared memory-ingest envelope.)

pub mod wechat_ingest;

#[cfg(test)]
#[path = "wechat_ingest_tests.rs"]
mod tests;

pub use wechat_ingest::{
    list_ingest_envelope, list_ingest_payload, memory_doc_ingest_list_snapshot,
    memory_doc_ingest_peer_transcript, validate_scan, WechatChatRow, WechatMessageRow,
    WechatScanPayload,
};
