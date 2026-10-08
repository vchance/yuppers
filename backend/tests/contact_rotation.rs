//! Rotating `CONTACT_DATA_KEY` (`contact-data rotate`; docs/operations.md,
//! "Contact data key"). In a database of its own: rotating re-encrypts
//! everything in it, which the other tests, running under the test key,
//! could no longer read.

mod common;

use common::App;
use uuid::Uuid;
use yuppers_backend::contact::{Field, Key, KeyConfig, Keys, Kind, store};

const DATABASE: &str = "yuppers_test_contact_rotation";

/// The key rotated to, and one nobody configured.
const NEW_KEY: &str = "qnUFPSCP0Eaq/15YoDxxR4U/vCoOs0tC21Sgoz3d1L0=";
const STRAY_KEY: &str = "d98ZNeqrDB+QUJJTTOLnAbu0WeMrckWgggXNkns8ea0=";

fn key(text: &str) -> Key {
    Key::parse("CONTACT_DATA_KEY", text).unwrap()
}

/// Runs `contact-data` as the owner would, with these keys.
fn contact_data(app: &App, args: &[&str], current: &str, previous: &str) -> (bool, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_contact-data"))
        .args(args)
        .env("MIGRATION_DATABASE_URL", &app.owner_url)
        .env("CONTACT_DATA_KEY", current)
        .env("CONTACT_DATA_KEY_PREVIOUS", previous)
        .output()
        .unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

/// The first byte of every value in every encrypted column: the key it is
/// under.
async fn key_ids(app: &App) -> Vec<i32> {
    let mut ids = Vec::new();
    for (table, column) in store::ENCRYPTED_COLUMNS {
        let found: Vec<i32> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT DISTINCT get_byte({column}, 0) FROM {table} WHERE {column} IS NOT NULL"
        )))
        .fetch_all(&app.owner)
        .await
        .unwrap();
        ids.extend(found);
    }
    ids.sort();
    ids.dedup();
    ids
}

