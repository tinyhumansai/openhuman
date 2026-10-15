use super::*;
use tinysecurity_bus::{CallerContext, Verdict};

fn decision(generation: u64, cacheable: bool) -> Decision {
    Decision {
        generation,
        verdict: Verdict::Allow,
        cacheable,
    }
}

fn key(policy_generation: u64, fingerprint: &str, scope: &str) -> CacheKey {
    CacheKey {
        policy_generation,
        fingerprint: fingerprint.into(),
        scope: scope.into(),
    }
}

#[test]
fn only_decisions_the_module_marks_cacheable_are_stored() {
    let mut cache = DecisionCache::default();
    assert!(!cache.put(key(1, "a", "s"), &decision(3, false)));
    assert!(cache.get(&key(1, "a", "s")).is_none());
    assert!(cache.put(key(1, "a", "s"), &decision(3, true)));
    assert_eq!(cache.get(&key(1, "a", "s")), Some(decision(3, true)));
}

#[test]
fn a_cached_decision_is_scoped_to_policy_generation_and_subject() {
    let mut cache = DecisionCache::default();
    cache.put(key(1, "a", "tenant-a"), &decision(3, true));
    assert!(cache.get(&key(2, "a", "tenant-a")).is_none());
    assert!(cache.get(&key(1, "a", "tenant-b")).is_none());
    assert!(cache.get(&key(1, "b", "tenant-a")).is_none());
}

#[test]
fn a_new_module_generation_flushes_everything() {
    let mut cache = DecisionCache::default();
    cache.put(key(1, "a", "s"), &decision(3, true));
    cache.observe(4);
    assert_eq!(cache.len(), 0);
}

#[test]
fn policy_changed_invalidates_the_cache() {
    let mut cache = DecisionCache::default();
    cache.put(key(1, "a", "s"), &decision(3, true));
    cache.invalidate(&PolicyChanged {
        generation: 3,
        policy_id: None,
        policy_generation: None,
    });
    assert!(cache.get(&key(1, "a", "s")).is_none());
}

#[test]
fn zero_generation_decisions_are_never_stored() {
    let mut cache = DecisionCache::default();
    assert!(!cache.put(key(1, "a", "s"), &decision(0, true)));
}

fn caller(call_id: &str) -> CallerContext {
    CallerContext {
        agent: "a".into(),
        thread: None,
        flow: None,
        call_id: call_id.into(),
        origin: tinysecurity_bus::Origin::Operator,
        tier: tinysecurity_bus::AccessTier::Read,
    }
}

fn call(command: &str) -> ToolCall {
    ToolCall {
        tool: "shell".into(),
        arguments_json: "{}".into(),
        checks: vec![tinysecurity_bus::Check::Command {
            command: command.into(),
        }],
    }
}

#[test]
fn fingerprint_ignores_only_the_call_id() {
    let id = PolicyId("p1".into());
    let a = call_fingerprint(&id, &caller("c1"), &call("ls")).unwrap();
    assert_eq!(
        a,
        call_fingerprint(&id, &caller("c2"), &call("ls")).unwrap()
    );
    assert_ne!(
        a,
        call_fingerprint(&id, &caller("c1"), &call("rm x")).unwrap()
    );
    assert_ne!(
        a,
        call_fingerprint(&PolicyId("p2".into()), &caller("c1"), &call("ls")).unwrap()
    );
}

#[test]
fn scoped_response_must_match_the_active_policy_generation() {
    let active = ActivePolicy {
        policy_id: PolicyId("p".into()),
        policy_generation: 2,
        subject: SubjectScope {
            user: "u".into(),
            workspace: "w".into(),
            agent: "a".into(),
        },
        registration: String::new(),
    };
    let fp = "0".repeat(64);
    assert!(validate_scoped(5, 2, &fp, &active).is_ok());
    assert!(validate_scoped(5, 3, &fp, &active).is_err());
    assert!(validate_scoped(0, 2, &fp, &active).is_err());
    assert!(validate_scoped(5, 2, "short", &active).is_err());
}

#[test]
fn a_cached_decision_is_not_served_under_another_module_generation() {
    let mut cache = DecisionCache::default();
    cache.put(key(1, "a", "s"), &decision(3, true));
    assert!(cache.get(&key(1, "a", "s")).is_some());
    // Simulate a module generation change that was not yet observed as a flush.
    cache.module_generation = 4;
    assert!(cache.get(&key(1, "a", "s")).is_none());
}
