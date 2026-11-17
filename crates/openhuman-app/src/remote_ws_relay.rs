use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteSocketRelay {
    base_url: String,
    path: String,
    live_voice_path: String,
}

static RELAYS: OnceLock<Mutex<HashMap<String, RemoteSocketRelay>>> = OnceLock::new();

/// Start a loopback-only TCP relay for Socket.IO on a private-LAN HTTP core.
/// The random path keeps other local clients from reusing the relay.
#[tauri::command]
pub(crate) async fn relay_remote_socket(url: String) -> Result<RemoteSocketRelay, String> {
    let target = parse_private_http_target(&url)?;
    let relays = RELAYS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut relays = relays.lock().await;
    if let Some(relay) = relays.get(&url) {
        return Ok(relay.clone());
    }

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| format!("starting local socket relay: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("reading local socket relay address: {error}"))?;
    let secret = random_secret();
    let relay = RemoteSocketRelay {
        base_url: format!("http://{address}"),
        path: format!("/{secret}/socket.io/"),
        live_voice_path: format!("/{secret}/ws/live-voice"),
    };
    let target = Arc::new(target);
    let secret_for_task = Arc::new(secret);
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else {
                break;
            };
            let target = Arc::clone(&target);
            let secret = Arc::clone(&secret_for_task);
            tokio::spawn(async move {
                if let Err(error) = relay_connection(client, *target, &secret).await {
                    log::debug!("[socket relay] connection closed: {error}");
                }
            });
        }
    });
    relays.insert(url, relay.clone());
    Ok(relay)
}

fn parse_private_http_target(url: &str) -> Result<SocketAddr, String> {
    let parsed = url::Url::parse(url).map_err(|_| "core URL is invalid".to_owned())?;
    if parsed.scheme() != "http"
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("socket relay requires a plain HTTP core URL".to_owned());
    }
    let host = parsed
        .host_str()
        .ok_or("core URL has no host")?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let ip = host
        .parse::<IpAddr>()
        .map_err(|_| "socket relay requires a private IP address".to_owned())?;
    if !is_private_ip(ip) || ip.is_loopback() {
        return Err("socket relay is limited to private-LAN IP addresses".to_owned());
    }
    let port = parsed
        .port_or_known_default()
        .ok_or("core URL has no port")?;
    Ok(SocketAddr::new(ip, port))
}

fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                mapped.is_private() || mapped.is_link_local()
            } else {
                ip.is_unique_local() || ip.is_unicast_link_local()
            }
        }
    }
}

fn random_secret() -> String {
    rand::random::<[u8; 16]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn relay_connection(
    mut client: TcpStream,
    target: SocketAddr,
    secret: &str,
) -> Result<(), String> {
    let request = read_request_header(&mut client)
        .await
        .map_err(|error| format!("reading socket handshake: {error}"))?;
    let rewritten = rewrite_socket_request(&request, secret, target)?;
    let mut upstream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(target))
        .await
        .map_err(|_| "connecting to private-LAN core timed out".to_owned())?
        .map_err(|error| format!("connecting to private-LAN core: {error}"))?;
    upstream
        .write_all(&rewritten)
        .await
        .map_err(|error| format!("forwarding socket handshake: {error}"))?;
    tokio::io::copy_bidirectional(&mut client, &mut upstream)
        .await
        .map_err(|error| format!("relaying socket connection: {error}"))?;
    Ok(())
}

async fn read_request_header(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    while request.len() < 16 * 1024 {
        stream.read_exact(&mut byte).await?;
        request.push(byte[0]);
        if request.ends_with(b"\r\n\r\n") {
            return Ok(request);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "socket handshake headers exceed 16 KiB",
    ))
}

fn rewrite_socket_request(
    request: &[u8],
    secret: &str,
    target: SocketAddr,
) -> Result<Vec<u8>, String> {
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("socket handshake has incomplete headers")?;
    let first_line_end = request[..header_end + 2]
        .windows(2)
        .position(|window| window == b"\r\n")
        .ok_or("socket handshake has no request line")?;
    let request_line = std::str::from_utf8(&request[..first_line_end])
        .map_err(|_| "socket handshake request line is not UTF-8")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("socket handshake has no method")?;
    let path = parts.next().ok_or("socket handshake has no path")?;
    let version = parts.next().ok_or("socket handshake has no HTTP version")?;
    if parts.next().is_some() {
        return Err("socket handshake request line is malformed".to_owned());
    }
    let secret_prefix = format!("/{secret}/");
    let path_and_query = path
        .strip_prefix(&secret_prefix)
        .ok_or("socket relay path is invalid")?;
    let (path, query) = path_and_query
        .split_once('?')
        .map_or((path_and_query, ""), |(path, query)| (path, query));
    let target_path = if path == "socket.io/" {
        "/socket.io/"
    } else if path == "ws/live-voice" {
        "/ws/live-voice"
    } else {
        return Err("socket relay path is invalid".to_owned());
    };
    let query = if query.is_empty() {
        String::new()
    } else {
        format!("?{query}")
    };
    let line = format!("{method} {target_path}{query} {version}");
    let mut rewritten = Vec::with_capacity(request.len());
    rewritten.extend_from_slice(line.as_bytes());
    rewritten.extend_from_slice(b"\r\n");
    let headers_start = (first_line_end + 2).min(header_end);
    let headers = std::str::from_utf8(&request[headers_start..header_end])
        .map_err(|_| "socket handshake headers are not UTF-8")?;
    for header in headers.split("\r\n").filter(|header| !header.is_empty()) {
        if header
            .split_once(':')
            .is_some_and(|(name, _)| name.eq_ignore_ascii_case("host"))
        {
            rewritten.extend_from_slice(format!("Host: {target}\r\n").as_bytes());
        } else {
            rewritten.extend_from_slice(header.as_bytes());
            rewritten.extend_from_slice(b"\r\n");
        }
    }
    rewritten.extend_from_slice(b"\r\n");
    Ok(rewritten)
}

#[cfg(test)]
#[path = "remote_ws_relay_tests.rs"]
mod tests;
