//! Payment options (migration 0026; README, "Payment options"): the names a
//! person may save for being paid in another app, and, per agreement,
//! whether the other party is shown them.
//!
//! **Yuppers never moves money** (DESIGN.md §3, §7). A payment option is a
//! name in someone else's app. The person who owes money on an agreement in
//! force may be shown the other party's, as links that open that app, and
//! pays there or any other way; the payment is recorded only when they say
//! so themselves ("I've paid") and the other party confirms receiving it.
//! Nothing is inferred from a link being opened, and the service never
//! contacts the apps.
//!
//! **Optional, and off by default.** Saving any is optional, and saving them
//! shows them nowhere: a party shows them on one agreement at a time
//! (`payment_offer`), and can stop at any moment. They are not part of the
//! terms, not in a revision or its content hash, and not in the history.
//!
//! **At rest** they are encrypted as an email address is (`crate::contact`),
//! one column per app, bound to its table and column. They are decrypted to
//! show their owner, and to show the other party of an agreement where the
//! owner shows them, while that party owes them money still to be paid
//! ([`for_payer`]). Never in a notification, an email, a text message, a
//! push, a Wallet pass, a link preview, the record or a log.

use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::contact::{self, Field, Unreadable};
use crate::error::{ApiError, ErrorCode};

/// Changes one account may make to its payment options in an hour: saving
/// or removing them, and showing them on an agreement or not. More than a
/// person makes; few enough that nobody can use the endpoints to churn the
/// database. Counted on the account's `payment_handle` row.
pub const WRITES_PER_HOUR: i32 = 30;

impl Field {
    pub const PAYMENT_VENMO: Field = Field::new("payment_handle", "venmo");
    pub const PAYMENT_CASH_APP: Field = Field::new("payment_handle", "cash_app");
    pub const PAYMENT_PAYPAL: Field = Field::new("payment_handle", "paypal");
    pub const PAYMENT_ZELLE: Field = Field::new("payment_handle", "zelle");
}

/// A person's payment options, each optional. In a request, what to save;
/// in a reply, what is saved, as stored: a Venmo username without its `@`,
/// a $Cashtag without its `$`, a PayPal.Me name, and for Zelle a lower-case
/// email address or a US number in international form (`+12025550142`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PaymentHandles {
    /// A Venmo username: 5 to 30 letters, digits, `-` or `_`. A leading `@`
    /// is dropped.
    #[serde(default)]
    pub venmo: Option<String>,
    /// A Cash App $Cashtag: 1 to 20 letters, digits or `_`, with at least
    /// one letter. A leading `$` is dropped.
    #[serde(default)]
    pub cash_app: Option<String>,
    /// A PayPal.Me name: 1 to 20 letters or digits.
    #[serde(default)]
    pub paypal: Option<String>,
    /// The email address or US phone number Zelle pays this person at.
    /// Shown, never linked: Zelle has no links.
    #[serde(default)]
    pub zelle: Option<String>,
}

impl PaymentHandles {
    pub fn is_empty(&self) -> bool {
        self.venmo.is_none()
            && self.cash_app.is_none()
            && self.paypal.is_none()
            && self.zelle.is_none()
    }

    /// Each option as it is stored, or `InvalidRequest` for one that is not
    /// one. Empty text is no option.
    pub fn normalized(&self) -> Result<Self, ApiError> {
        fn each(
            value: &Option<String>,
            normalize: fn(&str) -> Option<String>,
        ) -> Result<Option<String>, ApiError> {
            match value.as_deref().map(str::trim) {
                None | Some("") => Ok(None),
                Some(text) => normalize(text)
                    .map(Some)
                    .ok_or_else(|| ErrorCode::InvalidRequest.into()),
            }
        }
        Ok(Self {
            venmo: each(&self.venmo, venmo)?,
            cash_app: each(&self.cash_app, cash_app)?,
            paypal: each(&self.paypal, paypal)?,
            zelle: each(&self.zelle, zelle)?,
        })
    }
}

