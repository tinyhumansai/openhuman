use super::*;

#[test]
fn relay_targets_are_limited_to_private_http_addresses() {
    assert_eq!(
        parse_private_http_target("http://192.168.1.74:7788/rpc").unwrap(),
        "192.168.1.74:7788".parse().unwrap()
    );
    assert_eq!(
        parse_private_http_target("http://[fd00::1]:7788/rpc").unwrap(),
        "[fd00::1]:7788".parse().unwrap()
    );
    assert!(parse_private_http_target("http://8.8.8.8:7788/rpc").is_err());
    assert!(parse_private_http_target("http://core.example:7788/rpc").is_err());
    assert!(parse_private_http_target("https://192.168.1.74:7788/rpc").is_err());
    assert!(parse_private_http_target("http://127.0.0.1:7788/rpc").is_err());
}

#[test]
fn relay_rewrites_only_its_secret_socket_io_path() {
    let request =
        b"GET /secret/socket.io/?EIO=4&transport=websocket HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    let target = "192.168.1.74:7788".parse().unwrap();
    let rewritten = rewrite_socket_request(request, "secret", target).unwrap();
    assert!(rewritten.starts_with(b"GET /socket.io/?EIO=4&transport=websocket HTTP/1.1\r\n"));
    assert!(String::from_utf8_lossy(&rewritten).contains("Host: 192.168.1.74:7788"));
    assert!(rewrite_socket_request(
        b"GET /secret/ws/live-voice?token=abc HTTP/1.1\r\n\r\n",
        "secret",
        target
    )
    .unwrap()
    .starts_with(b"GET /ws/live-voice?token=abc HTTP/1.1\r\n"));
    assert!(rewrite_socket_request(
        b"GET /other/socket.io/?EIO=4 HTTP/1.1\r\n\r\n",
        "secret",
        target
    )
    .is_err());
}
