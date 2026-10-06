//! Telling a party that the other one did something (DESIGN.md §12).
//!
//! A change to an exchange queues its messages in the outbox, in the same
//! transaction as the events that caused them, so a message exists exactly
//! when its event does. The worker delivers them: by email ([`outbox`],
//! [`smtp`]) and, to those with the app, by push ([`push`], [`expo`]).
//! Which change calls for which message is a rule, and lives in
//! `domain::notification`. One-time codes go by email, or by text message
//! to a phone number ([`sms`]); a party who turned on text updates for an
//! agreement is also texted about its status changes ([`sms_updates`]).

use crate::auth::SendFuture;

pub mod expo;
mod html;
pub mod outbox;
pub mod push;
pub mod sms;
pub mod sms_updates;
pub mod smtp;
pub mod wording;

/// One email, ready to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    /// Plain text.
    pub body: String,
    /// The same message as HTML, sent beside the text as
    /// `multipart/alternative`. Without it the message is the text alone.
    pub html: Option<String>,
    /// The queued message this is. Delivery is at least once: a worker that
    /// dies after sending and before recording it will send again, so a
    /// provider that can drop repeats should be handed this to do it with.
    pub reference: i64,
}

/// Delivers an email.
pub trait EmailSender: Send + Sync {
    fn send<'a>(&'a self, email: &'a Email) -> SendFuture<'a>;
}

/// Development delivery: writes the message to the worker's log, which is
/// where you read it. Never configured in production, where it would mean
/// nobody is ever told anything.
pub struct LogEmailSender;

impl EmailSender for LogEmailSender {
    fn send<'a>(&'a self, email: &'a Email) -> SendFuture<'a> {
        Box::pin(async move {
            // The text is what you read here. The HTML says the same, and a
            // whole document would bury it, so only its size is noted; the
            // preview example writes it out to open in a browser (README).
            tracing::info!(
                to = email.to,
                subject = email.subject,
                body = email.body,
                html_bytes = email.html.as_ref().map_or(0, String::len),
                reference = email.reference,
                "notification email (development delivery)"
            );
            Ok(())
        })
    }
}
