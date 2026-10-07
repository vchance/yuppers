//! Text messages (DESIGN.md §8, §12): the SMS adapter at the edge.
//!
//! The service's own texts are the agreement updates a person turns on,
//! "Yuppers.app agreement updates" (`super::sms_updates`, which the worker
//! sends through an [`SmsSender`]: Twilio's Messages API, from the number
//! registered for that program). One-time codes for phone numbers are not
//! among them: Twilio Verify makes, texts and checks those, from Twilio's own
//! senders and in its own template (`super::verify`). [`CodeRouter`] sends a
//! code for a phone number that way, or to the development log, and a code
//! for an email address as before, so the rest of the service still sees one
//! [`CodeSender`].
//!
//! Texts people send back to the updates' number reach the service through
//! Twilio's webhook, whose requests are signed ([`twilio_signature_valid`]).
//!
//! Each message costs money, so it is kept to one segment in every language
//! (`encoding` below, and the tests), and the service caps how many it sends
//! per hour, codes and updates together (`crate::auth`, `SMS_MAX_PER_HOUR`).
//!
//! What this file never does: write a phone number in full, or a code
//! outside the development delivery, to a log or an error.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use hyper::http::{HeaderName, HeaderValue, StatusCode, header};

use super::smtp::Secret;
use crate::auth::{CodeMessage, CodeSender, CodeVerifier, LogSender, SendFuture, SignInChannel};
use crate::domain::identity::Identifier;
use crate::outbound::Client;

/// One text message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sms<'a> {
    /// The number, in E.164 form (`+15551234567`).
    pub to: &'a str,
    pub text: &'a str,
}

/// Delivers a text message.
pub trait SmsSender: Send + Sync {
    fn send<'a>(&'a self, sms: Sms<'a>) -> SendFuture<'a>;
}

/// A phone number as a log may show it: the country code's first digit and
/// the last two, `+1••••••••67`.
pub fn masked(number: &str) -> String {
    match Identifier::parse(number) {
        Ok(phone @ Identifier::Phone(_)) => phone.masked(),
        _ => "[a phone number]".to_owned(),
    }
}

// ---- Length -----------------------------------------------------------------

/// How a text is sent, and how much of a segment it takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// The GSM 03.38 alphabet, seven bits a character: 160 to a segment.
    /// A character of its extension table takes two.
    Gsm7 { septets: usize },
    /// Anything outside it, such as `ó` or a curly apostrophe, sends the
    /// whole text as UCS-2: 70 UTF-16 units to a segment.
    Ucs2 { units: usize },
}

impl Encoding {
    /// Whether the text fits in a single segment, which is one message's
    /// price.
    pub fn fits_one_segment(self) -> bool {
        match self {
            Encoding::Gsm7 { septets } => septets <= 160,
            Encoding::Ucs2 { units } => units <= 70,
        }
    }
}

/// The GSM 03.38 basic character set.
const GSM7_BASIC: &str = "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?\
    ¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà";
/// Its extension table, each character sent as an escape and itself.
const GSM7_EXTENSION: &str = "^{}\\[~]|€\u{c}";

/// How `text` would be sent.
pub fn encoding(text: &str) -> Encoding {
    let mut septets = 0;
    for c in text.chars() {
        if GSM7_BASIC.contains(c) {
            septets += 1;
        } else if GSM7_EXTENSION.contains(c) {
            septets += 2;
        } else {
            return Encoding::Ucs2 {
                units: text.encode_utf16().count(),
            };
        }
    }
    Encoding::Gsm7 { septets }
}

// ---- Development delivery --------------------------------------------------

/// Development delivery: writes the message to the API's log, which is where
/// you read the code, with the number masked. Never configured in
/// production, where anyone who can read logs could sign in as anyone.
pub struct LogSmsSender;

impl SmsSender for LogSmsSender {
    fn send<'a>(&'a self, sms: Sms<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            let encoding = encoding(sms.text);
            tracing::info!(
                to = masked(sms.to),
                text = sms.text,
                encoding = ?encoding,
                "text message (development delivery)"
            );
            Ok(())
        })
    }
}

// ---- Twilio -----------------------------------------------------------------

/// Where Twilio's REST API is.
pub const TWILIO_ORIGIN: &str = "https://api.twilio.com";

