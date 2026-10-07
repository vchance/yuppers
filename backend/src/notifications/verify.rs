//! One-time codes for phone numbers through Twilio Verify
//! (`SMS_CODE_DELIVERY=verify`).
//!
//! Twilio makes the code, texts it from its own senders in its own template,
//! and checks it; the service never sees the code it sends. Verify needs no
//! messaging campaign of ours, which the owner's Sole Proprietor brand could
//! not register for codes: the campaign on the Messaging API covers only
//! "Yuppers.app agreement updates" (`super::sms_updates`).
//!
//! Two calls, each a form POST with HTTP Basic, the same credentials as the
//! Messages API (`SMS_ACCOUNT_SID` and an API key or the auth token):
//!
//! - **Starting a verification**,
//!   `POST https://verify.twilio.com/v2/Services/{ServiceSid}/Verifications`
//!   with `To` (E.164), `Channel=sms` and `Locale` (`en`, `es`): Twilio texts
//!   a code. While a verification is pending, asking again resends the same
//!   code: "The token remains the same during its validity period until the
//!   verification is successful", ten minutes by default. See
//!   <https://www.twilio.com/docs/verify/api/verification> and
//!   <https://www.twilio.com/docs/verify/api/rate-limits-and-timeouts>.
//! - **Checking a code**,
//!   `POST https://verify.twilio.com/v2/Services/{ServiceSid}/VerificationCheck`
//!   with `To` and `Code`: `status` is `approved` for the right code, and
//!   stays `pending` for a wrong one. Twilio deletes a verification once it
//!   is approved, expired or out of check attempts, and a check then answers
//!   `404 Not Found`; past its check attempts it answers error 60202. See
//!   <https://www.twilio.com/docs/verify/api/verification-check> and
//!   <https://www.twilio.com/docs/api/errors/60202>.
//!
//! **Purposes.** A Verify service's codes are one sequence per number: a
//! code started for signing in would be resent, and approved, by a deletion
//! started at the same number in the same ten minutes. So each purpose has
//! a service of its own (`TWILIO_VERIFY_SERVICE_SID` for signing in and
//! confirming a number, which have always been one kind of code, and
//! `TWILIO_VERIFY_DELETION_SERVICE_SID` for deleting an account), and a code
//! is only ever checked against its own purpose's service. The service's
//! own records bind it too: a code is put to Twilio only when a request for
//! that identifier and purpose is live (`crate::auth::OfferedCode`).
//!
//! What this file never does: write a phone number in full, a code, or
//! Twilio's message (which may quote the number) to a log or an error. A
//! refusal is described by its HTTP status and Twilio's error code.

use std::time::Duration;

use hyper::http::StatusCode;

use super::sms::{TwilioCredential, twilio_error_code, twilio_headers};
use crate::auth::{CheckFuture, CodeVerifier, Purpose, SendFuture};
use crate::outbound::Client;

/// Where Twilio's Verify API is.
pub const VERIFY_ORIGIN: &str = "https://verify.twilio.com";

/// Twilio's error for a verification checked too many times: a wrong code,
/// as far as the person is concerned.
const MAX_CHECK_ATTEMPTS_REACHED: u64 = 60202;

/// The Verify services, one per purpose ([`Purpose`]), each `VA…`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyServices {
    /// Signing in, and confirming a number added to an account.
    pub sign_in: String,
    /// Confirming that an account is to be deleted.
    pub delete_account: String,
}

/// Makes, sends and checks codes through Twilio Verify.
pub struct TwilioVerify {
    client: Client,
    origin: String,
    account_sid: String,
    credential: TwilioCredential,
    services: VerifyServices,
}

impl TwilioVerify {
    /// A verifier against the API at `origin`: [`VERIFY_ORIGIN`], or a
    /// stand-in in a test. Each request must be answered within `timeout`.
    pub fn new(
        origin: &str,
        account_sid: String,
        credential: TwilioCredential,
        services: VerifyServices,
        timeout: Duration,
    ) -> Self {
        Self {
            client: Client::new(timeout),
            origin: origin.trim_end_matches('/').to_owned(),
            account_sid,
            credential,
            services,
        }
    }

    /// The URL of `resource` (`Verifications`, `VerificationCheck`) in the
    /// service for `purpose`.
    pub fn url(&self, purpose: Purpose, resource: &str) -> String {
        let service = match purpose {
            Purpose::SignIn => &self.services.sign_in,
            Purpose::DeleteAccount => &self.services.delete_account,
        };
        let service: String = form_urlencoded::byte_serialize(service.as_bytes()).collect();
        format!("{}/v2/Services/{service}/{resource}", self.origin)
    }

