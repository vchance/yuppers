//! Delivery by SMTP: the email adapter at the edge (DESIGN.md §12, §13).
//!
//! Nearly every email provider offers SMTP, so a deployment supplies a host
//! and credentials, not code. One sender serves both notifications
//! ([`EmailSender`]) and one-time codes ([`CodeSender`]). It sends codes to
//! email addresses only; a code for a phone number goes by Twilio Verify when
//! `SMS_CODE_DELIVERY` is on (`super::sms::CodeRouter`), and is refused here
//! when it is off.
//!
//! What this file never does: write the password anywhere, or put anything in
//! a message that the wording did not give it.

use std::fmt;
use std::time::Duration;

use anyhow::Context;
use lettre::message::header::ContentType;
use lettre::message::{Mailbox, Message, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{Address, AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use super::wording::Wording;
use super::{Email, EmailSender};
use crate::auth::{CodeMessage, CodeSender, SendFuture, SignInChannel};
use crate::domain::identity::Identifier;

/// How the connection to the server is protected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsMode {
    /// TLS from the first byte (SMTPS, usually port 465). The default.
    Tls,
    /// A plain connection that must upgrade with `STARTTLS` before anything
    /// else is said (usually port 587). A server that cannot is refused.
    StartTls,
    /// Plain text throughout. For a relay on the same host or a test server,
    /// never for a server reached over a network.
    None,
}

impl TlsMode {
    /// Reads the setting's value: `tls`, `starttls` or `none`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tls" => Some(Self::Tls),
            "starttls" => Some(Self::StartTls),
            "none" => Some(Self::None),
            _ => Option::None,
        }
    }

    /// The port the mode is usually served on.
    pub fn default_port(self) -> u16 {
        match self {
            Self::Tls => 465,
            Self::StartTls => 587,
            Self::None => 25,
        }
    }
}

/// A value that must not appear in logs or error messages. Its `Debug` form
/// says only that it exists.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The value itself, for the one place it is sent.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// Where and how to send.
#[derive(Clone, Debug)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    /// A username and password, if the server wants them.
    pub credentials: Option<(String, Secret)>,
    /// The sender address, with an optional name: `Yuppers <no-reply@example.com>`.
    pub from: String,
    /// How long one connection may be silent before the send fails.
    pub timeout: Duration,
}

/// Sends through one SMTP server.
pub struct SmtpSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    wording: Wording,
}

impl SmtpSender {
    pub fn new(settings: SmtpSettings, wording: Wording) -> anyhow::Result<Self> {
        let from: Mailbox = settings.from.parse().with_context(|| {
            format!(
                "EMAIL_FROM (or SMTP_FROM) {:?} is not a mailbox",
                settings.from
            )
        })?;
        let tls = match settings.tls {
            TlsMode::None => Tls::None,
            mode => {
                let parameters = TlsParameters::new(settings.host.clone())
                    .context("TLS cannot be set up for the SMTP server")?;
                match mode {
                    TlsMode::Tls => Tls::Wrapper(parameters),
                    _ => Tls::Required(parameters),
                }
            }
        };
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(settings.host)
            .port(settings.port)
            .tls(tls)
            .timeout(Some(settings.timeout));
        if let Some((username, Secret(password))) = settings.credentials {
            builder = builder.credentials(Credentials::new(username, password));
        }
        Ok(Self {
            transport: builder.build(),
            from,
            wording,
        })
    }

    /// Sends `body` as the plain text and, if there is one, `html` beside it
    /// as `multipart/alternative`: text first, so a client that shows
    /// either picks the HTML and one that cannot still has the whole text.
    async fn deliver(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        html: Option<&str>,
        id: Option<String>,
    ) -> anyhow::Result<()> {
        // No error here names the recipient or carries the server's text,
        // which usually quotes the address (`550 5.1.1 <ana@...> unknown`):
        // these errors are logged and stored in `outbox.last_error`. The
        // caller knows which message, or which request, it was.
        let to: Address = to
            .parse()
            .map_err(|_| anyhow::anyhow!("the recipient is not an address SMTP can deliver to"))?;
        let builder = Message::builder()
            .from(self.from.clone())
            .to(Mailbox::new(None, to))
            .subject(subject)
            .message_id(id);
        let message = match html {
            // Each part is UTF-8, said in its own Content-Type.
            Some(html) => builder.multipart(MultiPart::alternative_plain_html(
                body.to_owned(),
                html.to_owned(),
            )),
            None => builder
                .header(ContentType::TEXT_PLAIN)
                .body(body.to_owned()),
        }
        .map_err(|_| anyhow::anyhow!("the message could not be built"))?;
        let response = self
            .transport
            .send(message)
            .await
            .map_err(|error| anyhow::anyhow!(describe(&error)))?;
        // What the server said on taking the message, such as a provider's
        // own ID for it, so a delivery can be traced in the provider's
        // records. Any line that could carry an address is left out.
        let reply: Vec<&str> = response
            .message()
            .filter(|line| !line.contains('@'))
            .collect();
        tracing::info!(
            code = %response.code(),
            reply = %reply.join(" "),
            "email handed to the SMTP server"
        );
        Ok(())
    }
}

