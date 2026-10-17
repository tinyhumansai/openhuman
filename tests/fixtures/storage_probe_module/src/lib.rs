//! A module that relays one raw call to the host's storage interface and
//! reports the answer, so a test can see exactly what the host decided for a
//! real loaded module.
//!
//! `Call` takes `[object, selector, request, confidential]` and returns
//! `{"ok": <reply>}` or `{"err": {"name": .., "message": ..}}`. `PutBlob`
//! takes `[key, len]` and sends an inline blob of `len` bytes the same way. The request is
//! forwarded untouched: the probe adds no scope, no module id and no grant,
//! because the host must not take any from a request.
//!
//! The streamed paths use the module-side storage client a real module links
//! (`tinystoragedrivers-busproxy`), so they cross the real peer-bound bus
//! streams and unicast watch signals:
//!
//! * `StreamPut [key, len]` uploads `len` bytes of `0xAB` by `PutStream`.
//! * `StreamGet [key]` downloads by `GetStream` and reports the length read and
//!   whether every byte was `0xAB`.
//! * `WatchPut [collection, id]` opens a watch, writes a document and reports
//!   the first notice the watch receives.
//! * `OpenStream [len]` opens a stream toward the host and returns its
//!   reference without using it; `UseStream [key, len]` then sends that stream
//!   by a raw `PutStream`. `RawGetStream [key]` returns the head and stream
//!   reference of a `GetStream` without accepting the stream.
use async_trait::async_trait;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use tinybus::{
    CallContext, Connection, Error, Interface, InterfaceName, MemberName, Result, StreamDescriptor,
    StreamWriter,
};
use tinystoragedrivers_busproxy::connect_storage;
use tinystoragedrivers_core::{BlobReader, Filter, StorageBackend, WatchNotice, WatchOptions};
use tokio::io::AsyncReadExt;

const STORAGE: &str = "ai.tinyhumans.tinystorage.Storage";
const PROBE: &str = "ai.tinyhumans.test.StorageProbe";

/// The byte every streamed body is made of.
const FILL: u8 = 0xAB;
/// Bytes per streamed write.
const CHUNK: usize = 64 * 1024;

/// The stream `OpenStream` left open, waiting for `UseStream`.
static OPEN: Mutex<Option<(StreamWriter, u64)>> = Mutex::new(None);

struct Probe {
    connection: Connection,
}

fn body(len: u64) -> BlobReader {
    Box::pin(tokio::io::repeat(FILL).take(len))
}

fn failed(error: impl std::fmt::Display) -> Value {
    json!({ "err": { "name": "probe", "message": error.to_string() } })
}

impl Probe {
    async fn stream_put(&self, key: &str, len: u64) -> Value {
        let storage = match connect_storage(self.connection.clone()).await {
            Ok(storage) => storage,
            Err(error) => return failed(error),
        };
        let scoped = match storage.for_scope(&tinystoragedrivers_busproxy::module_scope_label()) {
            Ok(scoped) => scoped,
            Err(error) => return failed(error),
        };
        match scoped.blobs().put_stream(key, body(len), len, None).await {
            Ok(meta) => json!({ "ok": { "len": meta.len } }),
            Err(error) => failed(error),
        }
    }

