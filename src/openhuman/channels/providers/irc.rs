use crate::openhuman::channels::traits::{Channel, ChannelMessage, SendMessage};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, Mutex};

// Use tokio_rustls's re-export of rustls types
use tokio_rustls::rustls;

/// Read timeout for IRC — if no data arrives within this duration, the
/// connection is considered dead. IRC servers typically PING every 60-120s.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Monotonic counter to ensure unique message IDs under burst traffic.
static MSG_SEQ: AtomicU64 = AtomicU64::new(0);

/// IRC over TLS channel.
///
/// Connects to an IRC server using TLS, joins configured channels,
/// and forwards PRIVMSG messages to the `OpenHuman` message bus.
/// Supports both channel messages and private messages (DMs).
pub struct IrcChannel {
    server: String,
    port: u16,
    nickname: String,
    username: String,
    channels: Vec<String>,
    allowed_users: Vec<String>,
    server_password: Option<String>,
    nickserv_password: Option<String>,
    sasl_password: Option<String>,
    verify_tls: bool,
    /// Shared write half of the TLS stream for sending messages.
    writer: Arc<Mutex<Option<WriteHalf>>>,
}

type WriteHalf = tokio::io::WriteHalf<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

/// Style instruction prepended to every IRC message before it reaches the LLM.
/// IRC clients render plain text only — no markdown, no HTML, no XML.
const IRC_STYLE_PREFIX: &str = "\
[context: you are responding over IRC. \
Plain text only. No markdown, no tables, no XML/HTML tags. \
Never use triple backtick code fences. Use a single blank line to separate blocks instead. \
Be terse and concise. \
Use short lines. Avoid walls of text.]\n";

/// Reserved bytes for the server-prepended sender prefix (`:nick!user@host `).
const SENDER_PREFIX_RESERVE: usize = 64;

/// A parsed IRC message.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IrcMessage {
    prefix: Option<String>,
    command: String,
    params: Vec<String>,
}

impl IrcMessage {
    /// Parse a raw IRC line into an `IrcMessage`.
    ///
    /// IRC format: `[:<prefix>] <command> [<params>] [:<trailing>]`
    fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return None;
        }

        let (prefix, rest) = if let Some(stripped) = line.strip_prefix(':') {
            let space = stripped.find(' ')?;
            (Some(stripped[..space].to_string()), &stripped[space + 1..])
        } else {
            (None, line)
        };

        // Split at trailing (first `:` after command/params)
        let (params_part, trailing) = if let Some(colon_pos) = rest.find(" :") {
            (&rest[..colon_pos], Some(&rest[colon_pos + 2..]))
        } else {
            (rest, None)
        };

        let mut parts: Vec<&str> = params_part.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let command = parts.remove(0).to_uppercase();
        let mut params: Vec<String> = parts.iter().map(std::string::ToString::to_string).collect();
        if let Some(t) = trailing {
            params.push(t.to_string());
        }

        Some(IrcMessage {
            prefix,
            command,
            params,
        })
    }

    /// Extract the nickname from the prefix (nick!user@host → nick).
    fn nick(&self) -> Option<&str> {
        self.prefix.as_ref().and_then(|p| {
            let end = p.find('!').unwrap_or(p.len());
            let nick = &p[..end];
            if nick.is_empty() {
                None
            } else {
                Some(nick)
            }
        })
    }
}

