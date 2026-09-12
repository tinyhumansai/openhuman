use super::*;
use tokio::io::{duplex, AsyncReadExt};

#[tokio::test]
async fn stdio_loop_writes_one_line_per_response() {
    let (mut client_write, server_read) = duplex(4096);
    let (server_write, mut client_read) = duplex(4096);

    let server = tokio::spawn(async move { run_stdio(server_read, server_write).await });

    client_write
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"ping"}
"#,
        )
        .await
        .unwrap();
    drop(client_write);

    let mut output = String::new();
    client_read.read_to_string(&mut output).await.unwrap();
    server.await.unwrap().unwrap();

    let response: serde_json::Value = serde_json::from_str(output.trim()).expect("json response");
    assert_eq!(response["id"], 1);
    assert!(response["result"].is_object());
}

#[test]
fn cli_help_exits_zero() {
    assert!(run_stdio_from_cli(&["--help".into()]).is_ok());
}

#[test]
fn cli_verbose_advances_to_next_arg() {
    assert!(run_stdio_from_cli(&["--verbose".into(), "--help".into()]).is_ok());
}