    async fn stream_get(&self, key: &str) -> Value {
        let storage = match connect_storage(self.connection.clone()).await {
            Ok(storage) => storage,
            Err(error) => return failed(error),
        };
        let scoped = match storage.for_scope(&tinystoragedrivers_busproxy::module_scope_label()) {
            Ok(scoped) => scoped,
            Err(error) => return failed(error),
        };
        let found = match scoped.blobs().get_stream(key).await {
            Ok(found) => found,
            Err(error) => return failed(error),
        };
        let Some(mut stream) = found else {
            return json!({ "ok": null });
        };
        let (mut read, mut clean) = (0u64, true);
        let mut buf = vec![0u8; CHUNK];
        loop {
            match stream.reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    read += n as u64;
                    clean &= buf[..n].iter().all(|byte| *byte == FILL);
                }
                Err(error) => return failed(error),
            }
        }
        json!({ "ok": { "len": read, "meta_len": stream.meta.len, "all_fill": clean } })
    }

    async fn watch_put(&self, collection: &str, id: &str) -> Value {
        let storage = match connect_storage(self.connection.clone()).await {
            Ok(storage) => storage,
            Err(error) => return failed(error),
        };
        let scoped = match storage.for_scope(&tinystoragedrivers_busproxy::module_scope_label()) {
            Ok(scoped) => scoped,
            Err(error) => return failed(error),
        };
        let options = WatchOptions {
            after: None,
            interval: Duration::from_millis(50),
        };
        let mut watch = match scoped
            .documents()
            .watch(collection, &Filter::All, &options)
            .await
        {
            Ok(watch) => watch,
            Err(error) => return failed(error),
        };
        if let Err(error) = scoped
            .documents()
            .put(
                collection,
                id,
                json!({ "from": "probe" }),
                Default::default(),
            )
            .await
        {
            return failed(error);
        }
        match tokio::time::timeout(Duration::from_secs(5), watch.next()).await {
            Ok(Ok(Some(WatchNotice::Invalidated { .. }))) => json!({ "ok": "invalidated" }),
            Ok(Ok(Some(_))) => json!({ "ok": "reset" }),
            Ok(Ok(None)) => failed("the watch ended"),
            Ok(Err(error)) => failed(error),
            Err(_) => failed("no watch signal within 5s"),
        }
    }

    fn host_proxy(&self) -> Result<tinybus::Proxy> {
        Ok(self
            .connection
            .proxy(STORAGE, "/ai/tinyhumans/tinystorage/Blobs", STORAGE)?
            .with_timeout(Duration::from_secs(6)))
    }

    async fn open_stream(&self, len: u64) -> Result<Value> {
        let host = tinybus::BusName::new(STORAGE)?;
        let writer = self
            .connection
            .open_stream_with_timeout(
                &host,
                StreamDescriptor::with_len(len),
                Duration::from_secs(6),
            )
            .await?;
        let reference = serde_json::to_value(writer.stream_ref())?;
        *OPEN.lock().unwrap() = Some((writer, len));
        Ok(json!({ "ok": reference }))
    }

    async fn use_stream(&self, key: &str, len: u64) -> Result<Value> {
        let Some((mut writer, opened)) = OPEN.lock().unwrap().take() else {
            return Ok(failed("no open stream"));
        };
        if opened != len {
            return Ok(failed("length differs from the open stream"));
        }
        let head = json!({ "key": key, "len": len });
        let proxy = self.host_proxy()?;
        let calling = proxy.call::<Value>(
            "PutStream",
            (head, serde_json::to_value(writer.stream_ref())?),
        );
        let sending = async {
            let chunk = vec![FILL; CHUNK];
            let mut left = len as usize;
            while left > 0 {
                let n = left.min(CHUNK);
                writer.write_chunk(&chunk[..n]).await?;
                left -= n;
            }
            writer.finish().await.map(|_| ())
        };
        let (reply, sent) = tokio::join!(calling, sending);
        Ok(match (reply, sent) {
            (Ok(value), Ok(())) => json!({ "ok": value }),
            (Err(Error::MethodFailed { name, message }), _) => {
                json!({ "err": { "name": name, "message": message } })
            }
            (Err(other), _) => failed(other),
            (Ok(_), Err(other)) => failed(other),
        })
    }

    async fn raw_get_stream(&self, key: &str) -> Result<Value> {
        let reply: Value = match self
            .host_proxy()?
            .call("GetStream", (json!({ "key": key }),))
            .await
        {
            Ok(reply) => reply,
            Err(Error::MethodFailed { name, message }) => {
                return Ok(json!({ "err": { "name": name, "message": message } }));
            }
            Err(other) => return Ok(failed(other)),
        };
        Ok(json!({ "ok": reply }))
    }
}