/// Encode SASL PLAIN credentials: base64(\0nick\0password).
fn encode_sasl_plain(nick: &str, password: &str) -> String {
    // Simple base64 encoder — avoids adding a base64 crate dependency.
    // The project's Discord channel uses a similar inline approach.
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let input = format!("\0{nick}\0{password}");
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(CHARS[(triple >> 18 & 0x3F) as usize] as char);
        out.push(CHARS[(triple >> 12 & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            out.push(CHARS[(triple >> 6 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }

        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }

    out
}

/// Split a message into lines safe for IRC transmission.
///
/// IRC is a line-based protocol — `\r\n` terminates each command, so any
/// newline inside a PRIVMSG payload would truncate the message and turn the
/// remainder into garbled/invalid IRC commands.
///
/// This function:
/// 1. Splits on `\n` (and strips `\r`) so each logical line becomes its own PRIVMSG.
/// 2. Splits any line that exceeds `max_bytes` at a safe UTF-8 boundary.
/// 3. Skips empty lines to avoid sending blank PRIVMSGs.
fn split_message(message: &str, max_bytes: usize) -> Vec<String> {
    let mut chunks = Vec::new();

    // Guard against max_bytes == 0 to prevent infinite loop
    if max_bytes == 0 {
        let mut full = String::new();
        for l in message
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .filter(|l| !l.is_empty())
        {
            if !full.is_empty() {
                full.push(' ');
            }
            full.push_str(l);
        }
        if full.is_empty() {
            chunks.push(String::new());
        } else {
            chunks.push(full);
        }
        return chunks;
    }

    for line in message.split('\n') {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }

        if line.len() <= max_bytes {
            chunks.push(line.to_string());
            continue;
        }

        // Line exceeds max_bytes — split at safe UTF-8 boundaries
        let mut remaining = line;
        while !remaining.is_empty() {
            if remaining.len() <= max_bytes {
                chunks.push(remaining.to_string());
                break;
            }

            let mut split_at = max_bytes;
            while split_at > 0 && !remaining.is_char_boundary(split_at) {
                split_at -= 1;
            }
            if split_at == 0 {
                // No valid boundary found going backward — advance forward instead
                split_at = max_bytes;
                while split_at < remaining.len() && !remaining.is_char_boundary(split_at) {
                    split_at += 1;
                }
            }

            chunks.push(remaining[..split_at].to_string());
            remaining = &remaining[split_at..];
        }
    }

    if chunks.is_empty() {
        chunks.push(String::new());
    }

    chunks
}

/// Configuration for constructing an `IrcChannel`.
pub struct IrcChannelConfig {
    pub server: String,
    pub port: u16,
    pub nickname: String,
    pub username: Option<String>,
    pub channels: Vec<String>,
    pub allowed_users: Vec<String>,
    pub server_password: Option<String>,
    pub nickserv_password: Option<String>,
    pub sasl_password: Option<String>,
    pub verify_tls: bool,
}

impl IrcChannel {
    pub fn new(cfg: IrcChannelConfig) -> Self {
        let username = cfg.username.unwrap_or_else(|| cfg.nickname.clone());
        Self {
            server: cfg.server,
            port: cfg.port,
            nickname: cfg.nickname,
            username,
            channels: cfg.channels,
            allowed_users: cfg.allowed_users,
            server_password: cfg.server_password,
            nickserv_password: cfg.nickserv_password,
            sasl_password: cfg.sasl_password,
            verify_tls: cfg.verify_tls,
            writer: Arc::new(Mutex::new(None)),
        }
    }

    fn is_user_allowed(&self, nick: &str) -> bool {
        if self.allowed_users.iter().any(|u| u == "*") {
            return true;
        }
        self.allowed_users
            .iter()
            .any(|u| u.eq_ignore_ascii_case(nick))
    }

    /// Create a TLS connection to the IRC server.
    async fn connect(
        &self,
    ) -> anyhow::Result<tokio_rustls::client::TlsStream<tokio::net::TcpStream>> {
        let addr = format!("{}:{}", self.server, self.port);
        let tcp = tokio::net::TcpStream::connect(&addr).await?;

        let tls_config = if self.verify_tls {
            let root_store: rustls::RootCertStore =
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned().collect();
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth()
        } else {
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerify))
                .with_no_client_auth()
        };

        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_config));
        let domain = rustls::pki_types::ServerName::try_from(self.server.clone())?;
        let tls = connector.connect(domain, tcp).await?;

        Ok(tls)
    }

    /// Send a raw IRC line (appends \r\n).
    async fn send_raw(writer: &mut WriteHalf, line: &str) -> anyhow::Result<()> {
        let data = format!("{line}\r\n");
        writer.write_all(data.as_bytes()).await?;
        writer.flush().await?;
        Ok(())
    }
}

/// Certificate verifier that accepts any certificate (for `verify_tls=false`).
#[derive(Debug)]
struct NoVerify;

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[async_trait]
#[allow(clippy::too_many_lines)]
impl Channel for IrcChannel {
    fn name(&self) -> &str {
        "irc"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        let mut guard = self.writer.lock().await;
        let writer = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("IRC not connected"))?;

