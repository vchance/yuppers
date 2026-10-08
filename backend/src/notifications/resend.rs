//! Delivery by Resend's HTTP API (`CODE_DELIVERY=resend`,
//! `NOTIFICATION_DELIVERY=resend`): the other email adapter at the edge,
//! beside SMTP (`super::smtp`), which stays an option and can point at
//! Resend's own SMTP server as well as anyone's.
//!
//! One call, `POST https://api.resend.com/emails`, with
//! `Authorization: Bearer <RESEND_API_KEY>`, a `User-Agent` (Resend refuses
//! a request without one with `403`) and a JSON body: `from`, `to` (one
//! address), `subject`, `text` and, as SMTP sends it beside the text,
//! `html`. Resend answers `200` with the message's `id`. See
//! <https://resend.com/docs/api-reference/emails/send-email>.
//!
//! **Repeats.** A notification is queued once and delivered at least once:
//! a worker that dies after Resend took a message, or a send that timed out
//! after it did, tries again. Each try carries the same `Idempotency-Key`,
//! made from the outbox row's own random key (`outbox.delivery_key`, never
//! its ID, which starts again from 1 after a reset or a restore, or in
//! another environment sending through the same Resend team), and for 24
//! hours Resend answers a repeat with its first answer instead of sending
//! again
//! (<https://resend.com/docs/dashboard/emails/idempotency-keys>). A
//! one-time code has no outbox row and no key: asking again is asking for
//! another code.
//!
//! **Refusals**, by status, as the outbox retries them
//! (<https://resend.com/docs/api-reference/errors>):
//!
//! | Answer | Means | Here |
//! |---|---|---|
//! | `400`, `422` | the request is wrong: an address Resend will not take, a missing field | [`Undeliverable`]: given up on at once |
//! | `409 invalid_idempotent_request` | this outbox row was already sent with other content (its language changed between tries) | [`KeyConflict`]: one copy is enough; given up on and logged as its own case |
//! | `401`, `403` | the key is missing, wrong, revoked or restricted, the From domain is not verified, or the account is over its quota (`email_above_quota`) | [`Outage`]: a configuration error, logged as one; the try is not counted, and the message waits up to an hour between tries for a day |
//! | `429 daily_quota_exceeded`, `429 monthly_quota_exceeded` | the plan's quota is spent | [`Outage`], as above |
//! | `409` (other), `429` (other), `5xx`, anything else | busy, over the rate (10 a second per team), or down | retried |
//! | no answer in time, no connection | | retried |
//!
//! What this file never does: write the key, the recipient, the subject or
//! the body to a log or an error. A refusal is described by its HTTP status
//! and Resend's error name (`validation_error`), never its message, which
//! can quote the address.

use std::time::Duration;

use anyhow::Context;
use hyper::http::{HeaderName, HeaderValue, StatusCode, header};
use lettre::message::Mailbox;
use serde_json::json;

use super::smtp::Secret;
use super::wording::Wording;
use super::{Email, EmailSender, KeyConflict, Outage, Undeliverable};
use crate::auth::{CodeMessage, CodeSender, SendFuture, SignInChannel};
use crate::build_info::BuildInfo;
use crate::domain::identity::Identifier;
use crate::outbound::{Answer, Client};

/// Where Resend's API is.
pub const RESEND_ORIGIN: &str = "https://api.resend.com";

/// What the sender needs.
#[derive(Clone, Debug)]
pub struct ResendSettings {
    /// `RESEND_API_KEY`.
    pub api_key: Secret,
    /// The sender, with an optional name: `Yuppers <no-reply@yuppers.app>`,
    /// on a domain verified in Resend.
    pub from: String,
    /// How long one request may take, from connecting to the last byte of
    /// the answer.
    pub timeout: Duration,
}

/// Sends through Resend's API.
pub struct ResendSender {
    client: Client,
    url: String,
    api_key: Secret,
    from: Mailbox,
    wording: Wording,
}

impl ResendSender {
    /// A sender against the API at `origin`: [`RESEND_ORIGIN`], or a
    /// stand-in in a test.
    pub fn new(origin: &str, settings: ResendSettings, wording: Wording) -> anyhow::Result<Self> {
        let from: Mailbox = settings.from.parse().with_context(|| {
            format!(
                "EMAIL_FROM (or SMTP_FROM) {:?} is not a mailbox",
                settings.from
            )
        })?;
        // Checked now, so a key that cannot be a header stops the start
        // instead of failing every send. The error does not quote it.
        if HeaderValue::from_str(&format!("Bearer {}", settings.api_key.expose())).is_err() {
            anyhow::bail!("RESEND_API_KEY holds characters a header cannot carry");
        }
        Ok(Self {
            client: Client::new(settings.timeout),
            url: format!("{}/emails", origin.trim_end_matches('/')),
            api_key: settings.api_key,
            from,
            wording,
        })
    }

    /// The key a queued message is sent under, the same on every try:
    /// its outbox row's own random key.
    pub fn idempotency_key(key: uuid::Uuid) -> String {
        format!("yuppers-outbox/{key}")
    }