#[tokio::test]
async fn rotating_re_encrypts_everything_and_leaves_every_index_as_it_was() {
    let app = App::start(DATABASE).await;
    let old = Keys::first(&common::contact_config());
    // An account with an address and a number, and records of consent with
    // a number.
    let ana = app.user("Ana").await;
    let phone = format!("+1999{:07}", Uuid::new_v4().as_u128() % 10_000_000);
    common::set_phone(&app.owner, ana.id, &phone, false).await;
    // A record's number is bound to its ID, taken first as the service does.
    let consent_id: i64 = sqlx::query_scalar("SELECT nextval('sms_consent_id_seq')")
        .fetch_one(&app.owner)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sms_consent (id, action, phone_encrypted, phone_index, source, keyword)
         OVERRIDING SYSTEM VALUE
         VALUES ($3, 'STOP', $1, $2, 'SMS_REPLY', 'STOP')",
    )
    .bind(old.seal(Field::SMS_CONSENT_PHONE.row(consent_id), &phone))
    .bind(old.index(Kind::Phone, &phone).as_slice())
    .bind(consent_id)
    .execute(&app.owner)
    .await
    .unwrap();
    let code_consent_id: i64 = sqlx::query_scalar("SELECT nextval('sms_code_consent_id_seq')")
        .fetch_one(&app.owner)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sms_code_consent (id, purpose, account_id, phone_encrypted, phone_hash,
                                       source, consent_version, consent_language)
         OVERRIDING SYSTEM VALUE
         VALUES ($3, 'SIGN_IN', $1, $2, sha256('x'), 'WEB', 'v', 'en')",
    )
    .bind(ana.id)
    .bind(old.seal(Field::SMS_CODE_CONSENT_PHONE.row(code_consent_id), &phone))
    .bind(code_consent_id)
    .execute(&app.owner)
    .await
    .unwrap();
    // Payment options, encrypted under the same key (`payments`).
    sqlx::query(
        "INSERT INTO payment_handle (account_id, venmo_encrypted, zelle_encrypted)
         VALUES ($1, $2, $3)",
    )
    .bind(ana.id)
    .bind(old.seal(Field::PAYMENT_VENMO, "ana-pays"))
    .bind(old.seal(Field::PAYMENT_ZELLE, &phone))
    .execute(&app.owner)
    .await
    .unwrap();
    let index_before: Vec<u8> = sqlx::query_scalar("SELECT email_index FROM account WHERE id = $1")
        .bind(ana.id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(key_ids(&app).await, [i32::from(old.current_id())]);

    // Before the rotation: the new key alone does not open the database.
    let new_alone = KeyConfig::new(key(NEW_KEY), None).unwrap();
    assert!(matches!(
        store::open(&app.owner, &new_alone).await,
        Err(store::OpenError::WrongKey)
    ));

    // The status says where things stand, under the current key.
    let (ok, said) = contact_data(&app, &["status"], common::CONTACT_DATA_KEY, "");
    assert!(ok, "{said}");
    assert!(said.contains("account.email_encrypted"), "{said}");
    assert!(said.contains("not set, and not needed"), "{said}");

    // Rotated, with the old key as the previous one.
    let (ok, said) = contact_data(&app, &["rotate"], NEW_KEY, common::CONTACT_DATA_KEY);
    assert!(ok, "{said}");
    assert!(said.contains("index key: re-encrypted"), "{said}");
    assert!(
        said.contains("CONTACT_DATA_KEY_PREVIOUS can be removed"),
        "{said}"
    );
    assert!(!said.contains(NEW_KEY) && !said.contains(common::CONTACT_DATA_KEY));

    // Everything is under the new key, which alone now opens the database
    // and reads every value; the old key alone opens nothing.
    let new = store::open(&app.owner, &new_alone).await.unwrap();
    assert_eq!(key_ids(&app).await, [i32::from(new.current_id())]);
    assert!(matches!(
        store::open(&app.owner, &common::contact_config()).await,
        Err(store::OpenError::WrongKey)
    ));
    let (email, number): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT email_encrypted, phone_encrypted FROM account WHERE id = $1")
            .bind(ana.id)
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(new.open(Field::ACCOUNT_EMAIL, &email).unwrap(), ana.email);
    assert_eq!(new.open(Field::ACCOUNT_PHONE, &number).unwrap(), phone);
    let records: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT phone_encrypted FROM sms_consent WHERE phone_index = $1
         UNION ALL
         SELECT phone_encrypted FROM sms_code_consent WHERE account_id = $2",
    )
    .bind(old.index(Kind::Phone, &phone).as_slice())
    .bind(ana.id)
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(
        new.open(Field::SMS_CONSENT_PHONE.row(consent_id), &records[0])
            .unwrap(),
        phone
    );
    assert_eq!(
        new.open(
            Field::SMS_CODE_CONSENT_PHONE.row(code_consent_id),
            &records[1]
        )
        .unwrap(),
        phone
    );
    assert!(old.open(Field::ACCOUNT_EMAIL, &email).is_err());
    let (venmo, zelle): (Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT venmo_encrypted, zelle_encrypted FROM payment_handle WHERE account_id = $1",
    )
    .bind(ana.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(new.open(Field::PAYMENT_VENMO, &venmo).unwrap(), "ana-pays");
    assert_eq!(new.open(Field::PAYMENT_ZELLE, &zelle).unwrap(), phone);
    assert!(said.contains("payment_handle.venmo_encrypted"), "{said}");

    // No index moved: what was found before is found the same way.
    let index_after: Vec<u8> = sqlx::query_scalar("SELECT email_index FROM account WHERE id = $1")
        .bind(ana.id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    assert_eq!(index_after, index_before);
    assert_eq!(new.index(Kind::Email, &ana.email).to_vec(), index_before);

    // Again: nothing left to re-encrypt.
    let (ok, said) = contact_data(&app, &["rotate"], NEW_KEY, common::CONTACT_DATA_KEY);
    assert!(ok, "{said}");
    assert!(
        said.contains("index key: already under the current key"),
        "{said}"
    );

    // A value under a key nobody configured is left, counted, and keeps
    // the service from starting until it is dealt with.
    let stray = Keys::first(&KeyConfig::new(key(STRAY_KEY), None).unwrap());
    sqlx::query("UPDATE sms_code_consent SET phone_encrypted = $1 WHERE account_id = $2")
        .bind(stray.seal(Field::SMS_CODE_CONSENT_PHONE.row(code_consent_id), &phone))
        .bind(ana.id)
        .execute(&app.owner)
        .await
        .unwrap();
    let (ok, said) = contact_data(&app, &["rotate"], NEW_KEY, common::CONTACT_DATA_KEY);
    assert!(!ok, "{said}");
    assert!(said.contains("1 values decrypt under neither"), "{said}");
    assert!(matches!(
        store::open(&app.owner, &new_alone).await,
        Err(store::OpenError::UnknownKey {
            table: "sms_code_consent",
            ..
        })
    ));
}