        // Calculate safe payload size:
        // 512 - sender prefix (~64 bytes for :nick!user@host) - "PRIVMSG " - target - " :" - "\r\n"
        let overhead = SENDER_PREFIX_RESERVE + 10 + message.recipient.len() + 2;
        let max_payload = 512_usize.saturating_sub(overhead);
        let chunks = split_message(&message.content, max_payload);

        for chunk in chunks {
            Self::send_raw(writer, &format!("PRIVMSG {} :{chunk}", message.recipient)).await?;
        }

        Ok(())
    }

    async fn listen(&self, tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        let mut current_nick = self.nickname.clone();
        tracing::info!(
            "IRC channel connecting to {}:{} as {}...",
            self.server,
            self.port,
            current_nick
        );

        let tls = self.connect().await?;
        let (reader, mut writer) = tokio::io::split(tls);

        // --- SASL negotiation ---
        if self.sasl_password.is_some() {
            Self::send_raw(&mut writer, "CAP REQ :sasl").await?;
        }

        // --- Server password ---
        if let Some(ref pass) = self.server_password {
            Self::send_raw(&mut writer, &format!("PASS {pass}")).await?;
        }

        // --- Nick/User registration ---
        Self::send_raw(&mut writer, &format!("NICK {current_nick}")).await?;
        Self::send_raw(
            &mut writer,
            &format!("USER {} 0 * :OpenHuman", self.username),
        )
        .await?;

        // Store writer for send()
        {
            let mut guard = self.writer.lock().await;
            *guard = Some(writer);
        }

        let mut buf_reader = BufReader::new(reader);
        let mut line = String::new();
        let mut registered = false;
        let mut sasl_pending = self.sasl_password.is_some();

        loop {
            line.clear();
            let n = tokio::time::timeout(READ_TIMEOUT, buf_reader.read_line(&mut line))
                .await
                .map_err(|_| {
                    anyhow::anyhow!("IRC read timed out (no data for {READ_TIMEOUT:?})")
                })??;
            if n == 0 {
                anyhow::bail!("IRC connection closed by server");
            }

            let Some(msg) = IrcMessage::parse(&line) else {
                continue;
            };

            match msg.command.as_str() {
                "PING" => {
                    let token = msg.params.first().map_or("", String::as_str);
                    let mut guard = self.writer.lock().await;
                    if let Some(ref mut w) = *guard {
                        Self::send_raw(w, &format!("PONG :{token}")).await?;
                    }
                }

                // CAP responses for SASL
                "CAP" => {
                    if sasl_pending && msg.params.iter().any(|p| p.contains("sasl")) {
                        if msg.params.iter().any(|p| p.contains("ACK")) {
                            // CAP * ACK :sasl — server accepted, start SASL auth
                            let mut guard = self.writer.lock().await;
                            if let Some(ref mut w) = *guard {
                                Self::send_raw(w, "AUTHENTICATE PLAIN").await?;
                            }
                        } else if msg.params.iter().any(|p| p.contains("NAK")) {
                            // CAP * NAK :sasl — server rejected SASL, proceed without it
                            tracing::warn!(
                                "IRC server does not support SASL, continuing without it"
                            );
                            sasl_pending = false;
                            let mut guard = self.writer.lock().await;
                            if let Some(ref mut w) = *guard {
                                Self::send_raw(w, "CAP END").await?;
                            }
                        }
                    }
                }

                "AUTHENTICATE" => {
                    // Server sends "AUTHENTICATE +" to request credentials
                    if sasl_pending && msg.params.first().is_some_and(|p| p == "+") {
                        // sasl_password is loaded from runtime config, not hard-coded
                        if let Some(password) = self.sasl_password.as_deref() {
                            let encoded = encode_sasl_plain(&current_nick, password);
                            let mut guard = self.writer.lock().await;
                            if let Some(ref mut w) = *guard {
                                Self::send_raw(w, &format!("AUTHENTICATE {encoded}")).await?;
                            }
                        } else {
                            // SASL was requested but no password is configured; abort SASL
                            tracing::warn!(
                                "SASL authentication requested but no SASL password is configured; aborting SASL"
                            );
                            sasl_pending = false;
                            let mut guard = self.writer.lock().await;
                            if let Some(ref mut w) = *guard {
                                Self::send_raw(w, "CAP END").await?;
                            }
                        }
                    }
                }

                // RPL_SASLSUCCESS (903) — SASL done, end CAP
                "903" => {
                    sasl_pending = false;
                    let mut guard = self.writer.lock().await;
                    if let Some(ref mut w) = *guard {
                        Self::send_raw(w, "CAP END").await?;
                    }
                }

                // SASL failure (904, 905, 906, 907)
                "904" | "905" | "906" | "907" => {
                    tracing::warn!("IRC SASL authentication failed ({})", msg.command);
                    sasl_pending = false;
                    let mut guard = self.writer.lock().await;
                    if let Some(ref mut w) = *guard {
                        Self::send_raw(w, "CAP END").await?;
                    }
                }

                // RPL_WELCOME — registration complete
                "001" => {
                    registered = true;
                    tracing::info!("IRC registered as {}", current_nick);

                    // NickServ authentication
                    if let Some(ref pass) = self.nickserv_password {
                        let mut guard = self.writer.lock().await;
                        if let Some(ref mut w) = *guard {
                            Self::send_raw(w, &format!("PRIVMSG NickServ :IDENTIFY {pass}"))
                                .await?;
                        }
                    }

                    // Join channels
                    for chan in &self.channels {
                        let mut guard = self.writer.lock().await;
                        if let Some(ref mut w) = *guard {
                            Self::send_raw(w, &format!("JOIN {chan}")).await?;
                        }
                    }
                }

                // ERR_NICKNAMEINUSE (433)
                "433" => {
                    let alt = format!("{current_nick}_");
                    tracing::warn!("IRC nickname {current_nick} is in use, trying {alt}");
                    let mut guard = self.writer.lock().await;
                    if let Some(ref mut w) = *guard {
                        Self::send_raw(w, &format!("NICK {alt}")).await?;
                    }
                    current_nick = alt;
                }

                "PRIVMSG" => {
                    if !registered {
                        continue;
                    }

                    let target = msg.params.first().map_or("", String::as_str);
                    let text = msg.params.get(1).map_or("", String::as_str);
                    let sender_nick = msg.nick().unwrap_or("unknown");

                    // Skip messages from NickServ/ChanServ
                    if sender_nick.eq_ignore_ascii_case("NickServ")
                        || sender_nick.eq_ignore_ascii_case("ChanServ")
                    {
                        continue;
                    }

                    if !self.is_user_allowed(sender_nick) {
                        continue;
                    }

                    // Determine reply target: if sent to a channel, reply to channel;
                    // if DM (target == our nick), reply to sender
                    let is_channel = target.starts_with('#') || target.starts_with('&');
                    let reply_target = if is_channel {
                        target.to_string()
                    } else {
                        sender_nick.to_string()
                    };
                    let content = if is_channel {
                        format!("{IRC_STYLE_PREFIX}<{sender_nick}> {text}")
                    } else {
                        format!("{IRC_STYLE_PREFIX}{text}")
                    };

                    let seq = MSG_SEQ.fetch_add(1, Ordering::Relaxed);
                    let channel_msg = ChannelMessage {
                        id: format!("irc_{}_{seq}", chrono::Utc::now().timestamp_millis()),
                        sender: sender_nick.to_string(),
                        reply_target,
                        content,
                        channel: "irc".to_string(),
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        thread_ts: None,
                    };

                    if tx.send(channel_msg).await.is_err() {
                        return Ok(());
                    }
                }

                // ERR_PASSWDMISMATCH (464) or other fatal errors
                "464" => {
                    anyhow::bail!("IRC password mismatch");
                }

                _ => {}
            }
        }
    }

    async fn health_check(&self) -> bool {
        // Lightweight connectivity check: TLS connect + QUIT
        match self.connect().await {
            Ok(tls) => {
                let (_, mut writer) = tokio::io::split(tls);
                let _ = Self::send_raw(&mut writer, "QUIT :health check").await;
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(test)]
#[path = "irc_tests.rs"]
mod tests;