fn is_name_char(c: char, extra: &[char]) -> bool {
    c.is_ascii_alphanumeric() || extra.contains(&c)
}

/// A Venmo username: 5 to 30 letters, digits, hyphens or underscores, as
/// Venmo allows. Typed with or without its `@`.
pub fn venmo(input: &str) -> Option<String> {
    let name = input.trim();
    let name = name.strip_prefix('@').unwrap_or(name);
    ((5..=30).contains(&name.len()) && name.chars().all(|c| is_name_char(c, &['-', '_'])))
        .then(|| name.to_owned())
}

/// A Cash App $Cashtag: 1 to 20 letters, digits or underscores, at least
/// one of them a letter, as Cash App allows. Typed with or without its `$`.
pub fn cash_app(input: &str) -> Option<String> {
    let tag = input.trim();
    let tag = tag.strip_prefix('$').unwrap_or(tag);
    ((1..=20).contains(&tag.len())
        && tag.chars().all(|c| is_name_char(c, &['_']))
        && tag.chars().any(|c| c.is_ascii_alphabetic()))
    .then(|| tag.to_owned())
}

/// A PayPal.Me name: 1 to 20 letters or digits, as PayPal allows. Typed
/// alone, or as the link (`paypal.me/name`).
pub fn paypal(input: &str) -> Option<String> {
    let name = input.trim();
    let lower = name.to_ascii_lowercase();
    let name = ["https://", "http://", ""]
        .iter()
        .flat_map(|scheme| ["www.paypal.me/", "paypal.me/"].map(|host| format!("{scheme}{host}")))
        .find(|prefix| lower.starts_with(prefix.as_str()))
        .map_or(name, |prefix| &name[prefix.len()..]);
    let name = name.trim_end_matches('/');
    ((1..=20).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| name.to_owned())
}

/// Where Zelle pays someone: an email address, lower-cased, or a US phone
/// number, as `+1` and ten digits. Zelle is for US bank accounts, so a
/// number elsewhere in the North American plan is refused.
pub fn zelle(input: &str) -> Option<String> {
    let text = input.trim();
    if text.contains('@') {
        return match crate::domain::identity::Identifier::parse(text) {
            Ok(crate::domain::identity::Identifier::Email(email)) => Some(email),
            _ => None,
        };
    }
    if !text
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '+' | ' ' | '-' | '(' | ')' | '.'))
        || text.matches('+').count() > 1
        || (text.contains('+') && !text.starts_with('+'))
    {
        return None;
    }
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    let national = match digits.len() {
        10 if !text.starts_with('+') => digits.as_str(),
        11 if digits.starts_with('1') => &digits[1..],
        _ => return None,
    };
    // An exchange code, like an area code, begins with 2 to 9.
    if !matches!(national.as_bytes()[3], b'2'..=b'9') {
        return None;
    }
    (crate::nanp::region(national) == Some(crate::nanp::Region::Us))
        .then(|| format!("+1{national}"))
}

/// The encrypted columns, in the order of [`PaymentHandles`]' fields.
pub const COLUMNS: [(&str, Field); 4] = [
    ("venmo_encrypted", Field::PAYMENT_VENMO),
    ("cash_app_encrypted", Field::PAYMENT_CASH_APP),
    ("paypal_encrypted", Field::PAYMENT_PAYPAL),
    ("zelle_encrypted", Field::PAYMENT_ZELLE),
];

type SealedRow = (
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
);

fn open(row: SealedRow) -> Result<PaymentHandles, Unreadable> {
    let keys = contact::keys();
    let (venmo, cash_app, paypal, zelle) = row;
    Ok(PaymentHandles {
        venmo: keys.reveal(Field::PAYMENT_VENMO, venmo.as_deref())?,
        cash_app: keys.reveal(Field::PAYMENT_CASH_APP, cash_app.as_deref())?,
        paypal: keys.reveal(Field::PAYMENT_PAYPAL, paypal.as_deref())?,
        zelle: keys.reveal(Field::PAYMENT_ZELLE, zelle.as_deref())?,
    })
}

