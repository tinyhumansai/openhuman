use percent_encoding::percent_decode_str;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use crate::error::Error;

pub struct BoundServer {
    pub port: u16,
    listener: TcpListener,
}

/// Bind to 127.0.0.1:{port} (or OS-assigned if port is None).
/// Returns BoundServer with the actual bound port.
pub async fn bind(port: Option<u16>) -> Result<BoundServer, Error> {
    let addr = format!("127.0.0.1:{}", port.unwrap_or(0));
    let listener = TcpListener::bind(&addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            Error::Callback(format!(
                "port {} is already in use; close other instances and retry",
                port.unwrap_or(0)
            ))
        } else {
            Error::Io(e)
        }
    })?;
    let actual_port = listener.local_addr().map_err(Error::Io)?.port();
    Ok(BoundServer {
        port: actual_port,
        listener,
    })
}

/// Wait for one /auth/callback?code=...&state=... request.
/// Returns (code, state). Ignores other requests (e.g. favicon).
pub async fn wait_for_callback(
    server: BoundServer,
    callback_path: &str,
) -> Result<(String, String), Error> {
    let BoundServer { listener, .. } = server;
    loop {
        let (mut stream, _) = listener.accept().await?;
        let buf = read_headers(&mut stream).await?;
        let request = String::from_utf8_lossy(&buf);

        if !is_callback_request(&request, callback_path) {
            let _ = stream
                .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                .await;
            continue;
        }

        let html = "<html><body>Login successful. You can close this tab.</body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = stream.write_all(response.as_bytes()).await;
        return parse_callback(&request);
    }
}

async fn read_headers(stream: &mut tokio::net::TcpStream) -> Result<Vec<u8>, Error> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 512];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if buf.len() + n >= 16384 {
            return Err(Error::Callback("request too large".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    Ok(buf)
}

fn is_callback_request(request: &str, callback_path: &str) -> bool {
    let path = request
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("");
    path.starts_with(callback_path) && path.contains("code=")
}

fn parse_callback(request: &str) -> Result<(String, String), Error> {
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| Error::Callback("malformed HTTP request".into()))?;

    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");

    let mut code = None;
    let mut state = None;

    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let decoded = percent_decode_str(v)
                .decode_utf8()
                .map_err(|_| Error::Callback(format!("param '{k}' is not valid UTF-8")))?
                .into_owned();
            match k {
                "code" => code = Some(decoded),
                "state" => state = Some(decoded),
                _ => {}
            }
        }
    }

    let code = code.ok_or_else(|| Error::Callback("missing code param".into()))?;
    let state = state.ok_or_else(|| Error::Callback("missing state param".into()))?;
    Ok((code, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(path: &str) -> String {
        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n")
    }

    #[test]
    fn parses_normal_callback() {
        let (code, state) = parse_callback(&get("/auth/callback?code=abc123&state=xyz")).unwrap();
        assert_eq!(code, "abc123");
        assert_eq!(state, "xyz");
    }

    #[test]
    fn decodes_percent_encoded_params() {
        let (code, state) =
            parse_callback(&get("/auth/callback?code=ab%2Bcd&state=x%3Dy")).unwrap();
        assert_eq!(code, "ab+cd");
        assert_eq!(state, "x=y");
    }

    #[test]
    fn extra_params_are_ignored() {
        let (code, state) =
            parse_callback(&get("/auth/callback?code=c&state=s&session_state=ignored")).unwrap();
        assert_eq!(code, "c");
        assert_eq!(state, "s");
    }

    #[test]
    fn missing_code_returns_error() {
        let err = parse_callback(&get("/auth/callback?state=s")).unwrap_err();
        assert!(err.to_string().contains("missing code"));
    }

    #[test]
    fn missing_state_returns_error() {
        let err = parse_callback(&get("/auth/callback?code=c")).unwrap_err();
        assert!(err.to_string().contains("missing state"));
    }

    #[test]
    fn non_callback_path_is_not_callback() {
        assert!(!is_callback_request(&get("/favicon.ico"), "/auth/callback"));
        assert!(!is_callback_request(&get("/"), "/auth/callback"));
    }

    #[test]
    fn callback_path_with_code_is_callback() {
        assert!(is_callback_request(
            &get("/auth/callback?code=abc&state=xyz"),
            "/auth/callback"
        ));
    }

    #[test]
    fn anthropic_callback_path_is_callback() {
        assert!(is_callback_request(
            &get("/callback?code=abc&state=xyz"),
            "/callback"
        ));
    }

    #[test]
    fn auth_callback_path_does_not_match_anthropic_request() {
        // A request to /callback must not be accepted when callback_path is /auth/callback.
        assert!(!is_callback_request(
            &get("/callback?code=abc&state=xyz"),
            "/auth/callback"
        ));
    }

    #[tokio::test]
    async fn bind_dynamic_port_returns_nonzero_port() {
        let server = bind(None).await.expect("bind should succeed");
        assert!(server.port > 0, "dynamic port should be nonzero");
    }

    #[tokio::test]
    async fn bind_specific_port_returns_that_port() {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let free_port = probe.local_addr().unwrap().port();
        drop(probe);

        let server = bind(Some(free_port)).await.expect("bind should succeed");
        assert_eq!(server.port, free_port);
    }
}
