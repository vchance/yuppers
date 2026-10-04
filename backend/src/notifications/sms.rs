//! One-time codes by text message (DESIGN.md §8, §12): the SMS adapter at
//! the edge.
//!
//! SMS carries one-time codes and nothing else. A code for a phone number
//! goes through an [`SmsSender`]; a code for an email address goes on as
//! before. [`CodeRouter`] makes that choice, so the rest of the service
//! still sees one [`CodeSender`].
//!
//! Each message costs money, so it is kept to one segment in every language
//! (`encoding` below, and the tests), and the service caps how many it sends
//! per hour (`crate::auth`, `SMS_MAX_PER_HOUR`).
//!
//! What this file never does: write a phone number in full, or a code
//! outside the development delivery, to a log or an error.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use hyper::http::{HeaderName, HeaderValue, StatusCode, header};

use super::smtp::Secret;
use super::wording::Wording;
use crate::auth::{CodeMessage, CodeSender, SendFuture, SignInChannel};
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
        let (username, password) = match &self.credential {
            TwilioCredential::AuthToken(token) => (self.account_sid.as_str(), token),
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
            let code = serde_json::from_slice::<serde_json::Value>(&answer.body)
                .ok()
                .and_then(|body| body["code"].as_u64());
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

// ---- Choosing the channel ---------------------------------------------------

/// Sends a code for a phone number by SMS, and any other code the way it
/// went before (`CODE_DELIVERY`).
pub struct CodeRouter {
    email: Arc<dyn CodeSender>,
    sms: Arc<dyn SmsSender>,
    wording: Wording,
}

impl CodeRouter {
    pub fn new(email: Arc<dyn CodeSender>, sms: Arc<dyn SmsSender>, wording: Wording) -> Self {
        Self {
            email,
            sms,
            wording,
        }
    }
}

impl CodeSender for CodeRouter {
    fn send<'a>(&'a self, message: CodeMessage<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            match message.to {
                Identifier::Phone(number) => {
                    let text =
                        self.wording
                            .code_sms(message.language, message.purpose, message.code);
                    self.sms
                        .send(Sms {
                            to: number,
                            text: &text,
                        })
                        .await
                }
                Identifier::Email(_) => self.email.send(message).await,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Purpose;
    use crate::languages;

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
    fn every_code_message_fits_one_segment_in_every_language() {
        let wording = Wording::embedded().unwrap();
        for language in languages::supported() {
            for purpose in [Purpose::SignIn, Purpose::DeleteAccount] {
                let text = wording.code_sms(language, purpose, "123456");
                let encoding = encoding(&text);
                assert!(
                    encoding.fits_one_segment(),
                    "{language} {}: {encoding:?} for {text:?}",
                    purpose.as_str()
                );
                assert!(text.contains("123456"), "{language}: {text:?}");
                assert!(text.contains("Yuppers"), "{language}: {text:?}");
                assert!(!text.contains('{') && !text.contains('}'), "{text:?}");
            }
        }
    }

    #[test]
    fn the_code_messages_read_as_written_and_are_sent_as_expected() {
        let wording = Wording::embedded().unwrap();
        let cases = [
            (
                "en",
                Purpose::SignIn,
                "123456 is your Yuppers sign-in code. Do not share it with anyone.",
                Encoding::Gsm7 { septets: 65 },
            ),
            (
                "en",
                Purpose::DeleteAccount,
                "123456 is your code to delete your Yuppers account. Do not share it with anyone.",
                Encoding::Gsm7 { septets: 80 },
            ),
            // "código" is not in the GSM alphabet, so Spanish goes as UCS-2
            // and must stay within 70.
            (
                "es",
                Purpose::SignIn,
                "123456 es tu código de Yuppers para entrar. No se lo des a nadie.",
                Encoding::Ucs2 { units: 65 },
            ),
            (
                "es",
                Purpose::DeleteAccount,
                "123456: código para eliminar tu cuenta de Yuppers. No lo compartas.",
                Encoding::Ucs2 { units: 67 },
            ),
        ];
        for (language, purpose, text, sent_as) in cases {
            let written = wording.code_sms(language, purpose, "123456");
            assert_eq!(written, text);
            assert_eq!(encoding(&written), sent_as, "{written}");
        }
        // A regional tag gets its base language; an unknown one the default.
        assert_eq!(
            wording.code_sms("es-MX", Purpose::SignIn, "1"),
            wording.code_sms("es", Purpose::SignIn, "1")
        );
        assert_eq!(
            wording.code_sms("tlh", Purpose::SignIn, "1"),
            wording.code_sms(languages::default(), Purpose::SignIn, "1")
        );
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
