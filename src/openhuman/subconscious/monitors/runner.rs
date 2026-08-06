use super::store::MonitorStore;
use super::types::{
    now_ms, MonitorEvent, MonitorSnapshot, MonitorStatus, MonitorStream, MAX_OUTPUT_BYTES,
};
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::harness::run_queue::{QueueMode, QueuedMessage, RunQueue};
use crate::openhuman::agent::host_runtime::RuntimeAdapter;
use crate::openhuman::security::{AuditLogger, CommandExecutionLog, SecurityPolicy};
use anyhow::Context;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "USER",
    "SHELL",
    "TMPDIR",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
];

#[derive(Clone)]
pub struct RunnerContext {
    pub security: Arc<SecurityPolicy>,
    pub runtime: Arc<dyn RuntimeAdapter>,
    pub audit: Arc<AuditLogger>,
    pub store: Arc<MonitorStore>,
    pub run_queue: Option<Arc<RunQueue>>,
}

pub async fn run_monitor(
    ctx: RunnerContext,
    snapshot: MonitorSnapshot,
    timeout: Duration,
    stop_rx: oneshot::Receiver<()>,
) {
    let monitor_id = snapshot.monitor_id.clone();
    let command = snapshot.command.clone();
    let start = Instant::now();
    tracing::info!(
        monitor_id = %monitor_id,
        timeout_ms = timeout.as_millis() as u64,
        "[monitor] runner starting"
    );
    publish_status(&snapshot, MonitorStatus::Running);
    let _ = ctx
        .store
        .set_status(&monitor_id, MonitorStatus::Running, None, None)
        .await;

    let outcome = run_child(&ctx, &snapshot, timeout, stop_rx).await;
    let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (status, success, exit_code, error) = match outcome {
        Ok(ChildOutcome::Completed(code)) => (
            MonitorStatus::Completed,
            code == Some(0),
            code,
            if code == Some(0) {
                None
            } else {
                Some(format!("command exited with status {:?}", code))
            },
        ),
        Ok(ChildOutcome::Stopped) => (MonitorStatus::Stopped, true, None, None),
        Ok(ChildOutcome::TimedOut) => (
            MonitorStatus::TimedOut,
            false,
            None,
            Some("monitor timed out".to_string()),
        ),
        Err(err) => (MonitorStatus::Failed, false, None, Some(err.to_string())),
    };
    let updated = ctx
        .store
        .set_status(&monitor_id, status.clone(), exit_code, error)
        .await;
    if let Some(snapshot) = updated {
        publish_status(&snapshot, status);
    }
    emit_audit(&ctx.audit, &command, true, success, duration_ms);
    tracing::info!(
        monitor_id = %monitor_id,
        success,
        duration_ms,
        "[monitor] runner finished"
    );
}

enum ChildOutcome {
    Completed(Option<i32>),
    Stopped,
    TimedOut,
}

async fn run_child(
    ctx: &RunnerContext,
    snapshot: &MonitorSnapshot,
    timeout: Duration,
    stop_rx: oneshot::Receiver<()>,
) -> anyhow::Result<ChildOutcome> {
    let mut cmd = ctx
        .runtime
        .build_shell_command(&snapshot.command, &ctx.security.action_dir)
        .context("building monitor command")?;
    cmd.env_clear();
    for var in SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("spawning monitor command")?;
    let stdout = child.stdout.take().context("capturing monitor stdout")?;
    let stderr = child.stderr.take().context("capturing monitor stderr")?;
    let (line_tx, mut line_rx) = mpsc::channel::<(MonitorStream, String)>(64);
    let stdout_handle = spawn_reader(stdout, MonitorStream::Stdout, line_tx.clone());
    let stderr_handle = spawn_reader(stderr, MonitorStream::Stderr, line_tx.clone());
    drop(line_tx);

    let mut output = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&snapshot.output_file)
        .await
        .with_context(|| format!("opening output file {}", snapshot.output_file.display()))?;
    let mut output_bytes = snapshot.output_bytes;
    let mut dropped_bytes = snapshot.dropped_bytes;
    let sleep = tokio::time::sleep(timeout);
    tokio::pin!(sleep);
    tokio::pin!(stop_rx);

    loop {
        tokio::select! {
            biased;
            _ = &mut stop_rx => {
                let _ = child.kill().await;
                join_readers(stdout_handle, stderr_handle, &snapshot.monitor_id).await;
                drain_lines(ctx, snapshot, &mut output, &mut output_bytes, &mut dropped_bytes, &mut line_rx).await?;
                return Ok(ChildOutcome::Stopped);
            }
            _ = &mut sleep => {
                let _ = child.kill().await;
                join_readers(stdout_handle, stderr_handle, &snapshot.monitor_id).await;
                drain_lines(ctx, snapshot, &mut output, &mut output_bytes, &mut dropped_bytes, &mut line_rx).await?;
                return Ok(ChildOutcome::TimedOut);
            }
            maybe = line_rx.recv() => {
                if let Some((stream, line)) = maybe {
                    record_line(ctx, snapshot, &mut output, &mut output_bytes, &mut dropped_bytes, stream, line).await?;
                }
            }
            status = child.wait() => {
                let status = status.context("waiting for monitor command")?;
                join_readers(stdout_handle, stderr_handle, &snapshot.monitor_id).await;
                drain_lines(ctx, snapshot, &mut output, &mut output_bytes, &mut dropped_bytes, &mut line_rx).await?;
                return Ok(ChildOutcome::Completed(status.code()));
            }
        }
    }
}

