//! Email addresses and phone numbers at rest (`yuppers_backend::contact`,
//! migration 0025): every value written sealed and never in plaintext, the
//! blind index's uniqueness, the associated data (column and record), the
//! records of consent the service's role may not change, the migration's
//! refusal to run on a database that holds rows, and the refusal to start
//! without the right key. The scan of the whole database for any address or
//! number has a database of its own (`tests/contact_scan.rs`), and so does
//! rotating the key (`tests/contact_rotation.rs`).

mod common;

use axum::http::StatusCode;
use common::App;
use common::texting::{Texting, address, number, texting};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use yuppers_backend::contact::{self, Field, Key, KeyConfig, Keys, Kind, store};
use yuppers_backend::db;

const DATABASE: &str = "yuppers_test_contact";

/// One test at a time: they share one database.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn start() -> (Texting, tokio::sync::MutexGuard<'static, ()>) {
    let turn = TURN.lock().await;
    (texting(DATABASE).await, turn)
}

/// The service writes every address and number sealed: the account's
/// encrypted copy opens to it, it is found by its index, and each record of
/// consent's number is bound to its row. No column for the plaintext is
/// left in any table.
#[tokio::test]
async fn every_value_is_written_sealed_and_no_plaintext_column_is_left() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let keys = contact::keys();
    let needles = test.whole_life().await;
    let (ana_email, ana_phone) = (&needles[0], &needles[3]);

    let encrypted: Vec<u8> =
        sqlx::query_scalar("SELECT email_encrypted FROM account WHERE email_index = $1")
            .bind(common::index(ana_email))
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(
        keys.open(Field::ACCOUNT_EMAIL, &encrypted).unwrap(),
        *ana_email
    );

    // Found where the service looks, by index.
    for (what, query) in [
        (
            "the account by its number",
            "SELECT count(*) FROM account WHERE phone_index = $1",
        ),
        (
            "the opt-out",
            "SELECT count(*) FROM sms_opt_out WHERE phone_index = $1",
        ),
        (
            "the records of consent",
            "SELECT count(*) FROM sms_consent WHERE phone_index = $1",
        ),
    ] {
        let found: i64 = sqlx::query_scalar(query)
            .bind(common::index(ana_phone))
            .fetch_one(&app.owner)
            .await
            .unwrap();
        assert!(found > 0, "{what}");
    }
    // Every record of consent with its number encrypted is bound to its row.
    let records: Vec<(i64, Option<Vec<u8>>)> =
        sqlx::query_as("SELECT id, phone_encrypted FROM sms_consent WHERE phone_index = $1")
            .bind(common::index(ana_phone))
            .fetch_all(&app.owner)
            .await
            .unwrap();
    assert!(records.iter().any(|(_, encrypted)| encrypted.is_some()));
    for (id, encrypted) in records {
        if let Some(encrypted) = encrypted {
            assert_eq!(
                keys.open(Field::SMS_CONSENT_PHONE.row(id), &encrypted)
                    .unwrap(),
                *ana_phone
            );
            assert!(
                keys.open(Field::SMS_CONSENT_PHONE.row(id + 1), &encrypted)
                    .is_err()
            );
        }
    }

    // No column a plaintext address or number could be in.
    let left: Vec<String> = sqlx::query_scalar(
        "SELECT table_name || '.' || column_name FROM information_schema.columns
         WHERE table_schema = 'public'
           AND column_name IN ('email', 'phone', 'identifier', 'bound_email', 'bound_phone')",
    )
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(left, Vec::<String>::new());
}

#[tokio::test]
async fn an_address_is_one_account_whatever_its_case() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let email = address();
    let first = test.sign_in(&email.to_uppercase(), "Ana").await;
    let again = test.sign_in(&email, "Ana").await;
    assert_eq!(first.id, again.id, "the same account");

    // Nobody else may add it, in any case.
    let ben = test.sign_in(&address(), "Ben").await;
    test.ask(Some(&ben), &email).await;
    let code = test.code(&email);
    app.post(
        &ben,
        "/v1/me/identifiers",
        json!({ "identifier": email.to_uppercase(), "code": code }),
    )
    .await
    .refused(StatusCode::CONFLICT, "IDENTIFIER_IN_USE");

    // And the database itself refuses a second account with its index.
    let sealed = common::sealed(&email);
    let refused = sqlx::query(
        "INSERT INTO account (email_encrypted, email_index, display_name) VALUES ($1, $2, '')",
    )
    .bind(&sealed.encrypted)
    .bind(sealed.index.as_slice())
    .execute(&app.db)
    .await
    .unwrap_err();
    assert_eq!(
        refused.as_database_error().unwrap().code().as_deref(),
        Some("23505")
    );
}

