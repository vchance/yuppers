use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

use crate::domain::exchange::Refusal;
use crate::domain::identity::InvalidIdentifier;

/// Stable codes for every refusal the API can return. Clients map these to
/// wording in the user's language and never parse message text
/// (DESIGN.md §13.3). Adding a code here is a contract change: the generated
/// client makes the shared wording tables fail to compile until it is covered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// The revision named in the request is no longer the open one.
    StaleRevision,
    /// The action belongs to the other party.
    WrongActor,
    /// The action is not allowed in the current state.
    ActionNotAllowed,
    /// An accepted contribution cannot be changed.
    ContributionLocked,
    /// The revision ran out before it was accepted.
    RevisionExpired,
    /// The initiator must confirm who the counterparty is first.
    CounterpartyNotConfirmed,
    /// The caller opened an invitation that named nobody, and the initiator
    /// has not confirmed them yet. Until then they can sign or leave.
    AwaitingConfirmation,
    /// The revision breaks a rule and was not sent.
    InvalidRevision,
    /// The request body is missing, malformed or out of range.
    InvalidRequest,
    /// Not a usable email address or phone number.
    InvalidIdentifier,
    /// A phone number of a country the service does not send codes to
    /// (`SMS_ALLOWED_COUNTRY_CODES`). An email address still works.
    PhoneCountryNotServed,
    /// The phone number replied STOP to our texts, so nothing is texted to
    /// it, one-time codes included, until it replies START. An email address
    /// still works.
    PhoneOptedOut,
    /// A code was asked for by text without the box beside the number ticked:
    /// the request named no `sms_consent`, or wording that is not the current
    /// one. Nothing was counted or sent. An email address needs none.
    SmsConsentRequired,
    /// A sign-in named a version of the Terms and the Privacy policy that
    /// the service does not know: a page loaded before they changed.
    /// Reloading shows the current sentence. Nothing was stored.
    TermsVersionUnknown,
    /// The one-time code is wrong, expired or used up. Deliberately one code
    /// for all three, so a guesser learns nothing.
    InvalidCode,
    /// Too many requests in a short time: one-time codes asked for, wrong
    /// codes offered from one network address, or other rate limits.
    TooManyRequests,
    /// Too many wrong one-time codes for this email address or phone number
    /// today (or, for deleting an account, by this account). Its codes are
    /// refused, right or wrong, until the day ends (UTC).
    TooManyGuesses,
    /// No valid session.
    Unauthenticated,
    AccountSuspended,
    /// The email address or phone number belongs to another account. No
    /// longer answered once its code is right: that is
    /// `IDENTIFIER_ON_OTHER_ACCOUNT`, with an offer to combine the two.
    IdentifierInUse,
    /// The code was right, and the email address or phone number belongs to
    /// another account, which the person has now shown they control. The
    /// body carries `combine`: what that account has, and a token for
    /// combining the two (`POST /v1/me/combine`).
    IdentifierOnOtherAccount,
    /// The account already has an email address (or phone number) of its
    /// own, and only one of each is kept: the request must say to replace it.
    IdentifierKindTaken,
    /// Adding or replacing an email address or phone number, while the
    /// account has one, needs a proof of one it already has, from a code
    /// sent to it (`POST /v1/me/identifiers/proof`), and none was given, or
    /// it is used, expired or another account's.
    ProofRequired,
    /// The code or proof is from an email address or phone number that came
    /// to the account in the last 24 hours, and would remove or replace one
    /// the account had before it. Use the older one, or wait.
    IdentifierTooRecent,
    /// The address typed is not the one the invitation was sent to. Says
    /// nothing more about it.
    NotInvitedAddress,
    /// No code could be sent to that address. Says nothing more about why.
    CodeNotSent,
    /// The account's only email address or phone number cannot be removed:
    /// it is how the account is signed in to. Add the other kind first.
    LastIdentifier,
    /// The offer to combine accounts is used, expired, or no longer true:
    /// the address or number proved has since left the other account, or
    /// that account is gone. Prove the address again for a new offer.
    CombineExpired,
    /// One of the two accounts is suspended, so they cannot be combined.
    CombineSuspended,
    /// The other account reviews reports, so it cannot be combined into
    /// another. The owner removes the role first.
    CombineReviewer,
    /// The two accounts are on the two sides of the same yup (or one opened
    /// the other's invitation), so they cannot be combined: one person
    /// cannot be both parties to an agreement.
    CombineSharedExchange,
    /// The exchange changed since the client last read it.
    VersionConflict,
    /// A display name and confirmation of age are needed before signing.
    ProfileIncomplete,
    /// The consent wording shown is not the current version.
    ConsentOutdated,
    /// The invitation link is not valid, or no longer.
    InvitationUnavailable,
    /// The invitation names someone else.
    InvitationNotForYou,
    /// The idempotency key was already used for a different request.
    IdempotencyKeyReused,
    /// The client build is too old to act and must update.
    ClientTooOld,
    /// Passes for this wallet are not issued here: the platform is not
    /// configured (DESIGN.md §11).
    WalletUnavailable,
    /// Staff review needs a recent sign-in: the session's one-time code was
    /// entered too long ago. Sign out and sign in again.
    SessionTooOld,
    /// A reviewer has hidden what was written in this exchange from the
    /// caller, so it cannot be signed or changed from here.
    ContentHidden,
    /// The report has already been resolved.
    ReportResolved,
    /// A reviewer cannot suspend another reviewer, or lift a reviewer's
    /// suspension. The owner takes the reviewer's role away first, from the
    /// command line (`staff revoke`); the report can then be resolved.
    SubjectIsReviewer,
    NotFound,
    ServiceUnavailable,
    Internal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: ErrorCode,
}