/// How the sender proves it may use the account: HTTP Basic, with either
/// pair Twilio takes. The URL names the account (`AC…`) either way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TwilioCredential {
    /// The account's own auth token, with the account SID as the username.
    AuthToken(Secret),
    /// An API key (`SK…`) and its secret: the recommended way, since a key
    /// can be revoked without touching the account or its other keys.
    ApiKey { sid: String, secret: Secret },
}

/// The headers of a form POST to any of Twilio's REST APIs (Messages,
/// Verify): HTTP Basic with the [`TwilioCredential`], marked sensitive.
pub(crate) fn twilio_headers(
    account_sid: &str,
    credential: &TwilioCredential,
) -> Vec<(HeaderName, HeaderValue)> {
    let (username, password) = match credential {
        TwilioCredential::AuthToken(token) => (account_sid, token),
        TwilioCredential::ApiKey { sid, secret } => (sid.as_str(), secret),
    };
    let credentials = base64::engine::general_purpose::STANDARD
        .encode(format!("{username}:{}", password.expose()));
    let mut authorization = HeaderValue::from_str(&format!("Basic {credentials}"))
        .expect("base64 is a valid header value");
    authorization.set_sensitive(true);
    vec![
        (header::AUTHORIZATION, authorization),
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        ),
        (header::ACCEPT, HeaderValue::from_static("application/json")),
    ]
}

/// Twilio's error code in an answer's JSON body, the only part of a refusal
/// kept: its message often quotes the number.
pub(crate) fn twilio_error_code(body: &[u8]) -> Option<u64> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|body| body["code"].as_u64())
}

/// Sends through Twilio's Messages API: one POST per message, with a
/// [`TwilioCredential`] as HTTP Basic credentials. Other providers' APIs are
/// much the same shape and would be another [`SmsSender`].
pub struct TwilioSmsSender {
    client: Client,
    origin: String,
    account_sid: String,
    credential: TwilioCredential,
    /// A number of the account's in E.164 form, or the SID of a Messaging
    /// Service (`MG…`), which picks the number itself.
    from: String,
}

impl TwilioSmsSender {
    /// A sender to the API at `origin`: [`TWILIO_ORIGIN`], or a stand-in in a
    /// test. Each request must be answered within `timeout`.
    pub fn new(
        origin: &str,
        account_sid: String,
        credential: TwilioCredential,
        from: String,
        timeout: Duration,
    ) -> Self {
        Self {
            client: Client::new(timeout),
            origin: origin.trim_end_matches('/').to_owned(),
            account_sid,
            credential,
            from,
        }
    }

    fn url(&self) -> String {
        let sid: String = form_urlencoded::byte_serialize(self.account_sid.as_bytes()).collect();
        format!("{}/2010-04-01/Accounts/{sid}/Messages.json", self.origin)
    }

    fn headers(&self) -> Vec<(HeaderName, HeaderValue)> {
        twilio_headers(&self.account_sid, &self.credential)
    }

    /// The form Twilio takes: who to, from what, and the text.
    pub fn form(&self, sms: Sms<'_>) -> String {
        let mut form = form_urlencoded::Serializer::new(String::new());
        form.append_pair("To", sms.to);
        if self.from.starts_with("MG") {
            form.append_pair("MessagingServiceSid", &self.from);
        } else {
            form.append_pair("From", &self.from);
        }
        form.append_pair("Body", sms.text);
        form.finish()
    }
}

impl SmsSender for TwilioSmsSender {
    fn send<'a>(&'a self, sms: Sms<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            let answer = self
                .client
                .post(&self.url(), &self.headers(), self.form(sms).into_bytes())
                .await?;
            if answer.status == StatusCode::CREATED || answer.status == StatusCode::OK {
                return Ok(());
            }
            // Twilio's message often quotes the number ("The 'To' number
            // +1555... is not valid"), so only its error code is kept.
            let code = twilio_error_code(&answer.body);
            let status = answer.status.as_u16();
            match code {
                Some(code) => anyhow::bail!(
                    "the SMS provider refused the message (HTTP {status}, error {code})"
                ),
                None => anyhow::bail!("the SMS provider refused the message (HTTP {status})"),
            }
        })
    }
}

// ---- Twilio's webhook signature ---------------------------------------------

