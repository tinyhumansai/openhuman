use super::*;
#[test]
fn split_deduplicates_and_discards_blanks() {
    assert_eq!(
        parse_api_keys(" key-a, key-b;key-a\nkey-c\r\n"),
        vec!["key-a", "key-b", "key-c"]
    );
}
#[test]
fn routes_around_cooldown_and_revoked_keys() {
    let scope = "key-pool-test-routing";
    let keys = vec!["a".to_owned(), "b".to_owned()];
    let (i, first) = next_key(scope, &keys).unwrap();
    assert_eq!(first, "a");
    record_result(scope, i, Some(KeyFailure::RateLimited));
    assert_eq!(next_key(scope, &keys).unwrap().1, "b");
    record_result(scope, 1, Some(KeyFailure::Authentication));
    assert!(next_key(scope, &keys).is_none());
    assert_eq!(health(scope).len(), 2);
}
#[test]
fn exhaustion_never_reuses_an_attempted_key() {
    let scope = "key-pool-test-exhausted";
    let keys = vec!["a".to_owned(), "b".to_owned()];
    let (first, _) = next_key_excluding(scope, &keys, &[]).unwrap();
    let (second, _) = next_key_excluding(scope, &keys, &[first]).unwrap();
    assert_ne!(first, second);
    assert!(next_key_excluding(scope, &keys, &[first, second]).is_none());
}
#[test]
fn outcome_is_attributed_by_secret_not_stale_array_position() {
    let scope = "key-pool-test-rotated";
    let initial = vec!["a".to_owned(), "b".to_owned()];
    let (_, key) = next_key(scope, &initial).unwrap();
    let updated = vec!["b".to_owned(), "a".to_owned()];
    next_key(scope, &updated).unwrap();
    record_result_for(scope, &key, Some(KeyFailure::Authentication), None);
    assert_eq!(next_key(scope, &updated).unwrap().1, "b");
}
#[test]
fn updates_pool_without_losing_existing_health() {
    let scope = "key-pool-test-update";
    let initial = vec!["one".to_owned(), "two".to_owned()];
    next_key(scope, &initial);
    record_result(scope, 0, Some(KeyFailure::Authentication));
    let updated = vec!["one".to_owned(), "three".to_owned()];
    assert_eq!(next_key(scope, &updated).unwrap().1, "three");
    assert!(health(scope)[0].0);
}
#[test]
fn one_run_reuses_healthy_key_but_fails_over_after_rate_limit() {
    let scope = "key-pool-test-session-affinity";
    let keys = vec!["key-a".to_owned(), "key-b".to_owned()];
    let (first, _) = choose_key_for_run(
        scope,
        &keys,
        &[],
        RotationStrategy::RoundRobin,
        Some("run-1"),
    )
    .unwrap();
    let (sticky, _) = choose_key_for_run(
        scope,
        &keys,
        &[],
        RotationStrategy::RoundRobin,
        Some("run-1"),
    )
    .unwrap();
    assert_eq!(first, sticky);
    record_result(scope, first, Some(KeyFailure::RateLimited));
    let (fallback, _) = choose_key_for_run(
        scope,
        &keys,
        &[],
        RotationStrategy::RoundRobin,
        Some("run-1"),
    )
    .unwrap();
    assert_ne!(fallback, first);
}
#[test]
fn weighted_scoring_prefers_unleased_successful_credentials() {
    let fresh = KeyHealth::new(secret_id("fresh"));
    let mut loaded = fresh.clone();
    loaded.active = 2;
    assert!(weighted_score(&fresh) > weighted_score(&loaded));
    loaded.active = 0;
    loaded.consecutive_failures = 2;
    assert!(weighted_score(&fresh) > weighted_score(&loaded));
}
#[test]
fn pool_retains_only_salted_key_ids_not_raw_secrets() {
    let scope = "key-pool-test-no-secrets";
    let configured = vec!["sensitive_example_token".to_owned()];
    next_key(scope, &configured);
    let pools = registry().lock().unwrap();
    let key = &pools.get(scope).unwrap().keys[0];
    assert_eq!(key.id, secret_id("sensitive_example_token"));
    assert_ne!(key.id, secret_id("different_token"));
}
#[test]
fn deleted_model_scope_does_not_remove_sibling_key_health() {
    let prefix = "key-pool-test-model-prune:";
    let a = format!("{prefix}model-a");
    let b = format!("{prefix}model-b");
    next_key(&a, &["a".to_owned()]);
    next_key(&b, &["b".to_owned()]);
    forget(&a);
    let remaining = provider_snapshots(prefix);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].0, "model-b");
    forget_provider(prefix);
}
#[test]
fn deleted_provider_scopes_are_pruned() {
    let prefix = "key-pool-test-prune:";
    next_key("key-pool-test-prune:model-a", &["a".to_owned()]);
    next_key("key-pool-test-prune:model-b", &["b".to_owned()]);
    forget_provider(prefix);
    assert!(provider_snapshots(prefix).is_empty());
}
#[test]
fn least_busy_reserves_active_lease_before_returning_the_key() {
    // The two calls simulate simultaneous requests that do not yield between
    // choosing their credentials. The first selection must reserve its lease
    // before the second request makes its choice, otherwise both select A.
    let scope = "key-pool-test-atomic-reservation";
    let keys = vec!["first".to_owned(), "second".to_owned()];
    let (first_index, first) =
        choose_key_and_acquire_for_run(scope, &keys, &[], RotationStrategy::LeastBusy, None)
            .expect("first credential available");
    assert_eq!(first_index, 0);
    assert_eq!(snapshot(scope)[first_index].active_leases, 1);

    let (second_index, second) =
        choose_key_and_acquire_for_run(scope, &keys, &[], RotationStrategy::LeastBusy, None)
            .expect("second credential available");
    assert_ne!(
        first_index, second_index,
        "lease reservation must influence the next selection"
    );
    assert_eq!(snapshot(scope)[second_index].active_leases, 1);

    release(scope, &first);
    release(scope, &second);
    assert!(snapshot(scope).iter().all(|row| row.active_leases == 0));
    forget(scope);
}

