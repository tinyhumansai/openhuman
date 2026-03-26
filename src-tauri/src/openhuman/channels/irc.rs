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
mod tests {
    use super::*;

    // ── IRC message parsing ──────────────────────────────────

    #[test]
    fn parse_privmsg_with_prefix() {
        let msg = IrcMessage::parse(":nick!user@host PRIVMSG #channel :Hello world").unwrap();
        assert_eq!(msg.prefix.as_deref(), Some("nick!user@host"));
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["#channel", "Hello world"]);
    }

    #[test]
    fn parse_privmsg_dm() {
        let msg = IrcMessage::parse(":alice!a@host PRIVMSG botname :hi there").unwrap();
        assert_eq!(msg.command, "PRIVMSG");
        assert_eq!(msg.params, vec!["botname", "hi there"]);
        assert_eq!(msg.nick(), Some("alice"));
    }

    #[test]
    fn parse_ping() {
        let msg = IrcMessage::parse("PING :server.example.com").unwrap();
        assert!(msg.prefix.is_none());
        assert_eq!(msg.command, "PING");
        assert_eq!(msg.params, vec!["server.example.com"]);
    }

    #[test]
    fn parse_numeric_reply() {
        let msg = IrcMessage::parse(":server 001 botname :Welcome to the IRC network").unwrap();
        assert_eq!(msg.prefix.as_deref(), Some("server"));
        assert_eq!(msg.command, "001");
        assert_eq!(msg.params, vec!["botname", "Welcome to the IRC network"]);
    }

    #[test]
    fn parse_no_trailing() {
        let msg = IrcMessage::parse(":server 433 * botname").unwrap();
        assert_eq!(msg.command, "433");
        assert_eq!(msg.params, vec!["*", "botname"]);
    }

    #[test]
    fn parse_cap_ack() {
        let msg = IrcMessage::parse(":server CAP * ACK :sasl").unwrap();
        assert_eq!(msg.command, "CAP");
        assert_eq!(msg.params, vec!["*", "ACK", "sasl"]);
    }

    #[test]
    fn parse_empty_line_returns_none() {
        assert!(IrcMessage::parse("").is_none());
        assert!(IrcMessage::parse("\r\n").is_none());
    }

    #[test]
    fn parse_strips_crlf() {
        let msg = IrcMessage::parse("PING :test\r\n").unwrap();
        assert_eq!(msg.params, vec!["test"]);
    }

    #[test]
    fn parse_command_uppercase() {
        let msg = IrcMessage::parse("ping :test").unwrap();
        assert_eq!(msg.command, "PING");
    }

    #[test]
    fn nick_extraction_full_prefix() {
        let msg = IrcMessage::parse(":nick!user@host PRIVMSG #ch :msg").unwrap();
        assert_eq!(msg.nick(), Some("nick"));
    }

    #[test]
    fn nick_extraction_nick_only() {
        let msg = IrcMessage::parse(":server 001 bot :Welcome").unwrap();
        assert_eq!(msg.nick(), Some("server"));
    }

    #[test]
    fn nick_extraction_no_prefix() {
        let msg = IrcMessage::parse("PING :token").unwrap();
        assert_eq!(msg.nick(), None);
    }

    #[test]
    fn parse_authenticate_plus() {
        let msg = IrcMessage::parse("AUTHENTICATE +").unwrap();
        assert_eq!(msg.command, "AUTHENTICATE");
        assert_eq!(msg.params, vec!["+"]);
    }

    // ── SASL PLAIN encoding ─────────────────────────────────

    #[test]
    fn sasl_plain_encode() {
        let encoded = encode_sasl_plain("jilles", "sesame");
        // \0jilles\0sesame → base64
        assert_eq!(encoded, "AGppbGxlcwBzZXNhbWU=");
    }

    #[test]
    fn sasl_plain_empty_password() {
        let encoded = encode_sasl_plain("nick", "");
        // \0nick\0 → base64
        assert_eq!(encoded, "AG5pY2sA");
    }

    // ── Message splitting ───────────────────────────────────

    #[test]
    fn split_short_message() {
        let chunks = split_message("hello", 400);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn split_long_message() {
        let msg = "a".repeat(800);
        let chunks = split_message(&msg, 400);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 400);
        assert_eq!(chunks[1].len(), 400);
    }

    #[test]
    fn split_exact_boundary() {
        let msg = "a".repeat(400);
        let chunks = split_message(&msg, 400);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn split_unicode_safe() {
        // 'é' is 2 bytes in UTF-8; splitting at byte 3 would split mid-char
        let msg = "ééé"; // 6 bytes
        let chunks = split_message(msg, 3);
        // Should split at char boundary (2 bytes), not mid-char
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "é");
        assert_eq!(chunks[1], "é");
        assert_eq!(chunks[2], "é");
    }

    #[test]
    fn split_empty_message() {
        let chunks = split_message("", 400);
        assert_eq!(chunks, vec![""]);
    }

    #[test]
    fn split_newlines_into_separate_lines() {
        let chunks = split_message("line one\nline two\nline three", 400);
        assert_eq!(chunks, vec!["line one", "line two", "line three"]);
    }

    #[test]
    fn split_crlf_newlines() {
        let chunks = split_message("hello\r\nworld", 400);
        assert_eq!(chunks, vec!["hello", "world"]);
    }

    #[test]
    fn split_skips_empty_lines() {
        let chunks = split_message("hello\n\n\nworld", 400);
        assert_eq!(chunks, vec!["hello", "world"]);
    }

    #[test]
    fn split_trailing_newline() {
        let chunks = split_message("hello\n", 400);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn split_multiline_with_long_line() {
        let long = "a".repeat(800);
        let msg = format!("short\n{long}\nend");
        let chunks = split_message(&msg, 400);
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0], "short");
        assert_eq!(chunks[1].len(), 400);
        assert_eq!(chunks[2].len(), 400);
        assert_eq!(chunks[3], "end");
    }

    #[test]
    fn split_only_newlines() {
        let chunks = split_message("\n\n\n", 400);
        assert_eq!(chunks, vec![""]);
    }

    // ── Allowlist ───────────────────────────────────────────

    #[test]
    fn wildcard_allows_anyone() {
        let ch = make_channel();
        // Default make_channel has wildcard
        assert!(ch.is_user_allowed("anyone"));
        assert!(ch.is_user_allowed("stranger"));
    }

    #[test]
    fn specific_user_allowed() {
        let ch = IrcChannel::new(IrcChannelConfig {
            server: "irc.test".into(),
            port: 6697,
            nickname: "bot".into(),
            username: None,
            channels: vec![],
            allowed_users: vec!["alice".into(), "bob".into()],
            server_password: None,
            nickserv_password: None,
            sasl_password: None,
            verify_tls: true,
        });
        assert!(ch.is_user_allowed("alice"));
        assert!(ch.is_user_allowed("bob"));
        assert!(!ch.is_user_allowed("eve"));
    }

    #[test]
    fn allowlist_case_insensitive() {
        let ch = IrcChannel::new(IrcChannelConfig {
            server: "irc.test".into(),
            port: 6697,
            nickname: "bot".into(),
            username: None,
            channels: vec![],
            allowed_users: vec!["Alice".into()],
            server_password: None,
            nickserv_password: None,
            sasl_password: None,
            verify_tls: true,
        });
        assert!(ch.is_user_allowed("alice"));
        assert!(ch.is_user_allowed("ALICE"));
        assert!(ch.is_user_allowed("Alice"));
    }

    #[test]
    fn empty_allowlist_denies_all() {
        let ch = IrcChannel::new(IrcChannelConfig {
            server: "irc.test".into(),
            port: 6697,
            nickname: "bot".into(),
            username: None,
            channels: vec![],
            allowed_users: vec![],
            server_password: None,
            nickserv_password: None,
            sasl_password: None,
            verify_tls: true,
        });
        assert!(!ch.is_user_allowed("anyone"));
    }

    // ── Constructor ─────────────────────────────────────────

    #[test]
    fn new_defaults_username_to_nickname() {
        let ch = IrcChannel::new(IrcChannelConfig {
            server: "irc.test".into(),
            port: 6697,
            nickname: "mybot".into(),
            username: None,
            channels: vec![],
            allowed_users: vec![],
            server_password: None,
            nickserv_password: None,
            sasl_password: None,
            verify_tls: true,
        });
        assert_eq!(ch.username, "mybot");
    }

    #[test]
    fn new_uses_explicit_username() {
        let ch = IrcChannel::new(IrcChannelConfig {
            server: "irc.test".into(),
            port: 6697,
            nickname: "mybot".into(),
            username: Some("customuser".into()),
            channels: vec![],
            allowed_users: vec![],
            server_password: None,
            nickserv_password: None,
            sasl_password: None,
            verify_tls: true,
        });
        assert_eq!(ch.username, "customuser");
        assert_eq!(ch.nickname, "mybot");
    }

    #[test]
    fn name_returns_irc() {
        let ch = make_channel();
        assert_eq!(ch.name(), "irc");
    }

    #[test]
    fn new_stores_all_fields() {
        let ch = IrcChannel::new(IrcChannelConfig {
            server: "irc.example.com".into(),
            port: 6697,
            nickname: "zcbot".into(),
            username: Some("openhuman".into()),
            channels: vec!["#test".into()],
            allowed_users: vec!["alice".into()],
            server_password: Some("serverpass".into()),
            nickserv_password: Some("nspass".into()),
            sasl_password: Some("saslpass".into()),
            verify_tls: false,
        });
        assert_eq!(ch.server, "irc.example.com");
        assert_eq!(ch.port, 6697);
        assert_eq!(ch.nickname, "zcbot");
        assert_eq!(ch.username, "openhuman");
        assert_eq!(ch.channels, vec!["#test"]);
        assert_eq!(ch.allowed_users, vec!["alice"]);
        assert_eq!(ch.server_password.as_deref(), Some("serverpass"));
        assert_eq!(ch.nickserv_password.as_deref(), Some("nspass"));
        assert_eq!(ch.sasl_password.as_deref(), Some("saslpass"));
        assert!(!ch.verify_tls);
    }

    // ── Config serde ────────────────────────────────────────

    #[test]
    fn irc_config_serde_roundtrip() {
        use crate::openhuman::config::schema::IrcConfig;

        let config = IrcConfig {
            server: "irc.example.com".into(),
            port: 6697,
            nickname: "zcbot".into(),
            username: Some("openhuman".into()),
            channels: vec!["#test".into(), "#dev".into()],
            allowed_users: vec!["alice".into()],
            server_password: None,
            nickserv_password: Some("secret".into()),
            sasl_password: None,
            verify_tls: Some(true),
        };

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: IrcConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.server, "irc.example.com");
        assert_eq!(parsed.port, 6697);
        assert_eq!(parsed.nickname, "zcbot");
        assert_eq!(parsed.username.as_deref(), Some("openhuman"));
        assert_eq!(parsed.channels, vec!["#test", "#dev"]);
        assert_eq!(parsed.allowed_users, vec!["alice"]);
        assert!(parsed.server_password.is_none());
        assert_eq!(parsed.nickserv_password.as_deref(), Some("secret"));
        assert!(parsed.sasl_password.is_none());
        assert_eq!(parsed.verify_tls, Some(true));
    }

    #[test]
    fn irc_config_minimal_toml() {
        use crate::openhuman::config::schema::IrcConfig;

        let toml_str = r#"
server = "irc.example.com"
nickname = "bot"
"#;
        let parsed: IrcConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.server, "irc.example.com");
        assert_eq!(parsed.port, 6697); // default
        assert_eq!(parsed.nickname, "bot");
        assert!(parsed.username.is_none());
        assert!(parsed.channels.is_empty());
        assert!(parsed.allowed_users.is_empty());
        assert!(parsed.server_password.is_none());
        assert!(parsed.nickserv_password.is_none());
        assert!(parsed.sasl_password.is_none());
        assert!(parsed.verify_tls.is_none());
    }

    #[test]
    fn irc_config_default_port() {
        use crate::openhuman::config::schema::IrcConfig;

        let json = r#"{"server":"irc.test","nickname":"bot"}"#;
        let parsed: IrcConfig = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.port, 6697);
    }

    // ── Helpers ─────────────────────────────────────────────

    fn make_channel() -> IrcChannel {
        IrcChannel::new(IrcChannelConfig {
            server: "irc.example.com".into(),
            port: 6697,
            nickname: "zcbot".into(),
            username: None,
            channels: vec!["#openhuman".into()],
            allowed_users: vec!["*".into()],
            server_password: None,
            nickserv_password: None,
            sasl_password: None,
            verify_tls: true,
        })
    }
}
