//! What the agent is allowed to do.
//!
//! # The trap this type exists to close
//!
//! Access is governed by **two independent mechanisms**, and setting only the
//! obvious one produces an agent that looks configured and silently refuses to
//! act:
//!
//! 1. **The autonomy tier** (`config.autonomy.level`) drives `SecurityPolicy` —
//!    which command classes are allowed, prompted, or blocked.
//! 2. **The turn origin** (a task-local, [`AgentTurnOrigin`]) is the caller's
//!    statement of authority, and the approval gate is *fail-closed* on it. An
//!    unlabelled call site is hard-denied for external-effect tools regardless
//!    of tier.
//!
//! An embedder that sets `level = "full"` and nothing else gets a turn whose
//! `shell`, `edit`, `apply_patch` and `*_exec` calls all refuse — with a
//! plausible-looking transcript, because the model narrates around the refusals.
//! It reads as a weak model rather than a missing scope, which is what makes it
//! expensive to diagnose. [`Access`] therefore always yields **both** halves
//! together, and they cannot be set independently.

use crate::openhuman::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};
use crate::openhuman::security::{AutonomyLevel, TrustedAccess, TrustedRoot};

/// The authority a harness's turns run with.
#[derive(Debug, Clone)]
pub struct Access {
    level: AutonomyLevel,
    /// `None` means "let the core apply its own default for a direct chat
    /// dispatch", which is the trusted-operator `Cli` allowance.
    origin: Option<AgentTurnOrigin>,
    trusted_roots: Vec<TrustedRoot>,
    allow_tool_install: bool,
    approval_gate: bool,
}

impl Default for Access {
    /// [`Access::supervised`] — the same default the core itself uses.
    fn default() -> Self {
        Self::supervised()
    }
}

impl Access {
    /// Observe but never act: no writes, no shell, no network side effects.
    ///
    /// The safe default for running an untrusted prompt.
    pub fn readonly() -> Self {
        Self {
            level: AutonomyLevel::ReadOnly,
            // A read-only agent has nothing to approve, and labelling the turn
            // as automation would grant an allowance the tier already refuses.
            origin: None,
            trusted_roots: Vec::new(),
            allow_tool_install: false,
            approval_gate: true,
        }
    }

    /// Act, but park risky operations for a human decision.
    ///
    /// The approval gate stays on, which means an *unattended* harness will
    /// stall here: parked turns hold for a 10-minute TTL and then deny. Use
    /// [`Access::full`] for automation, or answer the approvals.
    pub fn supervised() -> Self {
        Self {
            level: AutonomyLevel::Supervised,
            origin: None,
            trusted_roots: Vec::new(),
            allow_tool_install: false,
            approval_gate: true,
        }
    }

    /// Act autonomously within policy bounds, without pausing for approval.
    ///
    /// Sets the tier *and* labels turns as trusted automation, because either
    /// alone is not enough — see the module docs. Hard blocks are unaffected:
    /// credential stores and system directories stay forbidden, and the agent
    /// still cannot write into the workspace's internal state.
    ///
    /// This grants an agent real ability to run commands and edit files under
    /// its `action_dir`. Give it a directory you are willing to have changed.
    pub fn full() -> Self {
        Self {
            level: AutonomyLevel::Full,
            origin: Some(AgentTurnOrigin::TrustedAutomation {
                job_id: "embedded-harness".to_string(),
                source: TrustedAutomationSource::Workflow {
                    require_approval: false,
                },
            }),
            trusted_roots: Vec::new(),
            allow_tool_install: false,
            approval_gate: false,
        }
    }

    /// Grant access to a directory outside the workspace.
    ///
    /// Takes precedence over `workspace_only`, except for credential stores
    /// (`~/.ssh`, `~/.gnupg`, `~/.aws`), which stay blocked whatever is granted.
    pub fn trust(mut self, path: impl Into<String>, access: TrustedAccess) -> Self {
        self.trusted_roots.push(TrustedRoot {
            path: path.into(),
            access,
        });
        self
    }

    /// Permit the agent to install OS packages via the `install_tool` tool.
    ///
    /// Off in every preset, including [`full`](Self::full): installing software
    /// on the host reaches outside the action directory that otherwise bounds
    /// the blast radius, so it is opted into by name rather than implied by a
    /// tier.
    pub fn allow_tool_install(mut self, allow: bool) -> Self {
        self.allow_tool_install = allow;
        self
    }

    /// Override the turn origin.
    ///
    /// The presets pick one for you. Reach for this when the harness is driving
    /// on behalf of something with a narrower authority than "trusted
    /// automation" — an inbound external message, say — so the approval gate
    /// applies the grant that actually matches.
    pub fn origin(mut self, origin: AgentTurnOrigin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// The origin turns run under, if this access level states one.
    pub fn turn_origin(&self) -> Option<&AgentTurnOrigin> {
        self.origin.as_ref()
    }

    /// Whether the interactive approval gate should park turns.
    pub fn approval_gate_enabled(&self) -> bool {
        self.approval_gate
    }

    /// Write this access level into `config`.
    pub(super) fn apply(&self, config: &mut crate::openhuman::config::Config) {
        config.autonomy.level = self.level;
        config.autonomy.allow_tool_install = self.allow_tool_install;
        config
            .autonomy
            .trusted_roots
            .extend(self.trusted_roots.iter().cloned());
        // `auto_approve_all` is deliberately NOT set for `full()`. The origin
        // is the correct instrument — it says *who is calling*, which the gate
        // can reason about — whereas `auto_approve_all` is a blanket bypass
        // that would also cover call sites this harness never intended to
        // vouch for.
    }
}

#[cfg(test)]
#[path = "access_tests.rs"]
mod tests;