/// The account's own payment options, decrypted for it.
pub async fn load(conn: &mut PgConnection, account: Uuid) -> Result<PaymentHandles, ApiError> {
    let row: Option<SealedRow> = sqlx::query_as(
        "SELECT venmo_encrypted, cash_app_encrypted, paypal_encrypted, zelle_encrypted
         FROM payment_handle WHERE account_id = $1",
    )
    .bind(account)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(open).transpose()?.unwrap_or_default())
}

/// Counts one change to the account's payment options against
/// [`WRITES_PER_HOUR`], making its row if it has none, and holds the row to
/// the end of the transaction. Counted even when the change is then
/// refused, so that a script that keeps asking stays out.
async fn count_write(conn: &mut PgConnection, account: Uuid) -> Result<(), ApiError> {
    let writes: i32 = sqlx::query_scalar(
        "INSERT INTO payment_handle AS h (account_id, writes_in_window)
         VALUES ($1, 1)
         ON CONFLICT (account_id) DO UPDATE SET
             writes_in_window = CASE
                 WHEN h.write_window_started_at > now() - interval '1 hour'
                 THEN h.writes_in_window + 1 ELSE 1 END,
             write_window_started_at = CASE
                 WHEN h.write_window_started_at > now() - interval '1 hour'
                 THEN h.write_window_started_at ELSE now() END
         RETURNING writes_in_window",
    )
    .bind(account)
    .fetch_one(&mut *conn)
    .await?;
    if writes > WRITES_PER_HOUR {
        return Err(ErrorCode::TooManyRequests.into());
    }
    Ok(())
}

/// Saves the account's payment options, replacing what it had: an option
/// left out is removed. With none left, every agreement it showed them on
/// stops showing them, so that options saved again later are not shown
/// anywhere until the person says so.
pub async fn save(
    conn: &mut PgConnection,
    account: Uuid,
    handles: &PaymentHandles,
) -> Result<PaymentHandles, ApiError> {
    let handles = handles.normalized()?;
    count_write(conn, account).await?;
    let keys = contact::keys();
    let seal = |field: Field, value: &Option<String>| {
        value.as_deref().map(|value| keys.seal(field, value))
    };
    sqlx::query(
        "UPDATE payment_handle
         SET venmo_encrypted = $2, cash_app_encrypted = $3, paypal_encrypted = $4,
             zelle_encrypted = $5, updated_at = now()
         WHERE account_id = $1",
    )
    .bind(account)
    .bind(seal(Field::PAYMENT_VENMO, &handles.venmo))
    .bind(seal(Field::PAYMENT_CASH_APP, &handles.cash_app))
    .bind(seal(Field::PAYMENT_PAYPAL, &handles.paypal))
    .bind(seal(Field::PAYMENT_ZELLE, &handles.zelle))
    .execute(&mut *conn)
    .await?;
    if handles.is_empty() {
        sqlx::query("DELETE FROM payment_offer WHERE account_id = $1")
            .bind(account)
            .execute(&mut *conn)
            .await?;
    }
    Ok(handles)
}

/// Removes all of the account's payment options, and stops showing them
/// on every agreement.
pub async fn remove(conn: &mut PgConnection, account: Uuid) -> Result<(), ApiError> {
    save(conn, account, &PaymentHandles::default()).await?;
    Ok(())
}