/// The signature Twilio puts in `X-Twilio-Signature` on a request it makes
/// to a webhook: HMAC-SHA1, keyed with the account's auth token, over the
/// whole URL Twilio requested (query included) followed by every POST
/// parameter's name and value, sorted by name (and by value, for a name
/// given more than once), base64. Twilio signs with the auth token even
/// when the service sends with an API key, whose secret cannot check it.
pub fn twilio_signature(auth_token: &str, url: &str, params: &[(String, String)]) -> String {
    use hmac::Mac;
    base64::engine::general_purpose::STANDARD.encode(
        signature_mac(auth_token, url, params)
            .finalize()
            .into_bytes(),
    )
}

fn signature_mac(
    auth_token: &str,
    url: &str,
    params: &[(String, String)],
) -> hmac::Hmac<sha1::Sha1> {
    use hmac::{KeyInit, Mac};
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort();
    let mut mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(auth_token.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(url.as_bytes());
    for (name, value) in sorted {
        mac.update(name.as_bytes());
        mac.update(value.as_bytes());
    }
    mac
}

/// Whether `header`, the request's `X-Twilio-Signature`, is Twilio's
/// signature of `url` and `params` under `auth_token`. Compared in
/// constant time.
pub fn twilio_signature_valid(
    auth_token: &str,
    url: &str,
    params: &[(String, String)],
    header: &str,
) -> bool {
    use hmac::Mac;
    let Ok(given) = base64::engine::general_purpose::STANDARD.decode(header.trim()) else {
        return false;
    };
    signature_mac(auth_token, url, params)
        .verify_slice(&given)
        .is_ok()
}

// ---- Choosing the channel ---------------------------------------------------

/// How one-time codes reach phone numbers (`SMS_CODE_DELIVERY`).
#[derive(Clone)]
pub enum PhoneCodes {
    /// Development delivery: the service makes the code and writes it to
    /// the log, the number masked ([`LogSender`]). Counted as a text would
    /// be, so development meets every rule a real text does.
    Log,
    /// Twilio Verify makes the code, texts it in its own template from its
    /// own senders, and checks it (`super::verify`).
    Verify(Arc<dyn CodeVerifier>),
}

/// Sends a code for a phone number the way `SMS_CODE_DELIVERY` says
/// ([`PhoneCodes`]), and any other code the way it went before
/// (`CODE_DELIVERY`). A code for a phone number is counted as a text
/// message, against the service's hourly caps, whichever way it goes.
pub struct CodeRouter {
    email: Arc<dyn CodeSender>,
    phone: PhoneCodes,
}

impl CodeRouter {
    pub fn new(email: Arc<dyn CodeSender>, phone: PhoneCodes) -> Self {
        Self { email, phone }
    }
}

impl CodeSender for CodeRouter {
    fn send<'a>(&'a self, message: CodeMessage<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            match (message.to, &self.phone) {
                (Identifier::Email(_), _) => self.email.send(message).await,
                (Identifier::Phone(_), PhoneCodes::Log) => LogSender.send(message).await,
                // `crate::auth::request_code` has Verify make the code.
                (Identifier::Phone(_), PhoneCodes::Verify(_)) => {
                    anyhow::bail!("a code for a phone number is made and sent by Twilio Verify")
                }
            }
        })
    }

    fn charged_per_message(&self, to: &Identifier) -> bool {
        matches!(to, Identifier::Phone(_))
    }

    fn delivers(&self, channel: SignInChannel) -> bool {
        match channel {
            SignInChannel::Phone => true,
            SignInChannel::Email => self.email.delivers(channel),
        }
    }

    fn email_sender(&self) -> Option<String> {
        self.email.email_sender()
    }

    fn verifier(&self, to: &Identifier) -> Option<&dyn CodeVerifier> {
        match (to, &self.phone) {
            (Identifier::Phone(_), PhoneCodes::Verify(verifier)) => Some(verifier.as_ref()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_gsm_and_an_accent_outside_it_is_ucs2() {
        assert_eq!(encoding("Code 123456."), Encoding::Gsm7 { septets: 12 });
        // In the GSM alphabet.
        assert_eq!(encoding("é ñ ü ¿ ¡"), Encoding::Gsm7 { septets: 9 });
        // Two septets each.
        assert_eq!(encoding("€[]"), Encoding::Gsm7 { septets: 6 });
        // Not in it: the whole text goes as UCS-2.
        assert_eq!(encoding("código"), Encoding::Ucs2 { units: 6 });
        assert_eq!(encoding("Don’t"), Encoding::Ucs2 { units: 5 });
        assert!(Encoding::Gsm7 { septets: 160 }.fits_one_segment());
        assert!(!Encoding::Gsm7 { septets: 161 }.fits_one_segment());
        assert!(Encoding::Ucs2 { units: 70 }.fits_one_segment());
        assert!(!Encoding::Ucs2 { units: 71 }.fits_one_segment());
    }

    #[test]
    fn twilios_signature_is_checked_as_its_documentation_computes_it() {
        // The example in Twilio's documentation ("Webhooks security"):
        // this URL, these parameters and this auth token sign as below.
        let token = "12345";
        let url = "https://example.com/myapp.php?foo=1&bar=2";
        let params: Vec<(String, String)> = [
            ("CallSid", "CA1234567890ABCDE"),
            ("Caller", "+14158675310"),
            ("Digits", "1234"),
            ("From", "+14158675310"),
            ("To", "+18005551212"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
        let signature = twilio_signature(token, url, &params);
        assert_eq!(signature, "L/OH5YylLD5NRKLltdqwSvS0BnU=");
        assert!(twilio_signature_valid(token, url, &params, &signature));
        // The order they arrive in does not matter.
        let mut reversed = params.clone();
        reversed.reverse();
        assert!(twilio_signature_valid(token, url, &reversed, &signature));
        // Anything else does.
        assert!(!twilio_signature_valid("54321", url, &params, &signature));
        let bare = "https://example.com/myapp.php";
        assert!(!twilio_signature_valid(token, bare, &params, &signature));
        let mut changed = params.clone();
        changed[2].1 = "1235".to_owned();
        assert!(!twilio_signature_valid(token, url, &changed, &signature));
        assert!(!twilio_signature_valid(token, url, &params, "not base64!"));
        assert!(!twilio_signature_valid(token, url, &params, ""));
    }

    #[test]
    fn a_number_is_shown_masked() {
        assert_eq!(masked("+15551234567"), "+1••••••••67");
        assert_eq!(masked("not a number"), "[a phone number]");
        assert_eq!(masked("ana@example.test"), "[a phone number]");
    }

    #[test]
    fn the_form_names_the_number_or_the_messaging_service() {
        let sender = |from: &str| {
            TwilioSmsSender::new(
                TWILIO_ORIGIN,
                "AC123".to_owned(),
                TwilioCredential::AuthToken(Secret::new("token".to_owned())),
                from.to_owned(),
                Duration::from_secs(1),
            )
        };
        let sms = Sms {
            to: "+15551234567",
            text: "123456 es tu código & más",
        };
        assert_eq!(
            sender("+15550000000").form(sms),
            "To=%2B15551234567&From=%2B15550000000&Body=123456+es+tu+c%C3%B3digo+%26+m%C3%A1s"
        );
        assert!(
            sender("MG0123")
                .form(sms)
                .contains("&MessagingServiceSid=MG0123&")
        );
        assert_eq!(
            sender("+15550000000").url(),
            "https://api.twilio.com/2010-04-01/Accounts/AC123/Messages.json"
        );
        let headers = sender("+15550000000").headers();
        let (_, authorization) = &headers[0];
        // base64 of "AC123:token".
        assert_eq!(authorization.to_str().unwrap(), "Basic QUMxMjM6dG9rZW4=");
        assert!(authorization.is_sensitive());
    }

    #[test]
    fn an_api_key_signs_the_request_and_the_account_is_still_in_the_path() {
        let sender = TwilioSmsSender::new(
            TWILIO_ORIGIN,
            "AC123".to_owned(),
            TwilioCredential::ApiKey {
                sid: "SK456".to_owned(),
                secret: Secret::new("key-secret".to_owned()),
            },
            "+15550000000".to_owned(),
            Duration::from_secs(1),
        );
        assert_eq!(
            sender.url(),
            "https://api.twilio.com/2010-04-01/Accounts/AC123/Messages.json"
        );
        let headers = sender.headers();
        let (_, authorization) = &headers[0];
        // base64 of "SK456:key-secret".
        assert_eq!(
            authorization.to_str().unwrap(),
            "Basic U0s0NTY6a2V5LXNlY3JldA=="
        );
        assert!(authorization.is_sensitive());
        assert!(!format!("{:?}", sender.credential).contains("key-secret"));
    }
}
