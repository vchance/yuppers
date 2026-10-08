//! Telling a party that the other one did something (DESIGN.md §12).
//!
//! A change to an exchange queues its messages in the outbox, in the same
//! transaction as the events that caused them, so a message exists exactly
//! when its event does. The worker delivers them: by email ([`outbox`],
//! [`smtp`] or [`resend`]) and, to those with the app, by push ([`push`], [`expo`]).
//! Which change calls for which message is a rule, and lives in
//! `domain::notification`. One-time codes go by email, or by text message
//! to a phone number, made, sent and checked by Twilio Verify ([`verify`],
//! routed by [`sms`]); a party who turned on text updates for an agreement
//! is also texted about its status changes ([`sms_updates`]).

use crate::auth::SendFuture;

pub mod expo;
mod html;
pub mod outbox;
pub mod push;
pub mod resend;
pub mod sms;
pub mod sms_updates;
pub mod smtp;
pub mod verify;
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
    /// The queued message this is (its outbox ID), for the log.
    pub reference: i64,
    /// The queued message's own random key (`outbox.delivery_key`), the
    /// same on every try of it and never used for another. Delivery is at
    /// least once: a worker that dies after sending and before recording it
    /// will send again, so a provider that can drop repeats is handed this
    /// to do it with. Not the ID, which starts again from 1 in a database
    /// made afresh or restored, or in another environment.
    pub key: uuid::Uuid,
}

/// Delivers an email. An error is retried by the outbox, unless it is
/// [`Undeliverable`].
pub trait EmailSender: Send + Sync {
    fn send<'a>(&'a self, email: &'a Email) -> SendFuture<'a>;
}

/// A refusal that trying again cannot change, such as an address the
/// provider will not take: the outbox gives the message up at once instead
/// of retrying it. Its text is what `outbox.last_error` records, so it names
/// no address.
#[derive(Debug)]
pub struct Undeliverable(pub String);

impl std::fmt::Display for Undeliverable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Undeliverable {}

/// A refusal that says the provider will take nothing from this service
/// until someone changes something: a key or password it does not accept,
/// a sender domain it has not verified, a quota that is spent. Trying again
/// within minutes changes nothing, and counting each try would give every
/// message up within the hour such an outage lasts. The outbox does not
/// count the try: it waits longer each time, up to
/// [`outbox::DeliveryRules::retry_ceiling`], and gives the message up only
/// once it is older than [`outbox::DeliveryRules::outage_max_age`]. Its text
/// names no address.
#[derive(Debug)]
pub struct Outage(pub String);

impl std::fmt::Display for Outage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Outage {}

/// The provider already holds another message under this one's key
/// ([`Email::key`]): the same queued message, sent before with other
/// content, such as in a language the person changed between two tries.
/// One copy went out, so it is not sent again, and the message is given up
/// on and logged as its own case for someone to look at: with a random key
/// per message, it should not happen.
#[derive(Debug)]
pub struct KeyConflict(pub String);

impl std::fmt::Display for KeyConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeyConflict {}

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
                key = %email.key,
                "notification email (development delivery)"
            );
            Ok(())
        })
    }
}