/// Shows the account's payment options on an agreement, or stops. The
/// caller has checked that the account is a party to it, and, to show
/// them, that the agreement is sent and not closed. Showing them needs at
/// least one saved; stopping always works.
pub async fn set_shown(
    conn: &mut PgConnection,
    account: Uuid,
    exchange: Uuid,
    on: bool,
) -> Result<(), ApiError> {
    count_write(conn, account).await?;
    if !on {
        sqlx::query("DELETE FROM payment_offer WHERE exchange_id = $1 AND account_id = $2")
            .bind(exchange)
            .bind(account)
            .execute(&mut *conn)
            .await?;
        return Ok(());
    }
    let any: bool = sqlx::query_scalar(
        "SELECT num_nonnulls(venmo_encrypted, cash_app_encrypted, paypal_encrypted,
                             zelle_encrypted) > 0
         FROM payment_handle WHERE account_id = $1",
    )
    .bind(account)
    .fetch_one(&mut *conn)
    .await?;
    if !any {
        return Err(ErrorCode::ActionNotAllowed.into());
    }
    sqlx::query(
        "INSERT INTO payment_offer (exchange_id, account_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(exchange)
    .bind(account)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Whether the account shows its payment options on the agreement.
pub async fn shown(
    conn: &mut PgConnection,
    account: Uuid,
    exchange: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM payment_offer WHERE exchange_id = $1 AND account_id = $2)",
    )
    .bind(exchange)
    .bind(account)
    .fetch_one(&mut *conn)
    .await
}

/// The payee's payment options as the person who owes them money on an
/// agreement sees them: only while the payee shows them on this agreement
/// and their account is active, and `None` when they have none. Whether the
/// viewer owes money still to be paid is the caller's to decide.
pub async fn for_payer(
    conn: &mut PgConnection,
    payee: Uuid,
    exchange: Uuid,
) -> Result<Option<PaymentHandles>, ApiError> {
    let row: Option<SealedRow> = sqlx::query_as(
        "SELECT h.venmo_encrypted, h.cash_app_encrypted, h.paypal_encrypted, h.zelle_encrypted
         FROM payment_offer o
         JOIN payment_handle h ON h.account_id = o.account_id
         JOIN account a ON a.id = o.account_id AND a.status = 'ACTIVE'
         WHERE o.exchange_id = $1 AND o.account_id = $2",
    )
    .bind(exchange)
    .bind(payee)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row
        .map(open)
        .transpose()?
        .filter(|handles| !handles.is_empty()))
}