#[tokio::test]
async fn a_value_moved_to_another_column_does_not_decrypt_and_nothing_is_guessed() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let ana = app.user("Ana").await;
    let ben = app.user("Ben").await;
    common::set_phone(&app.owner, ana.id, &number(), false).await;
    assert_eq!(app.get(&ben, "/v1/me").await.ok()["email"], ben.email);

    // Ana's encrypted number put where Ben's address is: the bytes are a
    // valid ciphertext under the right key, but of another column.
    sqlx::query(
        "UPDATE account SET email_encrypted = (SELECT phone_encrypted FROM account WHERE id = $1)
         WHERE id = $2",
    )
    .bind(ana.id)
    .bind(ben.id)
    .execute(&app.owner)
    .await
    .unwrap();
    app.get(&ben, "/v1/me")
        .await
        .refused(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL");
    // Ana's own still reads.
    assert!(app.get(&ana, "/v1/me").await.ok()["phone"].is_string());
}

/// The service's role may add records of consent and remove them past
/// their retention, never change one; and a ciphertext copied from one
/// record to another does not decrypt there.
#[tokio::test]
async fn a_record_of_consent_is_bound_to_its_row_and_never_changed_by_the_service() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let keys = contact::keys();
    let (one, other) = (number(), number());
    let mut ids = Vec::new();
    for phone in [&one, &other] {
        let id: i64 = sqlx::query_scalar("SELECT nextval('sms_consent_id_seq')")
            .fetch_one(&app.db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO sms_consent (id, action, phone_encrypted, phone_index, source, keyword)
             OVERRIDING SYSTEM VALUE
             VALUES ($1, 'STOP', $2, $3, 'SMS_REPLY', 'STOP')",
        )
        .bind(id)
        .bind(keys.seal(Field::SMS_CONSENT_PHONE.row(id), phone))
        .bind(common::index(phone))
        .execute(&app.db)
        .await
        .unwrap();
        ids.push(id);
    }
    // The first's ciphertext does not decrypt as the second's, so a copy is
    // caught, not believed.
    let copied: Vec<u8> =
        sqlx::query_scalar("SELECT phone_encrypted FROM sms_consent WHERE id = $1")
            .bind(ids[0])
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(
        keys.open(Field::SMS_CONSENT_PHONE.row(ids[0]), &copied)
            .unwrap(),
        one
    );
    assert!(
        keys.open(Field::SMS_CONSENT_PHONE.row(ids[1]), &copied)
            .is_err()
    );
    for statement in [
        "UPDATE sms_consent SET phone_encrypted = NULL WHERE id = $1",
        "UPDATE sms_consent SET phone_index = phone_index WHERE id = $1",
        "UPDATE sms_code_consent SET phone_encrypted = NULL WHERE id = $1",
    ] {
        let refused = sqlx::query(statement)
            .bind(ids[0])
            .execute(&app.db)
            .await
            .unwrap_err();
        assert_eq!(
            refused.as_database_error().unwrap().code().as_deref(),
            Some("42501"),
            "{statement}"
        );
    }
    sqlx::query("DELETE FROM sms_consent WHERE id = ANY($1)")
        .bind(&ids)
        .execute(&app.owner)
        .await
        .unwrap();
}

