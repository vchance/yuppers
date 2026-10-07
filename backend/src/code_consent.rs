//! Consent to a one-time code by text, which Twilio Verify sends and checks
//! (`crate::notifications::verify`; README, "Signing in"): no SMS program of
//! ours, but each code is still texted only on a ticked box.
//!
//! Every form that texts a code shows a box, never ticked to begin with,
//! beside wording that says what will be sent, from whom and how often, the
//! rates, HELP and STOP, and the two policies' addresses (`smsCode` in the
//! wording files; the terms quote it word for word). The form sends a code
//! to a phone number only once the box is ticked, and says so in the
//! request: `sms_consent`, the version and language of the wording shown.
//! The service refuses a code for a phone number without it
//! (`SMS_CONSENT_REQUIRED`), before anything is counted or sent; an email
//! address needs none and the field is ignored for one.
//!
//! Three forms text a code, each with its own words before the shared rest:
//! signing in (`POST /v1/auth/codes`), confirming a deletion
//! (`POST /v1/me/deletion/codes`), and checking a number added for agreement
//! updates (`POST /v1/auth/codes` again, from a signed-in account: the only
//! place a signed-in person asks for one).
//!
//! **The record.** Each code issued after a tick is recorded with it, in the
//! transaction that stores the code (`sms_code_consent`, migration 0021):
//! why the code was asked for, the account where there is one, the number,
//! the wording's version and language, the client and the time, with the
//! request's address and user agent kept apart for as long as a signature's
//! (`sms_code_consent_network`). The number is kept in full only where it is
//! already the account's own; a number not yet shown to be anyone's is kept
//! as a keyed hash, which is enough to find the record from the number. The
//! worker removes records after `Rules::sms_consent_retention`, as for the
//! updates' consent ([`purge`]).

use std::net::IpAddr;

use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::Sha256;
use sqlx::{PgConnection, PgPool};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::auth::Purpose;
use crate::domain::Rules;
use crate::domain::identity::Identifier;
use crate::error::{ApiError, ErrorCode};
use crate::languages;
use crate::notifications::sms_updates::Source;

/// The version of the wording beside the box (`smsCode` in the wording
/// files), the same for all three forms. A client names it when it asks for
/// a code by text, and it is stored with the consent; new wording gets a new
/// version here and in `packages/shared/src/sms-code-consent.ts`.
pub const CODE_CONSENT_VERSION: &str = "2026-10-06";

/// What the box beside a phone number said, as a client sends it.
#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct SmsCodeConsent {
    /// The version of the wording shown beside the box.
    pub version: String,
    /// The language it was shown in, as a language tag.
    pub language: String,
}

/// Why a code was asked for, as the record names it. Each has its own words
/// beside the box. The text that carries the code is Twilio Verify's, the
/// same for all three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodePurpose {
    SignIn,
    DeleteAccount,
    /// Adding a number to an account, to turn on agreement updates.
    VerifyNumber,
}

impl CodePurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            CodePurpose::SignIn => "SIGN_IN",
            CodePurpose::DeleteAccount => "DELETE_ACCOUNT",
            CodePurpose::VerifyNumber => "VERIFY_NUMBER",
        }
    }

    /// What the code itself is good for, which decides how it is stored,
    /// limited and checked. A number being added is proved with a sign-in
    /// code, as any identifier added to an account always has been: only
    /// the text that carries it says what it is for here.
    pub fn code_purpose(self) -> Purpose {
        match self {
            CodePurpose::SignIn | CodePurpose::VerifyNumber => Purpose::SignIn,
            CodePurpose::DeleteAccount => Purpose::DeleteAccount,
        }
    }
}

/// A request for a code, with what it said about consent and where it came
/// from. Checked and recorded by `crate::auth::request_code`.
#[derive(Clone, Debug)]
pub struct CodeRequest<'a> {
    pub purpose: CodePurpose,
    /// The signed-in account asking, for a deletion or a number being added.
    pub account: Option<Uuid>,
    /// The request's `sms_consent`, if it had one.
    pub consent: Option<&'a SmsCodeConsent>,
    pub source: Source,
    pub address: Option<IpAddr>,
    pub user_agent: Option<&'a str>,
}

/// A consent checked: the version the service knows, in a supported language.
#[derive(Clone, Copy, Debug)]
pub struct Checked {
    pub language: &'static str,
}

impl CodeRequest<'_> {
    /// Checks what the request says about consent for a code to
    /// `identifier`. `None` for an email address, which needs none, whatever
    /// the request said. For a phone number: refused with
    /// `SMS_CONSENT_REQUIRED` without it, and for wording the service does
    /// not know, which is no consent to what it would send now (a page
    /// loaded before the wording changed: reloading it shows the box again);
    /// and `INVALID_REQUEST` for a language it does not support.
    pub fn check(&self, identifier: &Identifier) -> Result<Option<Checked>, ApiError> {
        if !matches!(identifier, Identifier::Phone(_)) {
            return Ok(None);
        }
        let consent = self.consent.ok_or(ErrorCode::SmsConsentRequired)?;
        if consent.version != CODE_CONSENT_VERSION {
            return Err(ErrorCode::SmsConsentRequired.into());
        }
        let language = languages::resolve(&consent.language).ok_or(ErrorCode::InvalidRequest)?;
        Ok(Some(Checked { language }))
    }
}