/// Why a send failed, without anything the server said beyond its reply
/// code, and without the underlying error's own text.
pub fn describe(error: &lettre::transport::smtp::Error) -> String {
    if let Some(code) = error.status() {
        let kind = if error.is_permanent() {
            "refused"
        } else {
            "turned away for now"
        };
        format!("the SMTP server {kind} the message (reply code {code})")
    } else if error.is_timeout() {
        "the SMTP server did not answer in time".to_owned()
    } else if error.is_tls() {
        "TLS with the SMTP server failed".to_owned()
    } else if error.is_response() {
        "the SMTP server's reply could not be understood".to_owned()
    } else if error.is_client() {
        // lettre's own fixed wording, such as "STARTTLS is not supported on
        // this server", which never quotes the server or the message.
        format!("the SMTP conversation failed on this side ({error})")
    } else {
        "the SMTP server could not be reached".to_owned()
    }
}

impl EmailSender for SmtpSender {
    fn send<'a>(&'a self, email: &'a Email) -> SendFuture<'a> {
        Box::pin(async move {
            // The same message sent again after a crash carries the same ID,
            // so a mailbox can tell it is a repeat.
            let id = format!("<outbox-{}@{}>", email.reference, self.from.email.domain());
            self.deliver(
                &email.to,
                &email.subject,
                &email.body,
                email.html.as_deref(),
                Some(id),
            )
            .await
        })
    }
}

impl CodeSender for SmtpSender {
    fn send<'a>(&'a self, message: CodeMessage<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            let Identifier::Email(to) = message.to else {
                // Refused, not logged: the code is still a code.
                anyhow::bail!("codes for phone numbers need SMS delivery, which is off");
            };
            let rendered = self
                .wording
                .code_email(message.language, message.purpose, message.code);
            self.deliver(
                to,
                &rendered.subject,
                &rendered.body,
                Some(&rendered.html),
                None,
            )
            .await
        })
    }

    /// Email only: a code for a phone number needs SMS delivery.
    fn delivers(&self, channel: SignInChannel) -> bool {
        channel == SignInChannel::Email
    }

    fn email_sender(&self) -> Option<String> {
        Some(self.from.email.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tls_mode_is_one_of_three_words() {
        assert_eq!(TlsMode::parse("tls"), Some(TlsMode::Tls));
        assert_eq!(TlsMode::parse("starttls"), Some(TlsMode::StartTls));
        assert_eq!(TlsMode::parse("none"), Some(TlsMode::None));
        for other in ["", "TLS", "ssl", "opportunistic"] {
            assert_eq!(TlsMode::parse(other), Option::None, "{other:?}");
        }
        assert_eq!(TlsMode::Tls.default_port(), 465);
        assert_eq!(TlsMode::StartTls.default_port(), 587);
    }

    #[test]
    fn the_password_is_not_in_the_settings_debug_output() {
        let settings = SmtpSettings {
            host: "smtp.example.test".to_owned(),
            port: 465,
            tls: TlsMode::Tls,
            credentials: Some(("user".to_owned(), Secret::new("hunter2".to_owned()))),
            from: "Yuppers <no-reply@example.test>".to_owned(),
            timeout: Duration::from_secs(30),
        };
        let shown = format!("{settings:?}");
        assert!(shown.contains("smtp.example.test") && shown.contains("user"));
        assert!(!shown.contains("hunter2"), "{shown}");
    }

    #[test]
    fn the_from_address_must_be_a_mailbox() {
        let settings = |from: &str| SmtpSettings {
            host: "localhost".to_owned(),
            port: 2525,
            tls: TlsMode::None,
            credentials: Option::None,
            from: from.to_owned(),
            timeout: Duration::from_secs(1),
        };
        let wording = || Wording::embedded().unwrap();
        assert!(SmtpSender::new(settings("not an address"), wording()).is_err());
        assert!(SmtpSender::new(settings("no-reply@example.test"), wording()).is_ok());
        assert!(SmtpSender::new(settings("Yuppers <no-reply@example.test>"), wording()).is_ok());
    }
}