    /// The request's body.
    pub fn body(from: &Mailbox, to: &str, subject: &str, text: &str, html: Option<&str>) -> String {
        let mut body = json!({
            "from": from.to_string(),
            "to": [to],
            "subject": subject,
            "text": text,
        });
        if let Some(html) = html {
            body["html"] = json!(html);
        }
        body.to_string()
    }

    fn headers(&self, key: Option<&str>) -> Vec<(HeaderName, HeaderValue)> {
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", self.api_key.expose()))
            .expect("checked in ResendSender::new");
        authorization.set_sensitive(true);
        let agent = format!("yuppers-backend/{}", BuildInfo::current().version);
        let mut headers = vec![
            (header::AUTHORIZATION, authorization),
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                header::USER_AGENT,
                HeaderValue::from_str(&agent).unwrap_or(HeaderValue::from_static("yuppers")),
            ),
        ];
        if let Some(key) = key.and_then(|key| HeaderValue::from_str(key).ok()) {
            headers.push((HeaderName::from_static("idempotency-key"), key));
        }
        headers
    }

    async fn deliver(
        &self,
        to: &str,
        subject: &str,
        text: &str,
        html: Option<&str>,
        key: Option<String>,
    ) -> anyhow::Result<()> {
        let body = Self::body(&self.from, to, subject, text, html);
        let answer = self
            .client
            .post(&self.url, &self.headers(key.as_deref()), body.into_bytes())
            .await
            .map_err(|error| anyhow::anyhow!("Resend: {error}"))?;
        accepted(&answer)?;
        // Resend's own ID for the message, to find it in Resend's records.
        tracing::info!(id = %message_id(&answer.body), "email handed to Resend");
        Ok(())
    }
}

/// Resend's ID from an answer, if it looks like one, or `unknown`.
fn message_id(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|body| body["id"].as_str().map(str::to_owned))
        .filter(|id| is_token(id))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Resend's error name (`validation_error`) from a refusal, if it has one
/// that looks like a name. Its message is never read: it can quote the
/// recipient.
fn error_name(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|body| body["name"].as_str().map(str::to_owned))
        .filter(|name| is_token(name))
}

/// Short, and letters, digits, `_` and `-` only: an ID or a name, never
/// anything quoted from a request.
fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Whether Resend took the message, and if not, how the outbox should treat
/// it (see the table at the top of this file).
pub fn accepted(answer: &Answer) -> anyhow::Result<()> {
    let status = answer.status;
    if status.is_success() {
        return Ok(());
    }
    let name = error_name(&answer.body);
    let described = match &name {
        Some(name) => format!("HTTP {}, {name}", status.as_u16()),
        None => format!("HTTP {}", status.as_u16()),
    };
    match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            Err(Undeliverable(format!("Resend refused the message ({described})")).into())
        }
        StatusCode::CONFLICT if name.as_deref() == Some("invalid_idempotent_request") => {
            Err(KeyConflict(format!(
                "Resend already took this message, with other content, under the same key \
                 ({described})"
            ))
            .into())
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            // Nothing will go until someone changes something at Resend or
            // in the settings: said loudly, for every try.
            tracing::error!(
                status = status.as_u16(),
                name = name.as_deref().unwrap_or("none"),
                "Resend refused the API key or the sender: check RESEND_API_KEY, its sending \
                 access, that EMAIL_FROM's domain is verified in Resend, and the account's quota"
            );
            Err(Outage(format!(
                "Resend refused the API key or the sender ({described})"
            ))
            .into())
        }
        StatusCode::TOO_MANY_REQUESTS
            if matches!(
                name.as_deref(),
                Some("daily_quota_exceeded" | "monthly_quota_exceeded")
            ) =>
        {
            tracing::error!(
                status = status.as_u16(),
                name = name.as_deref().unwrap_or("none"),
                "Resend's sending quota is spent: nothing goes until it renews or the plan changes"
            );
            Err(Outage(format!("Resend's sending quota is spent ({described})")).into())
        }
        _ => anyhow::bail!("Resend did not take the message for now ({described})"),
    }
}

impl EmailSender for ResendSender {
    fn send<'a>(&'a self, email: &'a Email) -> SendFuture<'a> {
        Box::pin(async move {
            self.deliver(
                &email.to,
                &email.subject,
                &email.body,
                email.html.as_deref(),
                Some(Self::idempotency_key(email.key)),
            )
            .await
        })
    }
}

impl CodeSender for ResendSender {
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
    use axum::body::Bytes;

    use super::*;

    fn answer(status: u16, body: &str) -> Answer {
        Answer {
            status: StatusCode::from_u16(status).unwrap(),
            body: Bytes::from(body.to_owned()),
        }
    }

    fn settings(from: &str, key: &str) -> ResendSettings {
        ResendSettings {
            api_key: Secret::new(key.to_owned()),
            from: from.to_owned(),
            timeout: Duration::from_secs(1),
        }
    }

