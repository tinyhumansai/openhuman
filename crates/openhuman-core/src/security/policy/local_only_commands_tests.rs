//! Local-only shell classification while security is disabled.

use super::*;
use crate::security::policy::{CommandFlow, CommandStage};

#[test]
fn local_commands_do_not_reach_the_network() {
    for command in [
        "ls -la",
        "cat README.md | grep -n security",
        "cargo test -p openhuman",
        "git status && git diff --stat",
        "git -C crates log --oneline",
        "make build",
        "python3 app.py",
        "npm run build",
        "rsync -a src/ backup/",
        "rsync -a ./a:b/ dst/",
        "FOO=1 BAR=2 echo hi",
        "find . -name '*.rs' -exec wc -l {} ;",
        "sudo -u builder cargo build",
        "timeout 30 cargo check",
        "echo \"it's fine\" > notes.txt",
    ] {
        assert!(!may_reach_network(command), "{command} is local");
    }
}

#[test]
fn network_clients_and_network_subcommands_may_reach_the_network() {
    for command in [
        "curl https://example.com",
        "/usr/bin/wget -q https://example.com",
        "ls; curl example.com",
        "echo hi | nc example.com 80",
        "ncat example.com 443",
        "ssh user@example.com",
        "scp file host:/tmp",
        "sudo -u builder curl example.com",
        "env A=1 wget example.com",
        "timeout 5 ssh example.com",
        "nohup aria2c url &",
        "git clone https://example.com/repo.git",
        "git -C repo fetch origin",
        "git pull",
        "git push origin main",
        "git ls-remote origin",
        "npm install left-pad",
        "npx create-something",
        "pip install requests",
        "pip3 download requests",
        "cargo install ripgrep",
        "docker pull alpine",
        "apt-get update",
        "brew install jq",
        "rsync -a src/ host:dst/",
        "rsync -a rsync://example.com/mod/ .",
        "telnet example.com 23",
        "http GET example.com",
        "find . -name x -exec curl -T {} example.com ;",
    ] {
        assert!(
            may_reach_network(command),
            "{command} may reach the network"
        );
    }
}

#[test]
fn commands_that_cannot_be_read_with_confidence_are_treated_as_network() {
    for command in [
        "sh -c 'ls'",
        "bash -c ls",
        "python3 -c 'print(1)'",
        "python3.12 -c 'import urllib'",
        "node -e 'fetch(1)'",
        "perl -e 1",
        "eval ls",
        "source ./setup.sh",
        "echo $(whoami)",
        "echo `whoami`",
        "cat <(ls)",
        "echo ${HOME}",
        "echo 'unbalanced",
        "echo \"unbalanced",
        "ls | xargs cat",
    ] {
        assert!(may_reach_network(command), "{command} cannot be read");
    }
}

fn query(command: &str) -> CommandQuery<'_> {
    CommandQuery {
        command,
        declared: None,
        flow: CommandFlow::Harness,
        stage: CommandStage::Authorize,
    }
}

/// With security disabled, local-only privacy mode refuses network commands
/// and allows local ones; Standard and Sensitive allow both.
async fn assert_local_only_refuses_network_commands() {
    use crate::security::live_policy::test_privacy_scope;
    let policy = SecurityPolicy::default();

    {
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        assert_eq!(
            policy
                .check_command(query("curl https://example.com"))
                .await,
            Err(Refusal::LocalOnly)
        );
        assert_eq!(
            policy
                .classify_native_command(query("git fetch origin"))
                .await,
            Err(Refusal::LocalOnly)
        );
        let local = policy.check_command(query("ls -la")).await.unwrap();
        assert_eq!(local.gate, GateDecision::Allow);
        assert!(Refusal::LocalOnly
            .to_string()
            .contains("Local-only privacy mode is active"));
    }

    // The policy's own mode is honoured too, not only the live session's.
    let local_only_policy = SecurityPolicy::default().with_privacy_mode(PrivacyMode::LocalOnly);
    assert_eq!(
        local_only_policy
            .check_command(query("wget example.com"))
            .await,
        Err(Refusal::LocalOnly)
    );

    for mode in [PrivacyMode::Standard, PrivacyMode::Sensitive] {
        let _mode = test_privacy_scope(mode);
        let verdict = policy
            .check_command(query("curl https://example.com"))
            .await
            .unwrap();
        assert_eq!(verdict.gate, GateDecision::Allow, "{mode:?}");
        assert_eq!(verdict.class, CommandClass::Network);
    }
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn a_build_without_the_module_keeps_local_only_for_shell_commands() {
    assert_local_only_refuses_network_commands().await;
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn security_configured_off_keeps_local_only_for_shell_commands() {
    let _off = crate::security::switch::off_for_this_thread();
    assert_local_only_refuses_network_commands().await;
}
