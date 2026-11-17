//! Browser login transport for the TUI. Backend exchange, validation and
//! persistence remain in the shared TinyHumans session owner.

use std::{sync::Arc, time::Duration};

use openhuman_rpc::tinyhumans::{CoreLink, SessionManager, SessionState};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoginProvider {
    #[default]
    Google,
    Github,
    Twitter,
    Discord,
}

impl LoginProvider {
    fn slug(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Github => "github",
            Self::Twitter => "twitter",
            Self::Discord => "discord",
        }
    }
}

/// A cloneable handle which interrupts the callback wait without logging secrets.
#[derive(Clone)]
pub struct LoginCancellation(watch::Sender<bool>);

impl LoginCancellation {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
}

/// One browser round trip. Dropping it closes its loopback callback listener.
pub struct BrowserLogin<L: CoreLink> {
    listener: TcpListener,
    state: String,
    login_url: String,
    backend_url: String,
    port: u16,
    manager: Arc<SessionManager<L>>,
    cancelled: watch::Receiver<bool>,
    cancellation: LoginCancellation,
}

impl<L: CoreLink> BrowserLogin<L> {
    /// Bind before opening a browser so an immediate redirect cannot be lost.
    pub async fn start(
        manager: Arc<SessionManager<L>>,
        provider: LoginProvider,
    ) -> Result<Self, String> {
        let client = manager
            .client()
            .await
            .map_err(|_| "Could not resolve the login backend.".to_string())?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| "Could not start the login callback listener.".to_string())?;
        let port = listener
            .local_addr()
            .map_err(|_| "Could not read the login callback address.".to_string())?
            .port();
        let state = uuid::Uuid::new_v4().simple().to_string();
        let redirect = format!("http://127.0.0.1:{port}/auth?state={state}");
        let login_url = format!(
            "{}/auth/{}/login?redirect=app&redirectUri={}",
            client.base_url().trim_end_matches('/'),
            provider.slug(),
            percent_encode(&redirect)
        );
        let (sender, cancelled) = watch::channel(false);
        Ok(Self {
            listener,
            state,
            login_url,
            backend_url: client.base_url().to_string(),
            port,
            manager,
            cancelled,
            cancellation: LoginCancellation(sender),
        })
    }

    pub fn login_url(&self) -> &str {
        &self.login_url
    }
    /// Port to forward when running the TUI over SSH.
    pub fn callback_port(&self) -> u16 {
        self.port
    }
    pub fn cancellation(&self) -> LoginCancellation {
        self.cancellation.clone()
    }

    /// Wait for a valid callback, then let the manager exchange or validate the
    /// credential. Cancellation interrupts the browser wait; once accepted,
    /// storage completes atomically under the manager's mutation lock.
    pub async fn wait(mut self, timeout: Duration) -> Result<SessionState, String> {
        if *self.cancelled.borrow() {
            return Err("Login cancelled.".into());
        }
        let token = tokio::select! {
            biased;
            _ = self.cancelled.changed() => return Err("Login cancelled.".into()),
            result = tokio::time::timeout(timeout, accept_callback(&self.listener, &self.state)) => {
                result.map_err(|_| "Timed out waiting for browser login.".to_string())??
            }
        };
        let result = if is_login_token(&token) {
            self.manager
                .login_with_token_for_backend(&token, &self.backend_url)
                .await
        } else {
            self.manager
                .store_session_token_for_backend(&token, None, &self.backend_url)
                .await
        };
        result.map_err(|error| {
            super::safe_session_error(
                error,
                "The backend could not complete sign-in. Please try again.",
            )
        })
    }
}

fn is_login_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn percent_decode(value: &str) -> Option<String> {
    let mut result = Vec::new();
    let mut bytes = value.bytes();
    while let Some(b) = bytes.next() {
        result.push(match b {
            b'+' => b' ',
            b'%' => {
                let a = (bytes.next()? as char).to_digit(16)?;
                let b = (bytes.next()? as char).to_digit(16)?;
                (a * 16 + b) as u8
            }
            _ => b,
        });
    }
    String::from_utf8(result).ok()
}