    #[test]
    fn the_body_is_the_one_resend_takes() {
        let from: Mailbox = "Yuppers <no-reply@example.test>".parse().unwrap();
        let body: serde_json::Value = serde_json::from_str(&ResendSender::body(
            &from,
            "ana@example.test",
            "Hola",
            "Texto",
            Some("<p>Texto</p>"),
        ))
        .unwrap();
        assert_eq!(
            body,
            json!({
                "from": "Yuppers <no-reply@example.test>",
                "to": ["ana@example.test"],
                "subject": "Hola",
                "text": "Texto",
                "html": "<p>Texto</p>",
            })
        );
        let plain: serde_json::Value =
            serde_json::from_str(&ResendSender::body(&from, "a@b.test", "s", "t", None)).unwrap();
        assert!(plain.get("html").is_none());
    }

    #[test]
    fn the_idempotency_key_is_the_outbox_rows_own_and_within_resends_limit() {
        let row = uuid::Uuid::new_v4();
        let key = ResendSender::idempotency_key(row);
        assert_eq!(key, format!("yuppers-outbox/{row}"));
        assert!((1..=256).contains(&key.len()));
        assert_eq!(key, ResendSender::idempotency_key(row));
        assert_ne!(key, ResendSender::idempotency_key(uuid::Uuid::new_v4()));
    }

    #[test]
    fn refusals_are_classified_and_name_the_status_and_error_name_only() {
        assert!(accepted(&answer(200, r#"{"id":"x"}"#)).is_ok());
        let quoting = r#"{"statusCode":422,"name":"validation_error","message":"Invalid `to` field: ana@example.test"}"#;
        // What each is: given up on at once (U), a key conflict (K), an
        // outage (O), or retried (R).
        for (status, body, kind) in [
            (422, quoting, 'U'),
            (400, r#"{"name":"validation_error","message":"m"}"#, 'U'),
            (409, r#"{"name":"invalid_idempotent_request"}"#, 'K'),
            (409, r#"{"name":"concurrent_idempotent_requests"}"#, 'R'),
            (401, r#"{"name":"missing_api_key"}"#, 'O'),
            (403, r#"{"name":"validation_error"}"#, 'O'),
            (403, r#"{"name":"email_above_quota"}"#, 'O'),
            (429, r#"{"name":"daily_quota_exceeded"}"#, 'O'),
            (429, r#"{"name":"monthly_quota_exceeded"}"#, 'O'),
            (429, r#"{"name":"rate_limit_exceeded"}"#, 'R'),
            (429, "", 'R'),
            (500, "<html>oops</html>", 'R'),
            (503, "", 'R'),
        ] {
            let error = accepted(&answer(status, body)).unwrap_err();
            let found = if error.is::<Undeliverable>() {
                'U'
            } else if error.is::<KeyConflict>() {
                'K'
            } else if error.is::<Outage>() {
                'O'
            } else {
                'R'
            };
            assert_eq!(found, kind, "{status} {body}");
            let text = format!("{error:#}");
            assert!(text.contains(&format!("HTTP {status}")), "{text}");
            assert!(!text.contains("ana@"), "{text}");
            assert!(!text.contains("oops"), "{text}");
        }
        let error = accepted(&answer(422, quoting)).unwrap_err().to_string();
        assert_eq!(
            error,
            "Resend refused the message (HTTP 422, validation_error)"
        );
        // A name that is not a name is left out.
        let odd = r#"{"name":"ana@example.test"}"#;
        assert_eq!(
            accepted(&answer(422, odd)).unwrap_err().to_string(),
            "Resend refused the message (HTTP 422)"
        );
    }

    #[test]
    fn the_message_id_is_taken_only_if_it_looks_like_one() {
        assert_eq!(
            message_id(br#"{"id":"49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"}"#),
            "49a3999c-0ce1-4ea6-ab68-afcd6dc2e794"
        );
        assert_eq!(message_id(br#"{"id":"a b@c"}"#), "unknown");
        assert_eq!(message_id(b"not json"), "unknown");
    }

    #[test]
    fn the_from_address_must_be_a_mailbox_and_the_key_is_never_shown() {
        let wording = || Wording::embedded().unwrap();
        assert!(
            ResendSender::new(RESEND_ORIGIN, settings("not an address", "re_k"), wording())
                .is_err()
        );
        let sender = ResendSender::new(
            RESEND_ORIGIN,
            settings("Yuppers <no-reply@example.test>", "re_k"),
            wording(),
        )
        .unwrap();
        assert_eq!(
            CodeSender::email_sender(&sender).as_deref(),
            Some("no-reply@example.test")
        );
        assert_eq!(sender.url, "https://api.resend.com/emails");
        let error = ResendSender::new(
            RESEND_ORIGIN,
            settings("no-reply@example.test", "re_secret\nvalue"),
            wording(),
        )
        .err()
        .unwrap();
        assert!(!format!("{error:#}").contains("re_secret"), "{error:#}");
        let shown = format!("{:?}", settings("a@b.test", "re_secret_key"));
        assert!(!shown.contains("re_secret_key"), "{shown}");
    }
}
