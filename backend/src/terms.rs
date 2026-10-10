//! Acceptance of the Terms and the Privacy policy at sign-in.
//!
//! The sign-in screens say, in one sentence above the button, that
//! continuing means agreeing to both. The request that completes a sign-in
//! names the version of that wording the client showed (`terms_version`);
//! the service refuses a version it does not know
//! (`TERMS_VERSION_UNKNOWN`), and otherwise stores what was accepted: one
//! append-only row per sign-in (`terms_acceptance`, migration 0032), and the
//! latest on the account (`terms_version`, `terms_accepted_at`) so that
//! `GET /v1/me` can say it. Neither is ever shown to the other party.
//! Deleting an account removes its rows (`crate::deletion`).

use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{ApiError, ErrorCode};

/// The version of the Terms and the Privacy policy: the day they took effect
/// (`LEGAL_EFFECTIVE_DATES` in `packages/shared/src/legal-text.ts`). A client
/// names it when it completes a sign-in. Change it together with the
/// effective dates and `TERMS_VERSION` in `packages/shared/src/terms.ts`;
/// tests keep them in step.
pub const TERMS_VERSION: &str = "2026-10-09";

/// Refuses a version the service does not know: wording that is not the
/// current one, as from a page loaded before it changed.
pub fn check(version: &str) -> Result<(), ApiError> {
    if version == TERMS_VERSION {
        Ok(())
    } else {
        Err(ErrorCode::TermsVersionUnknown.into())
    }
}

/// Records the acceptance a completed sign-in made, in its transaction.
pub(crate) async fn record(
    conn: &mut PgConnection,
    account: Uuid,
    language: &str,
    session: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO terms_acceptance (account_id, terms_version, language, session_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(account)
    .bind(TERMS_VERSION)
    .bind(language)
    .bind(session)
    .execute(&mut *conn)
    .await?;
    sqlx::query("UPDATE account SET terms_version = $2, terms_accepted_at = now() WHERE id = $1")
        .bind(account)
        .bind(TERMS_VERSION)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_current_version_is_known() {
        assert!(check(TERMS_VERSION).is_ok());
        assert_eq!(
            check("2026-01-01").unwrap_err().code,
            ErrorCode::TermsVersionUnknown
        );
        assert_eq!(check("").unwrap_err().code, ErrorCode::TermsVersionUnknown);
    }
}
