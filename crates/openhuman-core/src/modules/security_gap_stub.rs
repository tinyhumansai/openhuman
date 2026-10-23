//! A scripted [`GapPort`] for tests of code that has no module runtime.
//!
//! It decides nothing the module owns: every answer is the one the test scripted,
//! and the questions asked are recorded so a test can pin what the caller sent. What
//! the module itself answers is pinned by the module-backed tests.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinysecurity_bus::{
    BudgetDecision2, CommandClass, CommandPolicy, Decision, HostActionClassRequest2, PathCheckKind,
    PathPolicy, PrivacyCheckRequest2, ProtectedFloor2, Verdict,
};

use super::{BudgetOp, GapPort};

type PathAnswer = Box<dyn Fn(&PathPolicy, PathCheckKind, &PathBuf) -> bool + Send + Sync>;
type FloorAnswer = Box<dyn Fn(&PathBuf) -> bool + Send + Sync>;
type BudgetAnswer = Box<dyn Fn(BudgetOp, &CommandPolicy) -> BudgetDecision2 + Send + Sync>;
type PrivacyAnswer = Box<dyn Fn(&PrivacyCheckRequest2) -> Verdict + Send + Sync>;

/// One recorded question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Asked {
    Paths(PathCheckKind, Vec<PathBuf>),
    Protected(Vec<PathBuf>),
    Budget(BudgetOp),
    Privacy(PrivacyCheckRequest2),
}

pub(crate) struct StubPort {
    paths: PathAnswer,
    floor: FloorAnswer,
    budget: BudgetAnswer,
    privacy: PrivacyAnswer,
    asked: Mutex<Vec<Asked>>,
    fail: bool,
}

impl std::fmt::Debug for StubPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubPort").finish_non_exhaustive()
    }
}

impl StubPort {
    /// Allows every path, protects none, has budget left and permits every transfer.
    pub(crate) fn allowing() -> Self {
        Self {
            paths: Box::new(|_, _, _| true),
            floor: Box::new(|_| false),
            budget: Box::new(|_, _| BudgetDecision2::Allow),
            privacy: Box::new(|_| Verdict::Allow),
            asked: Mutex::new(Vec::new()),
            fail: false,
        }
    }

    /// As [`Self::allowing`], with a scripted stand-in for the floor: credential
    /// stores and the system roots are protected. For tests of a caller that
    /// refuses protected locations; the module owns the real match and its own
    /// tests pin it.
    pub(crate) fn with_credential_floor() -> Self {
        Self::allowing().protecting(|path| {
            let text = path.to_string_lossy().to_ascii_lowercase();
            let credential = text
                .split(['/', '\\'])
                .any(|part| matches!(part, ".ssh" | ".gnupg" | ".aws" | ".kube" | ".netrc"));
            let system = ["/etc", "/root", "/boot", "/proc", "/sys"]
                .iter()
                .any(|root| text == *root || text.starts_with(&format!("{root}/")));
            credential || system
        })
    }

    /// A port whose every question is an `Err`, as a missing module answers.
    pub(crate) fn unavailable() -> Self {
        Self {
            fail: true,
            ..Self::allowing()
        }
    }

    pub(crate) fn paths(
        mut self,
        answer: impl Fn(&PathPolicy, PathCheckKind, &PathBuf) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.paths = Box::new(answer);
        self
    }

    pub(crate) fn protecting(
        mut self,
        answer: impl Fn(&PathBuf) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.floor = Box::new(answer);
        self
    }

    pub(crate) fn budget(
        mut self,
        answer: impl Fn(BudgetOp, &CommandPolicy) -> BudgetDecision2 + Send + Sync + 'static,
    ) -> Self {
        self.budget = Box::new(answer);
        self
    }

    pub(crate) fn privacy(
        mut self,
        answer: impl Fn(&PrivacyCheckRequest2) -> Verdict + Send + Sync + 'static,
    ) -> Self {
        self.privacy = Box::new(answer);
        self
    }

    pub(crate) fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }

    pub(crate) fn shared(self) -> Arc<Self> {
        Arc::new(self)
    }

    fn record(&self, asked: Asked) -> Result<(), String> {
        self.asked.lock().unwrap().push(asked);
        if self.fail {
            return Err("TinySecurity unavailable (scripted)".into());
        }
        Ok(())
    }
}

#[async_trait]
impl GapPort for StubPort {
    async fn check_paths(
        &self,
        scope: PathPolicy,
        kind: PathCheckKind,
        paths: Vec<PathBuf>,
    ) -> Result<Vec<bool>, String> {
        self.record(Asked::Paths(kind, paths.clone()))?;
        Ok(paths
            .iter()
            .map(|path| (self.paths)(&scope, kind, path))
            .collect())
    }

    async fn protected_paths(&self, paths: Vec<PathBuf>) -> Result<Vec<bool>, String> {
        self.record(Asked::Protected(paths.clone()))?;
        Ok(paths.iter().map(|path| (self.floor)(path)).collect())
    }

    async fn protected_floor(&self) -> Result<ProtectedFloor2, String> {
        if self.fail {
            return Err("TinySecurity unavailable (scripted)".into());
        }
        Ok(ProtectedFloor2 {
            components: vec![".ssh".into()],
            files: vec![".netrc".into()],
            pairs: vec![[".docker".into(), "config.json".into()]],
            prefixes: vec!["/etc".into()],
        })
    }

    async fn budget(
        &self,
        _workspace_dir: PathBuf,
        policy: CommandPolicy,
        op: BudgetOp,
    ) -> Result<BudgetDecision2, String> {
        self.record(Asked::Budget(op))?;
        Ok((self.budget)(op, &policy))
    }

    async fn privacy(&self, request: PrivacyCheckRequest2) -> Result<Decision, String> {
        self.record(Asked::Privacy(request.clone()))?;
        Ok(Decision {
            generation: 1,
            verdict: (self.privacy)(&request),
            cacheable: false,
        })
    }

    async fn host_action_class(
        &self,
        request: HostActionClassRequest2,
    ) -> Result<CommandClass, String> {
        if self.fail {
            return Err("TinySecurity unavailable (scripted)".into());
        }
        Ok(request.class())
    }
}