fn spawn_reader<R>(
    reader: R,
    stream: MonitorStream,
    tx: mpsc::Sender<(MonitorStream, String)>,
) -> JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if tx.send((stream.clone(), line)).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(%error, "[monitor] failed to read process stream");
                    break;
                }
            }
        }
    })
}

async fn join_readers(stdout: JoinHandle<()>, stderr: JoinHandle<()>, monitor_id: &str) {
    if let Err(error) = stdout.await {
        tracing::warn!(monitor_id, %error, "[monitor] stdout reader task failed");
    }
    if let Err(error) = stderr.await {
        tracing::warn!(monitor_id, %error, "[monitor] stderr reader task failed");
    }
}

async fn drain_lines(
    ctx: &RunnerContext,
    snapshot: &MonitorSnapshot,
    output: &mut tokio::fs::File,
    output_bytes: &mut usize,
    dropped_bytes: &mut usize,
    line_rx: &mut mpsc::Receiver<(MonitorStream, String)>,
) -> anyhow::Result<()> {
    while let Ok((stream, line)) = line_rx.try_recv() {
        record_line(
            ctx,
            snapshot,
            output,
            output_bytes,
            dropped_bytes,
            stream,
            line,
        )
        .await?;
    }
    Ok(())
}

async fn record_line(
    ctx: &RunnerContext,
    snapshot: &MonitorSnapshot,
    output: &mut tokio::fs::File,
    output_bytes: &mut usize,
    dropped_bytes: &mut usize,
    stream: MonitorStream,
    line: String,
) -> anyhow::Result<()> {
    let event = MonitorEvent {
        monitor_id: snapshot.monitor_id.clone(),
        thread_id: snapshot.thread_id.clone(),
        timestamp_ms: now_ms(),
        stream,
        line,
    };
    write_bounded_line(output, &event, output_bytes, dropped_bytes).await?;
    ctx.store
        .push_event(event.clone(), *output_bytes, *dropped_bytes)
        .await;
    publish_line(&event);
    enqueue_collect(ctx, snapshot, &event).await;
    Ok(())
}

async fn write_bounded_line(
    output: &mut tokio::fs::File,
    event: &MonitorEvent,
    output_bytes: &mut usize,
    dropped_bytes: &mut usize,
) -> anyhow::Result<()> {
    let line = format!(
        "{} [{}] {}\n",
        event.timestamp_ms,
        event.stream.as_str(),
        event.line
    );
    let bytes = line.as_bytes();
    if *output_bytes + bytes.len() <= MAX_OUTPUT_BYTES {
        output.write_all(bytes).await?;
        *output_bytes += bytes.len();
    } else {
        *dropped_bytes += bytes.len();
    }
    Ok(())
}

async fn enqueue_collect(ctx: &RunnerContext, snapshot: &MonitorSnapshot, event: &MonitorEvent) {
    let Some(run_queue) = ctx.run_queue.as_ref() else {
        return;
    };
    let Some(thread_id) = snapshot.thread_id.clone() else {
        return;
    };
    let text = format!(
        "[Monitor {} {}] {}",
        snapshot.monitor_id,
        event.stream.as_str(),
        event.line
    );
    run_queue
        .push(QueuedMessage {
            text,
            mode: QueueMode::Collect,
            client_id: "monitor".to_string(),
            thread_id: thread_id.clone(),
            queued_at_ms: event.timestamp_ms,
            model_override: None,
            temperature: None,
            profile_id: None,
            locale: None,
        })
        .await;
    let status = run_queue.status().await;
    publish_global(DomainEvent::RunQueueMessageQueued {
        thread_id,
        mode: QueueMode::Collect.to_string(),
        queue_depth: status.total,
    });
}

fn publish_status(snapshot: &MonitorSnapshot, status: MonitorStatus) {
    publish_global(DomainEvent::MonitorStatusChanged {
        monitor_id: snapshot.monitor_id.clone(),
        status: format!("{:?}", status).to_lowercase(),
        thread_id: snapshot.thread_id.clone(),
        description: snapshot.description.clone(),
    });
}

fn publish_line(event: &MonitorEvent) {
    publish_global(DomainEvent::MonitorLine {
        monitor_id: event.monitor_id.clone(),
        thread_id: event.thread_id.clone(),
        timestamp_ms: event.timestamp_ms,
        stream: event.stream.as_str().to_string(),
        line: event.line.clone(),
    });
}

fn emit_audit(audit: &AuditLogger, command: &str, allowed: bool, success: bool, duration_ms: u64) {
    if let Err(error) = audit.log_command_event(CommandExecutionLog {
        channel: "tool:monitor",
        command,
        risk_level: "unknown",
        approved: true,
        allowed,
        success,
        duration_ms,
    }) {
        tracing::warn!(%error, "[monitor] failed to persist audit event");
    }
}

pub fn output_path(workspace_dir: &Path, monitor_id: &str) -> std::path::PathBuf {
    workspace_dir
        .join("monitor")
        .join(format!("{monitor_id}.log"))
}
