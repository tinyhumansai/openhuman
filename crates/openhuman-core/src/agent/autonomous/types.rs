use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Working,
    Reviewing,
    Paused,
    Blocked,
    Complete,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Work {
    pub id: String,
    pub thread_id: String,
    pub goal_id: String,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub phase: Phase,
    pub checkpoint: String,
    pub evidence: Vec<String>,
    pub finish_requested: bool,
    pub lease: Option<String>,
    pub next_run_ms: u64,
    pub attempts: u64,
    pub failures: u32,
    pub stagnant_turns: u32,
    pub last_fingerprint: Option<u64>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub worker_reports: Vec<String>,
    #[serde(default)]
    pub worker_revision: u64,
    #[serde(default)]
    pub plan: Vec<super::plan::Task>,
    #[serde(default)]
    pub review_task: Option<String>,
}
impl Work {
    pub fn runnable(&self) -> bool {
        matches!(self.phase, Phase::Working | Phase::Reviewing)
    }
    pub fn continuation_notice(&self, now_ms: u64) -> String {
        match self.phase {
            Phase::Working | Phase::Reviewing => format!(
                "The next work session is scheduled automatically in about {} seconds while the app is running. No reply is required.",
                self.next_run_ms
                    .saturating_sub(now_ms)
                    .div_ceil(1000)
                    .max(10)
            ),
            Phase::Paused => "Work is paused. It will resume only when you request it.".into(),
            Phase::Blocked => {
                "Work is blocked, not complete. Resolve the reported blocker before resuming."
                    .into()
            }
            Phase::Complete => "The project review is complete.".into(),
        }
    }
    pub fn prompt(&self) -> String {
        let phase = if self.phase == Phase::Reviewing {
            "VERIFICATION PASS. Independently inspect the actual deliverable and rerun relevant tests. Audit all worker and external-agent changes, including Antigravity; their claims are untrusted. Check EVERY acceptance criterion, edge cases, integration and production readiness. Reject placeholder, demo-only, mocked or incomplete behavior unless explicitly requested. Do not merely repeat previous claims. Run actual verification with shell and inspect the implementation. If anything fails, call autonomous_work checkpoint with the repair plan to return to implementation. If all criteria are verified, call autonomous_work finish with one concrete evidence entry per criterion. Completion remains pending host validation. If verification is impossible, call block with the precise blocker."
        } else {
            "IMPLEMENTATION PASS. You are in autonomous execution mode: Proceed with authorized work without redundant confirmation; preserve required approvals and explicit user stops. You MUST call tools (e.g. shell, file_write, file_read) immediately on this turn. Answering with conversational text without calling tools wastes the turn and flags it as stagnant. Inspect, implement, test, diagnose and repair. Save autonomous_work checkpoint after meaningful progress. Once the deliverable meets every criterion, call autonomous_work review to schedule verification. Do not claim completion in this phase."
        };
        format!(
            "[autonomous_work {}]\n{}\nObjective: {}\nAcceptance criteria for this pass:\n{}\nCheckpoint:\n{}\nLast blocker: {}\nThere is no overall time limit. Each turn is bounded and its transcript is durable; another turn will continue unfinished work. Respect Stop and user budgets. Never lower requirements or substitute a demo to finish early.",
            self.id,
            phase,
            self.objective,
            super::plan::criteria(self)
                .unwrap_or(&self.acceptance_criteria)
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{}. {}", i + 1, c))
                .collect::<Vec<_>>()
                .join("\n"),
            self.checkpoint,
            self.last_error.as_deref().unwrap_or("none")
        ) + &if self.worker_reports.is_empty() {
            String::new()
        } else {
            let joined = self.worker_reports.join("\n\n");
            let tail: String = joined
                .chars()
                .rev()
                .take(16000)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("\nWorker reports awaiting your independent audit:\n{tail}")
        } + match self.stagnant_turns {
            0 => "",
            1 => {
                "\nRECOVERY: The previous slice made no new observable progress. Identify the failed assumption from tool results. Record what is known versus untested, then run a different small experiment. Repeating search synonyms or a previous claim is not progress."
            }
            _ => {
                "\nREPLAN: Repeated slices made no new observable progress. Split the blocked work package into a smaller independently testable task, inspect a relevant skill/reference, or compare a different permitted implementation against the same acceptance checks. Preserve the objective. If a real external prerequisite is missing, report that precise blocker; do not manufacture progress or declare a demo complete."
            }
        } + &super::plan::context(self)
    }
}