enum Callback {
    Ignore(&'static str),
    Rejected,
    Token(Zeroizing<String>),
}

fn classify(head: &str, expected_state: &str, port: u16) -> Callback {
    let mut hosts = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.eq_ignore_ascii_case("host"));
    if hosts.next().map(|(_, value)| value.trim()) != Some(format!("127.0.0.1:{port}").as_str())
        || hosts.next().is_some()
    {
        return Callback::Ignore("400 Bad Request");
    }
    // Top-level browser redirects carry no Origin. Cross-origin fetches are
    // not the browser round trip we initiated.
    if head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .any(|(key, _)| key.eq_ignore_ascii_case("origin"))
    {
        return Callback::Ignore("400 Bad Request");
    }
    let mut parts = head.lines().next().unwrap_or_default().split_whitespace();
    if parts.next() != Some("GET") {
        return Callback::Ignore("405 Method Not Allowed");
    }
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/auth" {
        return Callback::Ignore("404 Not Found");
    }
    let mut params = std::collections::HashMap::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let Some(key) = percent_decode(key) else {
            return Callback::Ignore("400 Bad Request");
        };
        let Some(value) = percent_decode(value) else {
            return Callback::Ignore("400 Bad Request");
        };
        if params.insert(key, Zeroizing::new(value)).is_some() {
            return Callback::Ignore("400 Bad Request");
        }
    }
    if params.get("state").map(|s| s.as_str()) != Some(expected_state) {
        return Callback::Ignore("400 Bad Request");
    }
    if params.contains_key("error") {
        return Callback::Rejected;
    }
    match params.remove("token").filter(|t| !t.trim().is_empty()) {
        Some(token) => Callback::Token(token),
        None => Callback::Ignore("400 Bad Request"),
    }
}

async fn accept_callback(listener: &TcpListener, state: &str) -> Result<Zeroizing<String>, String> {
    loop {
        let (mut stream, peer) = listener
            .accept()
            .await
            .map_err(|_| "The login callback listener failed.".to_string())?;
        if !peer.ip().is_loopback() {
            continue;
        }
        let head = match tokio::time::timeout(Duration::from_secs(5), read_head(&mut stream)).await
        {
            Ok(Some(head)) => head,
            _ => continue,
        };
        let port = listener
            .local_addr()
            .map_err(|_| "Could not read the login callback address.".to_string())?
            .port();
        let outcome = classify(&head, state, port);
        let (status, body) = match &outcome {
            Callback::Ignore(status) => (*status, "Invalid login callback."),
            Callback::Rejected => ("200 OK", "Login failed. Return to OpenHuman to try again."),
            Callback::Token(_) => (
                "200 OK",
                "Login received. Return to OpenHuman to finish signing in. You can close this tab.",
            ),
        };
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n\r\n{body}", body.len());
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            stream.write_all(response.as_bytes()),
        )
        .await;
        match outcome {
            Callback::Token(token) => return Ok(token),
            Callback::Rejected => {
                return Err("The provider rejected sign-in. Please try again.".into())
            }
            Callback::Ignore(_) => {}
        }
    }
}

async fn read_head(stream: &mut TcpStream) -> Option<Zeroizing<String>> {
    let mut bytes = Zeroizing::new(Vec::new());
    while bytes.len() < 8192 {
        let mut buffer = [0; 1024];
        let count = stream.read(&mut buffer).await.ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|b| b == b"\r\n\r\n") {
            return String::from_utf8(bytes.to_vec()).ok().map(Zeroizing::new);
        }
    }
    None
}

/// Open the system browser without shell interpolation or terminal output.
pub fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    let mut child = command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| {
            "Could not open the browser. Open the displayed login URL manually.".to_string()
        })?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
#[path = "browser_login_tests.rs"]
mod tests;