#[test]
fn reservations_honor_excluded_keys_and_run_affinity_without_double_acquire() {
    let scope = "key-pool-test-atomic-exclusions";
    let keys = vec!["a".to_owned(), "b".to_owned()];
    let (selected, secret) = choose_key_and_acquire_for_run(
        scope,
        &keys,
        &[],
        RotationStrategy::RoundRobin,
        Some("session"),
    )
    .unwrap();
    let (sticky, again) = choose_key_and_acquire_for_run(
        scope,
        &keys,
        &[],
        RotationStrategy::RoundRobin,
        Some("session"),
    )
    .unwrap();
    assert_eq!(selected, sticky, "healthy session binding stays stable");
    assert_eq!(secret, again);
    assert_eq!(snapshot(scope)[selected].active_leases, 2);
    assert_eq!(
        choose_key_and_acquire_for_run(scope, &keys, &[0, 1], RotationStrategy::RoundRobin, None),
        None,
        "exhausted candidate set cannot reserve another lease"
    );

    release(scope, &secret);
    release(scope, &again);
    assert_eq!(snapshot(scope)[selected].active_leases, 0);
    forget(scope);
}

#[test]
fn leases_are_released_without_underflow() {
    let scope = "key-pool-test-lease";
    let keys = vec!["a".to_owned()];
    next_key(scope, &keys);
    acquire(scope, "a");
    assert_eq!(snapshot(scope)[0].active_leases, 1);
    release(scope, "a");
    release(scope, "a");
    assert_eq!(snapshot(scope)[0].active_leases, 0);
}

#[test]
fn late_success_must_not_clear_an_active_rate_limit_from_another_lease() {
    let scope = "key-pool-test-concurrent-429-success";
    let secret = "same-provider-credential";
    let keys = vec![secret.to_owned()];
    assert!(next_key(scope, &keys).is_some());
    record_result_for(scope, secret, Some(KeyFailure::RateLimited), Some(120_000));
    assert_eq!(snapshot(scope)[0].status, "cooldown");

    // Another request was already in flight and returned 200 after the 429.
    // This is NOT evidence that provider's cooldown has expired.
    record_result_for(scope, secret, None, None);
    assert_eq!(snapshot(scope)[0].status, "cooldown");
    assert!(next_key(scope, &keys).is_none());
    forget(scope);
}

#[test]
fn parallel_rate_limit_outcomes_cannot_shorten_the_existing_deadline() {
    let scope = "key-pool-test-parallel-429";
    let secret = "rate-limited-key";
    next_key(scope, &[secret.to_owned()]);
    record_result_for(scope, secret, Some(KeyFailure::RateLimited), Some(240_000));
    let first_deadline = {
        let pools = registry().lock().unwrap();
        pools.get(scope).unwrap().keys[0].cooldown_until.unwrap()
    };
    record_result_for(scope, secret, Some(KeyFailure::RateLimited), Some(1000));
    let second_deadline = {
        let pools = registry().lock().unwrap();
        pools.get(scope).unwrap().keys[0].cooldown_until.unwrap()
    };
    assert!(
        second_deadline >= first_deadline,
        "an overlapping 429 shortened an active cooldown"
    );
    forget(scope);
}

