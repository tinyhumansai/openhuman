//! SSE streaming support for the OpenAI-compatible provider.
//!
//! Converts a raw `reqwest::Response` byte stream into a typed
//! `StreamChunk` stream via Server-Sent Events parsing.

use crate::openhuman::providers::traits::{StreamChunk, StreamError, StreamResult};
use futures_util::{stream, StreamExt};

use super::compatible_parse::parse_sse_line;

/// Convert SSE byte stream to text chunks.
pub(crate) fn sse_bytes_to_chunks(
    response: reqwest::Response,
    count_tokens: bool,
) -> stream::BoxStream<'static, StreamResult<StreamChunk>> {
    // Create a channel to send chunks
    let (tx, rx) = tokio::sync::mpsc::channel::<StreamResult<StreamChunk>>(100);

    tokio::spawn(async move {
        // Buffer for incomplete lines
        let mut buffer = String::new();

        // Get response body as bytes stream
        match response.error_for_status_ref() {
            Ok(_) => {}
            Err(e) => {
                let _ = tx.send(Err(StreamError::Http(e))).await;
                return;
            }
        }

        let mut bytes_stream = response.bytes_stream();

        while let Some(item) = bytes_stream.next().await {
            match item {
                Ok(bytes) => {
                    // Convert bytes to string and process line by line
                    let text = match String::from_utf8(bytes.to_vec()) {
                        Ok(t) => t,
                        Err(e) => {
                            let _ = tx
                                .send(Err(StreamError::InvalidSse(format!(
                                    "Invalid UTF-8: {}",
                                    e
                                ))))
                                .await;
                            break;
                        }
                    };

                    buffer.push_str(&text);

                    // Process complete lines
                    while let Some(pos) = buffer.find('\n') {
                        let line = buffer.drain(..=pos).collect::<String>();
                        buffer = buffer[pos + 1..].to_string();

                        match parse_sse_line(&line) {
                            Ok(Some(content)) => {
                                let mut chunk = StreamChunk::delta(content);
                                if count_tokens {
                                    chunk = chunk.with_token_estimate();
                                }
                                if tx.send(Ok(chunk)).await.is_err() {
                                    return; // Receiver dropped
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                let _ = tx.send(Err(e)).await;
                                return;
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(StreamError::Http(e))).await;
                    break;
                }
            }
        }

        // Send final chunk
        let _ = tx.send(Ok(StreamChunk::final_chunk())).await;
    });

    // Convert channel receiver to stream
    stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|chunk| (chunk, rx))
    })
    .boxed()
}
