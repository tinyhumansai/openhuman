use super::*;

#[tokio::test]
async fn ensure_local_http_binds_loopback_with_token_and_is_idempotent() {
    let a = ensure_local_http().await.expect("first start");
    assert!(
        a.addr.ip().is_loopback(),
        "must bind loopback only, got {}",
        a.addr
    );
    assert_ne!(a.addr.port(), 0, "must report a concrete bound port");
    assert!(!a.token.is_empty(), "must mint a bearer token");
    // Singleton: a second call returns the same endpoint, not a new server.
    let b = ensure_local_http().await.expect("second start");
    assert_eq!(
        a.addr, b.addr,
        "ensure_local_http must be a process-wide singleton"
    );
    assert_eq!(a.token, b.token, "the token must be stable across calls");
}

#[test]
fn mint_token_is_long_and_unique() {
    let t1 = mint_token();
    let t2 = mint_token();
    assert_eq!(t1.len(), 64, "two simple UUIDs → 64 hex chars");
    assert_ne!(t1, t2, "tokens must be random per mint");
}