#[test]
fn session_affinity_cache_evicts_earliest_expiring_binding() {
    let scope = "key-pool-test-affinity-order";
    let keys = vec!["stable".to_owned()];
    next_key(scope, &keys);
    {
        let mut pools = registry().lock().unwrap();
        let pool = pools.get_mut(scope).unwrap();
        let base = Instant::now();
        let id = secret_id("stable");
        for i in 0..512u64 {
            pool.sticky
                .insert(format!("run-{i}"), (id, base + Duration::from_secs(60 + i)));
        }
    }

    choose_key_for_run(
        scope,
        &keys,
        &[],
        RotationStrategy::RoundRobin,
        Some("run-new"),
    )
    .unwrap();
    {
        let pools = registry().lock().unwrap();
        let sticky = &pools.get(scope).unwrap().sticky;
        assert_eq!(sticky.len(), 512);
        assert!(
            !sticky.contains_key("run-0"),
            "evict oldest/earliest expiry, not arbitrary HashMap entry"
        );
        assert!(sticky.contains_key("run-511"));
        assert!(sticky.contains_key("run-new"));
    }
    forget(scope);
}

#[test]
fn untrusted_correlation_ids_do_not_fill_session_affinity_with_unbounded_strings() {
    let scope = "key-pool-test-large-correlation";
    let keys = vec!["safe".to_owned()];
    let long_id = "r".repeat(64 * 1024);
    for run_id in [&long_id[..], "newline\nseparator", "control\u{7}bell"] {
        assert!(choose_key_for_run(
            scope,
            &keys,
            &[],
            RotationStrategy::RoundRobin,
            Some(run_id),
        )
        .is_some());
    }
    let pools = registry().lock().unwrap();
    assert!(pools.get(scope).unwrap().sticky.is_empty());
    drop(pools);
    forget(scope);
    // No test-only correlation scope may leak into the process-wide registry.
    assert!(!registry().lock().unwrap().contains_key(scope));
}

#[test]
fn quota_cooldown_exponentially_backs_off_without_overriding_server_retry_after() {
    assert_eq!(quota_cooldown(1, None), Duration::from_secs(60));
    assert_eq!(quota_cooldown(2, None), Duration::from_secs(120));
    assert_eq!(quota_cooldown(3, None), Duration::from_secs(240));
    assert_eq!(quota_cooldown(4, None), Duration::from_secs(300));
    assert_eq!(quota_cooldown(u32::MAX, None), Duration::from_secs(300));
    assert_eq!(quota_cooldown(3, Some(1_500)), Duration::from_millis(1_500));
    assert_eq!(quota_cooldown(3, Some(u64::MAX)), Duration::from_secs(300));
}

#[test]
fn repeated_rate_limits_increase_default_cooldown_without_exposing_secrets() {
    let scope = "key-pool-test-progressive-429";
    let key = "never-log-this-secret".to_string();
    next_key(scope, std::slice::from_ref(&key));
    record_result_for(scope, &key, Some(KeyFailure::RateLimited), None);
    let first = snapshot(scope)[0].cooldown_remaining_ms;
    assert!(first > 55_000 && first <= 60_000);
    record_result_for(scope, &key, Some(KeyFailure::RateLimited), None);
    let second = snapshot(scope)[0].cooldown_remaining_ms;
    assert!(second > 115_000 && second <= 120_000);
    reset(scope);
    record_result_for(scope, &key, Some(KeyFailure::RateLimited), None);
    let after_reset = snapshot(scope)[0].cooldown_remaining_ms;
    assert!(after_reset > 55_000 && after_reset <= 60_000);
}

#[test]
fn explicit_retry_after_is_honored_and_concurrent_shorter_limits_do_not_erase_it() {
    let scope = "key-pool-test-progressive-429-explicit";
    let key = "another-secret".to_string();
    next_key(scope, std::slice::from_ref(&key));
    record_result_for(scope, &key, Some(KeyFailure::RateLimited), Some(250_000));
    let first = snapshot(scope)[0].cooldown_remaining_ms;
    record_result_for(scope, &key, Some(KeyFailure::RateLimited), Some(2_000));
    let second = snapshot(scope)[0].cooldown_remaining_ms;
    assert!(first > 245_000 && first <= 250_000);
    assert!(
        second > 244_000,
        "later 429 must not shorten existing cooldown"
    );
}