#[async_trait]
impl Interface for Probe {
    fn name(&self) -> InterfaceName {
        InterfaceName::new(PROBE).expect("a fixed name")
    }

    fn members(&self) -> Vec<MemberName> {
        [
            "Call",
            "PutBlob",
            "StreamPut",
            "StreamGet",
            "WatchPut",
            "OpenStream",
            "UseStream",
            "RawGetStream",
        ]
        .map(|member| MemberName::new(member).expect("a fixed name"))
        .to_vec()
    }

    async fn call(&self, member: &MemberName, args: Value) -> Result<Value> {
        match member.as_str() {
            "StreamPut" => {
                let (key, len): (String, u64) = serde_json::from_value(args)?;
                return Ok(self.stream_put(&key, len).await);
            }
            "StreamGet" => {
                let (key,): (String,) = serde_json::from_value(args)?;
                return Ok(self.stream_get(&key).await);
            }
            "WatchPut" => {
                let (collection, id): (String, String) = serde_json::from_value(args)?;
                return Ok(self.watch_put(&collection, &id).await);
            }
            "OpenStream" => {
                let (len,): (u64,) = serde_json::from_value(args)?;
                return self.open_stream(len).await;
            }
            "UseStream" => {
                let (key, len): (String, u64) = serde_json::from_value(args)?;
                return self.use_stream(&key, len).await;
            }
            "RawGetStream" => {
                let (key,): (String,) = serde_json::from_value(args)?;
                return self.raw_get_stream(&key).await;
            }
            _ => {}
        }
        let (object, selector, request, confidential): (String, String, Value, bool) =
            if member.as_str() == "PutBlob" {
                // `[key, len]`: the body is built here, because a body that
                // size cannot be sent to the probe over one bus frame either.
                let (key, len): (String, usize) = serde_json::from_value(args)?;
                (
                    "Blobs".into(),
                    "Put".into(),
                    json!({ "key": key, "bytes": vec![255u8; len] }),
                    false,
                )
            } else {
                serde_json::from_value(args)?
            };
        // Short, so a call the bus drops (an oversized frame) is an answer
        // here well inside the caller's own timeout.
        let proxy = self
            .connection
            .proxy(
                STORAGE,
                format!("/ai/tinyhumans/tinystorage/{object}"),
                STORAGE,
            )?
            .with_timeout(std::time::Duration::from_secs(6));
        let reply: Result<Value> = if confidential {
            proxy.call_confidential(&selector, (request,)).await
        } else {
            proxy.call(&selector, (request,)).await
        };
        Ok(match reply {
            Ok(value) => json!({ "ok": value }),
            Err(Error::MethodFailed { name, message }) => {
                json!({ "err": { "name": name, "message": message } })
            }
            Err(other) => json!({ "err": { "name": "transport", "message": other.to_string() } }),
        })
    }

    async fn call_with_context(
        &self,
        member: &MemberName,
        args: Value,
        _context: &CallContext,
    ) -> Result<Value> {
        self.call(member, args).await
    }
}

async fn setup(connection: Connection) -> Result<()> {
    connection
        .serve_at(
            "/probe".parse()?,
            Probe {
                connection: connection.clone(),
            },
        )
        .await
}

// The canonical SDK owns the native ABI; nothing unsafe is written here.
tinybus_module::module_export! {
    setup = setup,
    worker_threads = 1,
    provides = ["ai.tinyhumans.test.StorageProbe"],
    methods = ["Call", "PutBlob", "StreamPut", "StreamGet", "WatchPut", "OpenStream", "UseStream", "RawGetStream"],
    signals = [],
    requires = [],
    optional = [],
    lazy = false,
}