/// The keyed hash a number is kept as: HMAC-SHA256 under `APP_SECRET`. A
/// plain hash of a phone number can be reversed by trying every number.
pub fn phone_hash(secret: &[u8], phone: &str) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(b"sms-code-consent\0");
    mac.update(phone.as_bytes());
    mac.finalize().into_bytes().into()
}

/// Records the consent behind a code just issued to `phone`, in the
/// transaction that stores the code.
pub(crate) async fn record(
    conn: &mut PgConnection,
    secret: &[u8],
    phone: &str,
    request: &CodeRequest<'_>,
    checked: Checked,
) -> Result<(), sqlx::Error> {
    // Whose number it is now, if anyone's.
    let holder: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM account WHERE phone = $1 AND status = 'ACTIVE'")
            .bind(phone)
            .fetch_optional(&mut *conn)
            .await?;
    let account = match request.purpose {
        // Nobody is signed in: the account is the one that has the number.
        CodePurpose::SignIn => holder,
        CodePurpose::DeleteAccount | CodePurpose::VerifyNumber => request.account,
    };
    // In full only where it is that account's own number already.
    let kept = (account.is_some() && account == holder).then_some(phone);
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO sms_code_consent
             (purpose, account_id, phone, phone_hash, source, consent_version, consent_language)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(request.purpose.as_str())
    .bind(account)
    .bind(kept)
    .bind(phone_hash(secret, phone).as_slice())
    .bind(request.source.as_str())
    // [`CodeRequest::check`] has held it to this one.
    .bind(CODE_CONSENT_VERSION)
    .bind(checked.language)
    .fetch_one(&mut *conn)
    .await?;
    let user_agent: Option<String> = request
        .user_agent
        .map(|agent| agent.chars().take(512).collect());
    sqlx::query(
        "INSERT INTO sms_code_consent_network (consent_id, ip_address, user_agent)
         VALUES ($1, $2::inet, $3)",
    )
    .bind(id)
    .bind(request.address.map(|address| address.to_string()))
    .bind(user_agent)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Removes code consents older than `Rules::sms_consent_retention`, and
/// their address and user agent after as long as a signature's. A code is
/// one text, sent once, so nothing goes on resting on its record. Returns
/// how many rows went. Called by the worker.
pub async fn purge(db: &PgPool, rules: &Rules, at: OffsetDateTime) -> Result<u64, sqlx::Error> {
    let network = sqlx::query("DELETE FROM sms_code_consent_network WHERE recorded_at < $1")
        .bind(at - rules.network_metadata_retention)
        .execute(db)
        .await?
        .rows_affected();
    let records = sqlx::query("DELETE FROM sms_code_consent WHERE created_at < $1")
        .bind(at - rules.sms_consent_retention)
        .execute(db)
        .await?
        .rows_affected();
    Ok(network + records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(consent: Option<&SmsCodeConsent>) -> CodeRequest<'_> {
        CodeRequest {
            purpose: CodePurpose::SignIn,
            account: None,
            consent,
            source: Source::Web,
            address: None,
            user_agent: None,
        }
    }

    #[test]
    fn a_phone_number_needs_the_current_wording_in_a_supported_language() {
        let phone = Identifier::parse("+12015550123").unwrap();
        let code = |result: Result<Option<Checked>, ApiError>| result.map(|_| ()).unwrap_err().code;
        assert_eq!(
            code(request(None).check(&phone)),
            ErrorCode::SmsConsentRequired
        );
        let old = SmsCodeConsent {
            version: "2026-01-01".to_owned(),
            language: "en".to_owned(),
        };
        assert_eq!(
            code(request(Some(&old)).check(&phone)),
            ErrorCode::SmsConsentRequired
        );
        let klingon = SmsCodeConsent {
            version: CODE_CONSENT_VERSION.to_owned(),
            language: "tlh".to_owned(),
        };
        assert_eq!(
            code(request(Some(&klingon)).check(&phone)),
            ErrorCode::InvalidRequest
        );
        let current = SmsCodeConsent {
            version: CODE_CONSENT_VERSION.to_owned(),
            language: "es-MX".to_owned(),
        };
        let checked = request(Some(&current)).check(&phone).unwrap().unwrap();
        assert_eq!(checked.language, "es");
    }

    #[test]
    fn an_email_address_needs_none_whatever_was_sent() {
        let email = Identifier::parse("ana@example.test").unwrap();
        assert!(request(None).check(&email).unwrap().is_none());
        let nonsense = SmsCodeConsent {
            version: "?".to_owned(),
            language: "?".to_owned(),
        };
        assert!(request(Some(&nonsense)).check(&email).unwrap().is_none());
    }

    #[test]
    fn a_number_being_added_is_proved_with_a_sign_in_code() {
        assert_eq!(CodePurpose::SignIn.code_purpose(), Purpose::SignIn);
        assert_eq!(CodePurpose::VerifyNumber.code_purpose(), Purpose::SignIn);
        assert_eq!(
            CodePurpose::DeleteAccount.code_purpose(),
            Purpose::DeleteAccount
        );
    }

    #[test]
    fn a_number_is_hashed_under_the_secret() {
        let one = phone_hash(b"one secret", "+12015550123");
        assert_eq!(one, phone_hash(b"one secret", "+12015550123"));
        assert_ne!(one, phone_hash(b"another secret", "+12015550123"));
        assert_ne!(one, phone_hash(b"one secret", "+12015550124"));
    }
}