/// Migration 0025 drops the plaintext columns with what they hold, so on a
/// database where any of its tables has a row it refuses, changing nothing.
#[tokio::test]
async fn the_migration_refuses_a_database_that_holds_rows_and_changes_nothing() {
    let (test, _turn) = start().await;
    let owner_url = &test.app.owner_url;
    let name = format!("{DATABASE}_populated_{}", std::process::id());
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(owner_url)
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&admin)
        .await
        .unwrap();
    let (base, _) = owner_url.rsplit_once('/').unwrap();
    let populated = PgPoolOptions::new()
        .max_connections(1)
        .connect(&format!("{base}/{name}"))
        .await
        .unwrap();
    db::MIGRATOR.run_to(24, &populated).await.unwrap();
    // One account from before: an address in plaintext.
    sqlx::query("INSERT INTO account (email, display_name) VALUES ('ana@example.test', 'Ana')")
        .execute(&populated)
        .await
        .unwrap();

    let refused = db::MIGRATOR.run(&populated).await.unwrap_err();
    let said = refused.to_string();
    assert!(said.contains("Reset the database first"), "{said}");
    assert!(said.contains("account holds rows"), "{said}");
    // Nothing changed: the address is where it was, and 0025 is not applied.
    let email: String = sqlx::query_scalar("SELECT email FROM account")
        .fetch_one(&populated)
        .await
        .unwrap();
    assert_eq!(email, "ana@example.test");
    let applied: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&populated)
            .await
            .unwrap();
    assert_eq!(applied, 24);
    let key_table: bool = sqlx::query_scalar("SELECT to_regclass('contact_key') IS NOT NULL")
        .fetch_one(&populated)
        .await
        .unwrap();
    assert!(!key_table);

    populated.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .unwrap();
}

/// The api, the worker and migrate, run against the test database.
fn run(binary: &str, app: &App, key: Option<&str>, origin: &str) -> (bool, String) {
    let mut command = std::process::Command::new(binary);
    command
        .env("DATABASE_URL", &app.app_url)
        .env("MIGRATION_DATABASE_URL", &app.owner_url)
        .env("BIND_ADDR", "127.0.0.1:0")
        .env("WEB_ORIGIN", origin)
        .env("APP_SECRET", "contact-test-secret-0123456789abcdef")
        .env("CODE_DELIVERY", "log")
        .env("NOTIFICATION_DELIVERY", "log")
        .env("METRICS_ADDR", "")
        .env("WEB_DIR", "")
        .env("NO_COLOR", "1")
        // Set to nothing, which counts as not set, so that a .env cannot
        // give one.
        .env("CONTACT_DATA_KEY", key.unwrap_or(""))
        .env("CONTACT_DATA_KEY_PREVIOUS", "");
    let output = command.output().unwrap();
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), text)
}

#[tokio::test]
async fn nothing_starts_without_the_databases_contact_data_key() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let local = "http://127.0.0.1";
    let binaries = [
        env!("CARGO_BIN_EXE_api"),
        env!("CARGO_BIN_EXE_worker"),
        env!("CARGO_BIN_EXE_migrate"),
    ];
    for binary in binaries {
        let (started, said) = run(binary, app, None, local);
        assert!(!started, "{binary}: {said}");
        assert!(
            said.contains("CONTACT_DATA_KEY is not set"),
            "{binary}: {said}"
        );
        let (started, said) = run(binary, app, Some("not a key"), local);
        assert!(!started, "{binary}: {said}");
        assert!(
            said.contains("CONTACT_DATA_KEY is not base64"),
            "{binary}: {said}"
        );
        // The test key is published in the repository: refused for any
        // origin but this machine.
        let (started, said) = run(
            binary,
            app,
            Some(common::CONTACT_DATA_KEY),
            "https://yuppers.example",
        );
        assert!(!started, "{binary}: {said}");
        assert!(
            said.contains("published in the repository"),
            "{binary}: {said}"
        );
        assert!(!said.contains(common::CONTACT_DATA_KEY), "{binary}: {said}");
    }
    // A well-formed key that is not this database's.
    let other = "sX64RYIT8oRmSvwjZfGYoApyO+3+T3EqO1ZQOvRKSNo=";
    for binary in &binaries[..2] {
        let (started, said) = run(binary, app, Some(other), local);
        assert!(!started, "{binary}: {said}");
        assert!(
            said.contains("does not decrypt this database's contact data"),
            "{binary}: {said}"
        );
        assert!(!said.contains(other), "the key is never written: {said}");
    }
    let wrong = KeyConfig::new(Key::parse("CONTACT_DATA_KEY", other).unwrap(), None).unwrap();
    assert!(matches!(
        store::open(&app.db, &wrong).await,
        Err(store::OpenError::WrongKey)
    ));
    // The right one opens it.
    let keys: Keys = store::open(&app.db, &common::contact_config())
        .await
        .unwrap();
    assert_eq!(
        keys.index(Kind::Email, "a@b.test"),
        contact::keys().index(Kind::Email, "a@b.test")
    );
}
