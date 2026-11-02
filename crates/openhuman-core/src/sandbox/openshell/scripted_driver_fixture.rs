//! Test-only [`BoxDriver`] stand-in: records every call and answers execs from
//! a script, so the registry's lifecycle is testable without a TinyBox module.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Notify;

use super::{BoxDriver, BoxError, BoxExec, BoxOutput, BoxSettings};

/// One call the registry made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    Create(BoxSettings),
    Exec { resource: String, exec: BoxExec },
    Close(String),
}

/// How the next exec answers.
pub(crate) enum ExecStep {
    /// Reply with this output.
    Reply(BoxOutput),
    /// Fail with this error.
    Fail(BoxError),
    /// Never answer.
    Hang,
    /// Wait for the notification, then reply `ok`.
    WaitThenReply(Arc<Notify>),
    /// Wait for the notification, then fail.
    WaitThenFail(Arc<Notify>, BoxError),
}

/// A driver that mints `box-<n>` ids and follows its exec script; an exec past
/// the end of the script replies `ok` with exit code 0.
#[derive(Default)]
pub(crate) struct ScriptedDriver {
    calls: Mutex<Vec<Call>>,
    next_id: AtomicUsize,
    script: Mutex<VecDeque<ExecStep>>,
    create_delay: Mutex<Duration>,
    create_failures: AtomicUsize,
    close_fails: AtomicBool,
}

impl ScriptedDriver {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn then(&self, step: ExecStep) {
        self.script.lock().unwrap().push_back(step);
    }

    pub(crate) fn slow_create(&self, delay: Duration) {
        *self.create_delay.lock().unwrap() = delay;
    }

    pub(crate) fn fail_next_creates(&self, count: usize) {
        self.create_failures.store(count, Ordering::SeqCst);
    }

    pub(crate) fn fail_closes(&self) {
        self.close_fails.store(true, Ordering::SeqCst);
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub(crate) fn created(&self) -> usize {
        self.calls()
            .iter()
            .filter(|call| matches!(call, Call::Create(_)))
            .count()
    }

    pub(crate) fn closed(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Close(resource) => Some(resource),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn exec_resources(&self) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Exec { resource, .. } => Some(resource),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn last_exec(&self) -> Option<BoxExec> {
        self.calls().into_iter().rev().find_map(|call| match call {
            Call::Exec { exec, .. } => Some(exec),
            _ => None,
        })
    }

    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }
}

pub(crate) fn ok_output() -> BoxOutput {
    BoxOutput {
        exit_code: 0,
        stdout: b"ok".to_vec(),
        stderr: Vec::new(),
    }
}

#[async_trait]
impl BoxDriver for ScriptedDriver {
    async fn create(&self, settings: &BoxSettings) -> Result<String, BoxError> {
        self.record(Call::Create(settings.clone()));
        let delay = *self.create_delay.lock().unwrap();
        tokio::time::sleep(delay).await;
        let failures = self.create_failures.load(Ordering::SeqCst);
        if failures > 0 {
            self.create_failures.store(failures - 1, Ordering::SeqCst);
            return Err(BoxError::Driver("create refused".into()));
        }
        Ok(format!(
            "box-{}",
            self.next_id.fetch_add(1, Ordering::SeqCst)
        ))
    }

    async fn exec(&self, resource: &str, exec: &BoxExec) -> Result<BoxOutput, BoxError> {
        self.record(Call::Exec {
            resource: resource.to_owned(),
            exec: exec.clone(),
        });
        let step = self.script.lock().unwrap().pop_front();
        match step {
            None => Ok(ok_output()),
            Some(ExecStep::Reply(output)) => Ok(output),
            Some(ExecStep::Fail(error)) => Err(error),
            Some(ExecStep::Hang) => std::future::pending().await,
            Some(ExecStep::WaitThenReply(gate)) => {
                gate.notified().await;
                Ok(ok_output())
            }
            Some(ExecStep::WaitThenFail(gate, error)) => {
                gate.notified().await;
                Err(error)
            }
        }
    }

    async fn close(&self, resource: &str) -> Result<(), BoxError> {
        self.record(Call::Close(resource.to_owned()));
        if self.close_fails.load(Ordering::SeqCst) {
            return Err(BoxError::Driver("close refused".into()));
        }
        Ok(())
    }
}