/// Everything of an account's payment options, for deleting the account:
/// the options themselves and every agreement it showed them on.
pub async fn forget(conn: &mut PgConnection, account: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM payment_offer WHERE account_id = $1")
        .bind(account)
        .execute(&mut *conn)
        .await?;
    sqlx::query("DELETE FROM payment_handle WHERE account_id = $1")
        .bind(account)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_venmo_username_is_5_to_30_letters_digits_hyphens_or_underscores() {
        assert_eq!(venmo("@Dana-Fixes_1").as_deref(), Some("Dana-Fixes_1"));
        assert_eq!(venmo(" dana5 ").as_deref(), Some("dana5"));
        assert_eq!(venmo(&"a".repeat(30)).as_deref(), Some(&*"a".repeat(30)));
        for bad in [
            "dana",
            "@dan",
            &"a".repeat(31),
            "dana fixes",
            "dana.fixes",
            "dañafixes",
            "@@danafixes",
            "dana/fixes",
        ] {
            assert_eq!(venmo(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_cashtag_has_a_letter_and_at_most_20_characters() {
        assert_eq!(cash_app("$DanaFixes").as_deref(), Some("DanaFixes"));
        assert_eq!(cash_app("d").as_deref(), Some("d"));
        assert_eq!(cash_app("dana_99").as_deref(), Some("dana_99"));
        for bad in [
            "",
            "$",
            "12345",
            &"a".repeat(21),
            "dana-fixes",
            "dana fixes",
            "$$dana",
            "dana$",
        ] {
            assert_eq!(cash_app(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_paypal_me_name_is_letters_and_digits_and_may_be_given_as_its_link() {
        assert_eq!(paypal("DanaFixes").as_deref(), Some("DanaFixes"));
        assert_eq!(paypal("paypal.me/DanaFixes").as_deref(), Some("DanaFixes"));
        assert_eq!(
            paypal("https://www.PayPal.me/DanaFixes/").as_deref(),
            Some("DanaFixes")
        );
        for bad in [
            "",
            "dana_fixes",
            "dana-fixes",
            &"a".repeat(21),
            "paypal.me/",
            "https://evil.example/x",
        ] {
            assert_eq!(paypal(bad), None, "{bad}");
        }
    }

    #[test]
    fn zelle_takes_an_email_address_or_a_us_number() {
        assert_eq!(
            zelle(" Dana@Example.COM ").as_deref(),
            Some("dana@example.com")
        );
        assert_eq!(zelle("(202) 555-0142").as_deref(), Some("+12025550142"));
        assert_eq!(zelle("+1 202 555 0142").as_deref(), Some("+12025550142"));
        assert_eq!(zelle("1-202-555-0142").as_deref(), Some("+12025550142"));
        for bad in [
            "dana",
            "dana@",
            "202555014",
            "+44 20 7946 0958",
            // Canada and Jamaica share +1; Zelle is for US accounts.
            "+1 416 555 0142",
            "+1 876 555 0142",
            // An exchange code never begins with 0 or 1.
            "202 155 0142",
            "202+5550142",
        ] {
            assert_eq!(zelle(bad), None, "{bad}");
        }
    }

    #[test]
    fn options_are_normalized_together_and_empty_text_is_none() {
        let given = PaymentHandles {
            venmo: Some("@dana-fixes".into()),
            cash_app: Some("".into()),
            paypal: Some("  ".into()),
            zelle: Some("DANA@example.com".into()),
        };
        assert_eq!(
            given.normalized().unwrap(),
            PaymentHandles {
                venmo: Some("dana-fixes".into()),
                cash_app: None,
                paypal: None,
                zelle: Some("dana@example.com".into()),
            }
        );
        let bad = PaymentHandles {
            cash_app: Some("12".into()),
            ..PaymentHandles::default()
        };
        assert!(bad.normalized().is_err());
        assert!(PaymentHandles::default().normalized().unwrap().is_empty());
    }

    #[test]
    fn the_longest_of_each_fits_its_column() {
        // The bounds in migration 0026: what is stored is the text and
        // `contact::OVERHEAD` bytes.
        let keys = contact::Keys::first(
            &contact::KeyConfig::new(contact::Key::from_bytes([7; 32]), None).unwrap(),
        );
        let longest = [
            (Field::PAYMENT_VENMO, "a".repeat(30), 71),
            (Field::PAYMENT_CASH_APP, "a".repeat(20), 61),
            (Field::PAYMENT_PAYPAL, "a".repeat(20), 61),
            (
                Field::PAYMENT_ZELLE,
                format!("{}@example.com", "a".repeat(242)),
                295,
            ),
        ];
        for (field, value, most) in longest {
            assert_eq!(keys.seal(field, &value).len(), most, "{field:?}");
        }
        let shortest = [
            (Field::PAYMENT_VENMO, "abcde", 46),
            (Field::PAYMENT_CASH_APP, "a", 42),
            (Field::PAYMENT_PAYPAL, "a", 42),
            (Field::PAYMENT_ZELLE, "a@b.c", 46),
        ];
        for (field, value, least) in shortest {
            assert!(keys.seal(field, value).len() >= least, "{field:?}");
        }
    }

    #[test]
    fn an_option_does_not_decrypt_in_another_column() {
        let keys = contact::Keys::first(
            &contact::KeyConfig::new(contact::Key::from_bytes([7; 32]), None).unwrap(),
        );
        let sealed = keys.seal(Field::PAYMENT_VENMO, "dana-fixes");
        for (_, other) in COLUMNS.iter().skip(1) {
            assert_eq!(keys.open(*other, &sealed), Err(Unreadable), "{other:?}");
        }
        assert_eq!(keys.open(Field::ACCOUNT_EMAIL, &sealed), Err(Unreadable));
        assert_eq!(
            keys.open(Field::PAYMENT_VENMO, &sealed).unwrap(),
            "dana-fixes"
        );
    }
}
