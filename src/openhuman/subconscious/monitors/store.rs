use super::types::{now_ms, MonitorEvent, MonitorSnapshot, MonitorStatus, RECENT_EVENT_LIMIT};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::{oneshot, Mutex};

#[derive(Debug)]
pub struct ActiveMonitor {
    pub snapshot: MonitorSnapshot,
    pub stop_tx: Option<oneshot::Sender<()>>,
}

#[derive(Debug, Default)]
pub struct MonitorStore {
    monitors: Mutex<HashMap<String, ActiveMonitor>>,
}

static GLOBAL: OnceLock<Arc<MonitorStore>> = OnceLock::new();

pub fn global_store() -> Arc<MonitorStore> {
    GLOBAL
        .get_or_init(|| Arc::new(MonitorStore::default()))
        .clone()
}

impl MonitorStore {
    pub async fn insert(&self, snapshot: MonitorSnapshot, stop_tx: oneshot::Sender<()>) {
        tracing::info!(
            monitor_id = %snapshot.monitor_id,
            "[monitor] inserting active monitor"
        );
        self.monitors.lock().await.insert(
            snapshot.monitor_id.clone(),
            ActiveMonitor {
                snapshot,
                stop_tx: Some(stop_tx),
            },
        );
    }

    pub async fn list(&self) -> Vec<MonitorSnapshot> {
        let mut values: Vec<_> = self
            .monitors
            .lock()
            .await
            .values()
            .map(|m| m.snapshot.clone())
            .collect();
        values.sort_by_key(|m| m.started_at_ms);
        values
    }

    pub async fn get(&self, monitor_id: &str) -> Option<MonitorSnapshot> {
        self.monitors
            .lock()
            .await
            .get(monitor_id)
            .map(|m| m.snapshot.clone())
    }

    pub async fn output_file(&self, monitor_id: &str) -> Option<PathBuf> {
        self.get(monitor_id).await.map(|m| m.output_file)
    }

    pub async fn push_event(&self, event: MonitorEvent, output_bytes: usize, dropped_bytes: usize) {
        let mut guard = self.monitors.lock().await;
        if let Some(active) = guard.get_mut(&event.monitor_id) {
            active.snapshot.updated_at_ms = event.timestamp_ms;
            active.snapshot.output_bytes = output_bytes;
            active.snapshot.dropped_bytes = dropped_bytes;
            active.snapshot.recent_events.push(event);
            if active.snapshot.recent_events.len() > RECENT_EVENT_LIMIT {
                let mut recent = VecDeque::from(std::mem::take(&mut active.snapshot.recent_events));
                while recent.len() > RECENT_EVENT_LIMIT {
                    recent.pop_front();
                }
                active.snapshot.recent_events = recent.into();
            }
        }
    }

    pub async fn set_status(
        &self,
        monitor_id: &str,
        status: MonitorStatus,
        exit_code: Option<i32>,
        error: Option<String>,
    ) -> Option<MonitorSnapshot> {
        let mut guard = self.monitors.lock().await;
        let active = guard.get_mut(monitor_id)?;
        active.snapshot.status = status;
        active.snapshot.exit_code = exit_code;
        active.snapshot.error = error;
        active.snapshot.updated_at_ms = now_ms();
        Some(active.snapshot.clone())
    }

    pub async fn stop(&self, monitor_id: &str) -> Result<MonitorSnapshot, String> {
        let mut guard = self.monitors.lock().await;
        let active = guard
            .get_mut(monitor_id)
            .ok_or_else(|| format!("monitor `{monitor_id}` not found"))?;
        match active.snapshot.status {
            MonitorStatus::Starting | MonitorStatus::Running => {
                if let Some(tx) = active.stop_tx.take() {
                    let _ = tx.send(());
                }
                active.snapshot.status = MonitorStatus::Stopped;
                active.snapshot.updated_at_ms = now_ms();
            }
            _ => {}
        }
        Ok(active.snapshot.clone())
    }

    #[cfg(test)]
    pub async fn clear(&self) {
        self.monitors.lock().await.clear();
    }
}