impl ApiError {
    pub const fn new(status: StatusCode, code: ErrorCode) -> Self {
        Self { status, code }
    }
}

impl From<ErrorCode> for ApiError {
    fn from(code: ErrorCode) -> Self {
        use ErrorCode::*;
        let status = match code {
            InvalidRequest
            | InvalidIdentifier
            | PhoneCountryNotServed
            | NotInvitedAddress
            | SmsConsentRequired
            | TermsVersionUnknown
            | InvalidRevision
            | IdempotencyKeyReused => StatusCode::UNPROCESSABLE_ENTITY,
            InvalidCode | Unauthenticated | SessionTooOld => StatusCode::UNAUTHORIZED,
            WrongActor | AccountSuspended | InvitationNotForYou => StatusCode::FORBIDDEN,
            NotFound | InvitationUnavailable | WalletUnavailable => StatusCode::NOT_FOUND,
            TooManyRequests | TooManyGuesses => StatusCode::TOO_MANY_REQUESTS,
            StaleRevision
            | ActionNotAllowed
            | ContributionLocked
            | RevisionExpired
            | CounterpartyNotConfirmed
            | AwaitingConfirmation
            | IdentifierInUse
            | IdentifierOnOtherAccount
            | IdentifierKindTaken
            | ProofRequired
            | IdentifierTooRecent
            | CodeNotSent
            | LastIdentifier
            | CombineExpired
            | CombineSuspended
            | CombineReviewer
            | CombineSharedExchange
            | PhoneOptedOut
            | VersionConflict
            | ProfileIncomplete
            | ConsentOutdated
            | ContentHidden
            | ReportResolved
            | SubjectIsReviewer => StatusCode::CONFLICT,
            ClientTooOld => StatusCode::UPGRADE_REQUIRED,
            ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, code)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorBody { code: self.code })).into_response()
    }
}

impl From<Refusal> for ApiError {
    fn from(refusal: Refusal) -> Self {
        let code = match refusal {
            Refusal::WrongActor => ErrorCode::WrongActor,
            Refusal::NotAllowed => ErrorCode::ActionNotAllowed,
            Refusal::StaleRevision => ErrorCode::StaleRevision,
            Refusal::RevisionExpired => ErrorCode::RevisionExpired,
            Refusal::CounterpartyNotConfirmed => ErrorCode::CounterpartyNotConfirmed,
            Refusal::AwaitingConfirmation => ErrorCode::AwaitingConfirmation,
            Refusal::ContributionLocked(_) => ErrorCode::ContributionLocked,
            Refusal::UnknownContribution(_) => ErrorCode::NotFound,
            Refusal::InvalidRevision(_) => ErrorCode::InvalidRevision,
        };
        code.into()
    }
}

impl From<InvalidIdentifier> for ApiError {
    fn from(_: InvalidIdentifier) -> Self {
        ErrorCode::InvalidIdentifier.into()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        // The server's message can quote a key value, such as an email
        // address, so only what names the failure is logged, never the text.
        match &error {
            sqlx::Error::Database(failure) => tracing::error!(
                sqlstate = failure.code().as_deref().unwrap_or("?"),
                constraint = failure.constraint().unwrap_or("-"),
                table = failure.table().unwrap_or("-"),
                "database error"
            ),
            other => tracing::error!(kind = variant_name(other), "database error"),
        }
        ErrorCode::Internal.into()
    }
}

/// A database error in the form that may be logged or stored: the same
/// redaction as an API request's database error above. The server's
/// message, which can quote a key value such as an email address, is left
/// out; what names the failure (the SQLSTATE, the constraint and the table,
/// or the kind of error) is kept. The worker logs its jobs' errors with it.
pub struct Redacted<'a>(pub &'a sqlx::Error);

impl std::fmt::Display for Redacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            sqlx::Error::Database(failure) => write!(
                f,
                "database error: sqlstate {}, constraint {}, table {}",
                failure.code().as_deref().unwrap_or("?"),
                failure.constraint().unwrap_or("-"),
                failure.table().unwrap_or("-"),
            ),
            other => write!(f, "database error: {}", variant_name(other)),
        }
    }
}

/// The variant of an error, without its contents.
fn variant_name(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::Configuration(_) => "configuration",
        sqlx::Error::Io(_) => "io",
        sqlx::Error::Tls(_) => "tls",
        sqlx::Error::Protocol(_) => "protocol",
        sqlx::Error::RowNotFound => "row not found",
        sqlx::Error::TypeNotFound { .. } => "type not found",
        sqlx::Error::ColumnIndexOutOfBounds { .. } => "column index out of bounds",
        sqlx::Error::ColumnNotFound(_) => "column not found",
        sqlx::Error::ColumnDecode { .. } => "column decode",
        sqlx::Error::Encode(_) => "encode",
        sqlx::Error::Decode(_) => "decode",
        sqlx::Error::AnyDriverError(_) => "driver",
        sqlx::Error::PoolTimedOut => "pool timed out",
        sqlx::Error::PoolClosed => "pool closed",
        sqlx::Error::WorkerCrashed => "worker crashed",
        sqlx::Error::Migrate(_) => "migrate",
        sqlx::Error::Database(_) => "database",
        _ => "other",
    }
}