    /// The form that starts a verification: who to, by text, in what
    /// language.
    pub fn start_form(to: &str, language: &str) -> String {
        form_urlencoded::Serializer::new(String::new())
            .append_pair("To", to)
            .append_pair("Channel", "sms")
            .append_pair("Locale", language)
            .finish()
    }

    /// The form that checks a code.
    pub fn check_form(to: &str, code: &str) -> String {
        form_urlencoded::Serializer::new(String::new())
            .append_pair("To", to)
            .append_pair("Code", code)
            .finish()
    }

    async fn post(&self, url: &str, form: String) -> anyhow::Result<crate::outbound::Answer> {
        let headers = twilio_headers(&self.account_sid, &self.credential);
        self.client.post(url, &headers, form.into_bytes()).await
    }
}

/// A refusal, by its status and Twilio's error code only.
fn refused(what: &str, status: StatusCode, body: &[u8]) -> anyhow::Error {
    let status = status.as_u16();
    match twilio_error_code(body) {
        Some(code) => anyhow::anyhow!("Twilio Verify refused {what} (HTTP {status}, error {code})"),
        None => anyhow::anyhow!("Twilio Verify refused {what} (HTTP {status})"),
    }
}

impl CodeVerifier for TwilioVerify {
    fn start<'a>(&'a self, to: &'a str, purpose: Purpose, language: &'a str) -> SendFuture<'a> {
        Box::pin(async move {
            let answer = self
                .post(
                    &self.url(purpose, "Verifications"),
                    Self::start_form(to, language),
                )
                .await?;
            if answer.status == StatusCode::CREATED || answer.status == StatusCode::OK {
                return Ok(());
            }
            Err(refused("the code", answer.status, &answer.body))
        })
    }

    fn check<'a>(&'a self, to: &'a str, purpose: Purpose, code: &'a str) -> CheckFuture<'a> {
        Box::pin(async move {
            let answer = self
                .post(
                    &self.url(purpose, "VerificationCheck"),
                    Self::check_form(to, code),
                )
                .await?;
            match answer.status {
                StatusCode::OK | StatusCode::CREATED => {
                    let status = serde_json::from_slice::<serde_json::Value>(&answer.body)
                        .ok()
                        .and_then(|body| body["status"].as_str().map(str::to_owned));
                    match status.as_deref() {
                        Some("approved") => Ok(true),
                        Some(_) => Ok(false),
                        None => anyhow::bail!("Twilio Verify's answer to a check had no status"),
                    }
                }
                // Approved already, expired, or out of attempts: gone.
                StatusCode::NOT_FOUND => Ok(false),
                StatusCode::TOO_MANY_REQUESTS
                    if twilio_error_code(&answer.body) == Some(MAX_CHECK_ATTEMPTS_REACHED) =>
                {
                    Ok(false)
                }
                status => Err(refused("a check", status, &answer.body)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::smtp::Secret;

    fn verify() -> TwilioVerify {
        TwilioVerify::new(
            VERIFY_ORIGIN,
            "AC123".to_owned(),
            TwilioCredential::AuthToken(Secret::new("token".to_owned())),
            VerifyServices {
                sign_in: "VA111".to_owned(),
                delete_account: "VA222".to_owned(),
            },
            Duration::from_secs(1),
        )
    }

    #[test]
    fn each_purpose_has_its_own_service() {
        let verify = verify();
        assert_eq!(
            verify.url(Purpose::SignIn, "Verifications"),
            "https://verify.twilio.com/v2/Services/VA111/Verifications"
        );
        assert_eq!(
            verify.url(Purpose::DeleteAccount, "VerificationCheck"),
            "https://verify.twilio.com/v2/Services/VA222/VerificationCheck"
        );
    }

    #[test]
    fn the_forms_are_the_ones_verify_takes() {
        assert_eq!(
            TwilioVerify::start_form("+15551234567", "es"),
            "To=%2B15551234567&Channel=sms&Locale=es"
        );
        assert_eq!(
            TwilioVerify::check_form("+15551234567", "123456"),
            "To=%2B15551234567&Code=123456"
        );
    }

    #[test]
    fn a_refusal_names_the_status_and_code_only() {
        let body = br#"{"code": 60200, "message": "Invalid parameter: To +15551234567"}"#;
        let error = refused("the code", StatusCode::BAD_REQUEST, body).to_string();
        assert_eq!(
            error,
            "Twilio Verify refused the code (HTTP 400, error 60200)"
        );
        assert!(!error.contains("5551234567"));
    }
}
