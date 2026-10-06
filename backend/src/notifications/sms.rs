//! One-time codes by text message (DESIGN.md §8, §12): the SMS adapter at
//! the edge.
//!
//! SMS carries one-time codes, and the agreement updates a person turns on
//! (`super::sms_updates`, which the worker sends through the same
//! [`SmsSender`]). A code for a phone number goes through an [`SmsSender`];
//! a code for an email address goes on as before. [`CodeRouter`] makes that
//! choice, so the rest of the service still sees one [`CodeSender`].
//!
//! Texts people send back reach the service through Twilio's webhook,
//! whose requests are signed ([`twilio_signature_valid`]).
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
                            .code_sms(message.language, message.reason, message.code);
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
    use crate::code_consent::CodePurpose;
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

    const REASONS: [CodePurpose; 3] = [
        CodePurpose::SignIn,
        CodePurpose::DeleteAccount,
        CodePurpose::VerifyNumber,
    ];

    #[test]
    fn every_code_message_fits_one_segment_in_every_language() {
        let wording = Wording::embedded().unwrap();
        for language in languages::supported() {
            let texts = REASONS.map(|reason| wording.code_sms(language, reason, "123456"));
            for (reason, text) in REASONS.iter().zip(&texts) {
                let encoding = encoding(text);
                assert!(
                    encoding.fits_one_segment(),
                    "{language} {}: {encoding:?} for {text:?}",
                    reason.as_str()
                );
                // The program's name first, as in every text of ours.
                assert!(text.starts_with("Yuppers.app: "), "{language}: {text:?}");
                assert!(text.contains("123456"), "{language}: {text:?}");
                assert!(!text.contains('{') && !text.contains('}'), "{text:?}");
            }
            // Each says what its code is for.
            let [sign_in, delete, verify] = &texts;
            assert!(
                sign_in != delete && delete != verify && sign_in != verify,
                "{language}: {texts:?}"
            );
        }
    }

    #[test]
    fn the_code_messages_read_as_written_and_are_sent_as_expected() {
        let wording = Wording::embedded().unwrap();
        let cases = [
            (
                "en",
                CodePurpose::SignIn,
                "Yuppers.app: 123456 is your sign-in code. Do not share it with anyone.",
                Encoding::Gsm7 { septets: 70 },
            ),
            (
                "en",
                CodePurpose::DeleteAccount,
                "Yuppers.app: 123456 is your code to delete your account. Do not share it with anyone.",
                Encoding::Gsm7 { septets: 85 },
            ),
            (
                "en",
                CodePurpose::VerifyNumber,
                "Yuppers.app: 123456 is your code to confirm this phone number. Do not share it with anyone.",
                Encoding::Gsm7 { septets: 91 },
            ),
            // "código" is not in the GSM alphabet, so Spanish goes as UCS-2
            // and must stay within 70.
            (
                "es",
                CodePurpose::SignIn,
                "Yuppers.app: 123456 es tu código para entrar. No se lo des a nadie.",
                Encoding::Ucs2 { units: 67 },
            ),
            (
                "es",
                CodePurpose::DeleteAccount,
                "Yuppers.app: 123456: código para eliminar tu cuenta. No lo compartas.",
                Encoding::Ucs2 { units: 69 },
            ),
            (
                "es",
                CodePurpose::VerifyNumber,
                "Yuppers.app: 123456: código para confirmar tu número. No lo compartas.",
                Encoding::Ucs2 { units: 70 },
            ),
        ];
        for (language, reason, text, sent_as) in cases {
            let written = wording.code_sms(language, reason, "123456");
            assert_eq!(written, text);
            assert_eq!(encoding(&written), sent_as, "{written}");
            assert!(sent_as.fits_one_segment(), "{written}");
        }
        // A regional tag gets its base language; an unknown one the default.
        assert_eq!(
            wording.code_sms("es-MX", CodePurpose::SignIn, "1"),
            wording.code_sms("es", CodePurpose::SignIn, "1")
        );
        assert_eq!(
            wording.code_sms("tlh", CodePurpose::VerifyNumber, "1"),
            wording.code_sms(languages::default(), CodePurpose::VerifyNumber, "1")
        );
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
