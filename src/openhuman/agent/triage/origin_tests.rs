//! The two labels are a security decision (openhuman#5634), so they are pinned
//! by identity rather than by "is not `Unknown`" — a future edit that swapped
//! the remote label for `Cli` would still satisfy the weaker assertion while
//! handing a trust root to remote content.

use super::*;
use crate::openhuman::agent::triage::envelope::TriggerEnvelope;

fn composio_envelope() -> TriggerEnvelope {
    TriggerEnvelope::from_composio(
        "gmail",
        "new_message",
        "ti_meta_id",
        "ti_bCCTKZlajKi4",
        serde_json::json!({ "subject": "hello" }),
    )
}

#[test]
fn a_local_dispatch_is_the_trust_root() {
    assert!(
        matches!(local_trigger_origin(), AgentTurnOrigin::Cli),
        "local triage keeps the authority the caller already had"
    );
}

#[test]
fn a_remote_dispatch_parks_instead_of_being_trusted() {
    let origin = remote_trigger_origin(&composio_envelope());
    match origin {
        AgentTurnOrigin::TrustedAutomation { source, .. } => assert_eq!(
            source,
            TrustedAutomationSource::Workflow {
                require_approval: true
            },
            "remote payloads must force a park, not auto-allow"
        ),
        other => panic!("remote triage must not run as {}", other.class()),
    }
}

#[test]
fn a_remote_dispatch_is_never_the_trust_root() {
    // The rejected option, pinned so reintroducing it fails here first.
    assert!(
        !matches!(
            remote_trigger_origin(&composio_envelope()),
            AgentTurnOrigin::Cli
        ),
        "`Cli` grants a full trust root with no audit row — never for remote content"
    );
}

#[test]
fn the_job_id_identifies_the_trigger_without_quoting_it() {
    let envelope = composio_envelope();
    let AgentTurnOrigin::TrustedAutomation { job_id, .. } = remote_trigger_origin(&envelope) else {
        panic!("remote triage must be trusted automation");
    };
    assert_eq!(job_id, "composio:ti_bCCTKZlajKi4");
    // The gate renders `job_id` as `flow_id` at `info`, so payload text must
    // not reach it.
    assert!(
        !job_id.contains("hello"),
        "job_id must not carry payload content, got {job_id:?}"
    );
}

/// A tripwire, not a style check.
///
/// The labels above are only worth anything if every dispatch site actually
/// scopes one, and the failure mode is silent: a seventh caller that forgets
/// reads `Unknown`, the gate fails closed, and the symptom is a denied
/// escalation in a staging log weeks later — which is precisely how
/// openhuman#5634 was found. `AGENT_TURN_ORIGIN` is a task-local, so the
/// compiler cannot enforce this.
///
/// Adding a call site is therefore meant to fail here. Fix it by scoping the
/// label its provenance calls for and adding the path below — not by deleting
/// the entry.
#[test]
fn every_dispatch_site_scopes_an_origin() {
    const KNOWN_SITES: &[(&str, &str)] = &[
        // Remote payloads — park and audit.
        (
            "src/openhuman/memory/sync/composio/bus_part_01.rs",
            "remote",
        ),
        ("src/openhuman/integrations/task_sources/route.rs", "remote"),
        ("src/openhuman/skills/webhooks/ops.rs", "remote"),
        ("src/openhuman/skills/webhooks/bus.rs", "remote"),
        // Locally initiated — trust root.
        ("src/openhuman/desktop/notifications/rpc.rs", "local"),
        ("src/openhuman/agent/schemas.rs", "local"),
    ];

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found: Vec<String> = Vec::new();
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            // The triage module defines and tests `apply_decision`; it is the
            // callee, not a call site. Test files model callers rather than
            // being them.
            if rel.starts_with("src/openhuman/agent/triage/") || rel.ends_with("_tests.rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if text.contains("apply_decision(") {
                assert!(
                    text.contains("with_origin("),
                    "{rel} dispatches triage without scoping an origin, so it reaches the \
                     approval gate as `Unknown` and is denied (openhuman#5634). Scope \
                     `remote_trigger_origin` or `local_trigger_origin` by provenance."
                );

                // Every dispatch in the file must be inside a scope. Text
                // cannot prove *which* scope wraps *which* call, but an
                // unscoped call added beside a scoped one changes this ratio,
                // which is the shape the previous version of this test missed.
                let dispatches = text.matches("apply_decision(").count();
                let scopes = text.matches("with_origin(").count();
                assert_eq!(
                    dispatches, scopes,
                    "{rel} has {dispatches} triage dispatch(es) but {scopes} origin scope(s); \
                     every `apply_decision` call needs its own `with_origin` (openhuman#5634)"
                );

                // The provenance assertion — the reason two labels exist. A
                // remote payload scoped with the local trust root is the exact
                // defect this module prevents, and it is indistinguishable from
                // a correct file if the test only asks whether *some* origin
                // was scoped.
                if let Some((_, kind)) = KNOWN_SITES.iter().find(|(path, _)| *path == rel) {
                    let (want, reject) = match *kind {
                        "remote" => ("remote_trigger_origin", "local_trigger_origin"),
                        "local" => ("local_trigger_origin", "remote_trigger_origin"),
                        other => panic!("KNOWN_SITES has unknown provenance {other:?} for {rel}"),
                    };
                    assert!(
                        text.contains(want),
                        "{rel} is recorded as `{kind}` provenance but does not scope `{want}`"
                    );
                    assert!(
                        !text.contains(reject),
                        "{rel} is recorded as `{kind}` provenance but scopes `{reject}`. \
                         A remote, attacker-influenceable payload must never take the local \
                         trust root: that grants authority the caller never had, which is the \
                         defect openhuman#5634 exists to prevent. If the provenance genuinely \
                         changed, change the KNOWN_SITES entry deliberately — do not widen \
                         this assertion."
                    );
                }

                found.push(rel);
            }
        }
    }

    found.sort();
    let mut expected: Vec<String> = KNOWN_SITES.iter().map(|(p, _)| (*p).to_string()).collect();
    expected.sort();
    assert_eq!(
        found, expected,
        "the set of triage dispatch sites changed; label the new one by provenance \
         and record it in KNOWN_SITES"
    );
}
