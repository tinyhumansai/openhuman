//! Live IMAP/SMTP connection for operator inbox.
//!
//! Provides async email fetching via IMAP and sending via SMTP.
//! Uses `async-imap` + `tokio-rustls` for IMAP (matching `email_channel` pattern)
//! and `lettre` for SMTP.
//!
//! ## Log prefix
//!
//! `[operator-inbox-conn]`

use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tracing::{debug, info};

use super::imap_client::{FetchedEmail, ImapConfig, SmtpConfig};

const LOG_PREFIX: &str = "[operator-inbox-conn]";

/// Result of an IMAP fetch operation.
#[derive(Debug)]
pub struct FetchResult {
    pub emails: Vec<FetchedEmail>,
    pub new_count: usize,
}

/// Fetch new (UNSEEN) emails from IMAP server.
///
/// Connects via TLS, authenticates, selects mailbox, searches UNSEEN,
/// fetches and parses messages. Matches the pattern in `email_channel.rs`.
pub async fn fetch_new_emails(config: &ImapConfig) -> Result<FetchResult, String> {
    use super::imap_client::validate_imap_config;
    validate_imap_config(config)?;

    info!(
        "{LOG_PREFIX} connecting to {}:{} user={}",
        config.host, config.port, config.username
    );

    // Connect TCP.
    let addr = format!("{}:{}", config.host, config.port);
    let tcp = TcpStream::connect(&addr)
        .await
        .map_err(|e| format!("{LOG_PREFIX} TCP connect to {addr} failed: {e}"))?;

    // TLS via rustls (same pattern as email_channel).
    let certs = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.into(),
    };
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(certs)
        .with_no_client_auth();
    let connector: TlsConnector = Arc::new(tls_config).into();
    let sni: rustls_pki_types::ServerName = config
        .host
        .clone()
        .try_into()
        .map_err(|e| format!("{LOG_PREFIX} invalid hostname for SNI: {e}"))?;
    let stream = connector
        .connect(sni, tcp)
        .await
        .map_err(|e| format!("{LOG_PREFIX} TLS handshake failed: {e}"))?;

    // IMAP client.
    let client = async_imap::Client::new(stream);
    let mut session = client
        .login(&config.username, &config.password)
        .await
        .map_err(|(e, _)| format!("{LOG_PREFIX} IMAP login failed: {e}"))?;

    // Select mailbox.
    session
        .select(&config.mailbox)
        .await
        .map_err(|e| format!("{LOG_PREFIX} IMAP select '{}' failed: {e}", config.mailbox))?;

    debug!("{LOG_PREFIX} selected mailbox={}", config.mailbox);

    // Search UNSEEN.
    let uids = session
        .uid_search("UNSEEN")
        .await
        .map_err(|e| format!("{LOG_PREFIX} IMAP search UNSEEN failed: {e}"))?;

    if uids.is_empty() {
        info!("{LOG_PREFIX} no new messages");
        session.logout().await.ok();
        return Ok(FetchResult {
            emails: vec![],
            new_count: 0,
        });
    }

    info!("{LOG_PREFIX} found {} unseen messages", uids.len());

    // Fetch RFC822 bodies.
    let uid_set: String = uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let messages = session
        .uid_fetch(&uid_set, "RFC822")
        .await
        .map_err(|e| format!("{LOG_PREFIX} IMAP fetch failed: {e}"))?;

    // Parse messages using mail-parser.
    let mut emails = Vec::new();
    {
        use futures::StreamExt;
        let mut stream = messages;
        while let Some(msg_result) = stream.next().await {
            let msg = msg_result.map_err(|e| format!("{LOG_PREFIX} fetch stream error: {e}"))?;
            if let Some(body) = msg.body() {
                if let Some(parsed) = mail_parser::MessageParser::default().parse(body) {
                    let from = parsed
                        .from()
                        .and_then(|a| a.first())
                        .and_then(|a| a.address())
                        .map(|s| s.to_string())
                        .unwrap_or_default();

                    let to: Vec<String> = parsed
                        .to()
                        .map(|addrs| {
                            addrs
                                .iter()
                                .filter_map(|a| a.address().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    let subject = parsed.subject().unwrap_or("").to_string();
                    let message_id = parsed.message_id().map(String::from);
                    let in_reply_to = parsed.in_reply_to().as_text().map(String::from);
                    let references: Vec<String> = Vec::new(); // References header parsing deferred
                    let date = parsed.date().map(|d| d.to_rfc3339());
                    let body_text = parsed.body_text(0).unwrap_or_default().to_string();
                    let body_html = parsed.body_html(0).map(|h| h.to_string());

                    emails.push(FetchedEmail {
                        uid: msg.uid.unwrap_or(0),
                        message_id,
                        in_reply_to,
                        references,
                        from,
                        to,
                        subject,
                        date,
                        body_text,
                        body_html,
                        attachments: vec![],
                        flags: vec![],
                    });
                }
            }
        }
    } // stream dropped here, releasing borrow on session

    let new_count = emails.len();
    session.logout().await.ok();
    info!("{LOG_PREFIX} fetched {new_count} emails");

    Ok(FetchResult { emails, new_count })
}

/// Send an email reply via SMTP using lettre.
pub async fn send_reply(
    config: &SmtpConfig,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<(), String> {
    use super::imap_client::validate_smtp_config;
    validate_smtp_config(config)?;

    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

    info!(
        "{LOG_PREFIX} sending reply to_domain={} subject_len={}",
        to.split('@').nth(1).unwrap_or("unknown"),
        subject.len()
    );

    let email = Message::builder()
        .from(
            format!("{} <{}>", config.from_name, config.from_address)
                .parse()
                .map_err(|e| format!("{LOG_PREFIX} invalid from: {e}"))?,
        )
        .to(to
            .parse()
            .map_err(|e| format!("{LOG_PREFIX} invalid to: {e}"))?)
        .subject(subject)
        .body(body.to_string())
        .map_err(|e| format!("{LOG_PREFIX} email build: {e}"))?;

    let creds = Credentials::new(config.username.clone(), config.password.clone());

    // Honor the configured port and TLS mode: STARTTLS for submission ports
    // (e.g. 587), implicit TLS for 465, plaintext only when TLS is disabled.
    // `relay()` alone forces implicit TLS on 465 and silently ignores
    // `config.port`, which breaks the common 587/STARTTLS default.
    let builder = if config.use_tls {
        if config.port == 465 {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)
        }
        .map_err(|e| format!("{LOG_PREFIX} SMTP relay: {e}"))?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host)
    };
    let mailer = builder.port(config.port).credentials(creds).build();

    mailer
        .send(email)
        .await
        .map_err(|e| format!("{LOG_PREFIX} SMTP send: {e}"))?;

    info!(
        "{LOG_PREFIX} reply sent to_domain={}",
        to.split('@').nth(1).unwrap_or("unknown")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_result_default() {
        let r = FetchResult {
            emails: vec![],
            new_count: 0,
        };
        assert_eq!(r.new_count, 0);
        assert!(r.emails.is_empty());
    }
}
