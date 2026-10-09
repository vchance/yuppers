//! Adding, changing and removing an email address or phone number, and
//! combining two accounts (`yuppers_backend::combine`, migration 0028):
//! what moves, what stays as written, every refusal, retries and races, the
//! deletion log and its replay, and invitations sent to another address.

mod common;

use axum::http::{Method, StatusCode};
use common::texting::{Texting, address, number, texting};
use common::{Reply, User, accept, fence_job};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;
use yuppers_backend::combine::{self, IdentifierKind};
use yuppers_backend::deletion;
use yuppers_backend::deletion_log;
use yuppers_backend::notifications::sms_updates::CONSENT_VERSION;

const DATABASE: &str = "yuppers_test_combine";

/// One test at a time: the codes they read are kept per process.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn start() -> (Texting, tokio::sync::MutexGuard<'static, ()>) {
    let turn = TURN.lock().await;
    (texting(DATABASE).await, turn)
}

async fn scalar_i64(test: &Texting, query: &'static str, account: Uuid) -> i64 {
    sqlx::query_scalar(query)
        .bind(account)
        .fetch_one(&test.app.owner)
        .await
        .unwrap()
}

async fn status(test: &Texting, account: Uuid) -> (String, Option<Uuid>) {
    sqlx::query_as("SELECT status, merged_into FROM account WHERE id = $1")
        .bind(account)
        .fetch_one(&test.app.owner)
        .await
        .unwrap()
}

/// Proves `identifier` for `user`: asks for its code and enters it, with a
/// proof of the user's own where the user has one of that kind already.
async fn prove(test: &Texting, user: &User, identifier: &str) -> Reply {
    let me = test.app.get(user, "/v1/me").await.ok();
    let kind = if identifier.contains('@') {
        "email"
    } else {
        "phone"
    };
    let proof = if me[kind].is_string() {
        Some(own_proof(test, user).await)
    } else {
        None
    };
    test.ask(Some(user), identifier).await;
    test.app
        .post(
            user,
            "/v1/me/identifiers",
            json!({ "identifier": identifier, "code": test.code(identifier), "proof": proof }),
        )
        .await
}

/// The offer a proof of another account's identifier gives.
async fn offer(test: &Texting, user: &User, identifier: &str) -> Value {
    let reply = prove(test, user, identifier).await;
    reply.refused(StatusCode::CONFLICT, "IDENTIFIER_ON_OTHER_ACCOUNT");
    reply.body["combine"].clone()
}

async fn combine_with(test: &Texting, user: &User, token: &Value, key: &str) -> Reply {
    combine_proved(test, user, token, None, key).await
}

async fn combine_proved(
    test: &Texting,
    user: &User,
    token: &Value,
    proof: Option<&str>,
    key: &str,
) -> Reply {
    test.app
        .call(
            Some(user),
            Method::POST,
            "/v1/me/combine",
            Some(json!({ "token": token, "proof": proof })),
            &[("idempotency-key", key)],
        )
        .await
}

/// Takes up the offer, with a proof of the user's own email address where
/// the offer needs one.
async fn accept_offer(test: &Texting, user: &User, offered: &Value, key: &str) -> Reply {
    let proof = if offered["proof_required"] == true {
        Some(own_proof(test, user).await)
    } else {
        None
    };
    combine_proved(test, user, &offered["token"], proof.as_deref(), key).await
}

/// A proof of the user's own email address (or, without one, phone
/// number), from a code sent to it.
async fn own_proof(test: &Texting, user: &User) -> String {
    let me = test.app.get(user, "/v1/me").await.ok();
    let (channel, own) = match me["email"].as_str() {
        Some(email) => ("EMAIL", email.to_owned()),
        None => ("PHONE", me["phone"].as_str().unwrap().to_owned()),
    };
    proof_by(test, user, channel, &own).await
}

/// A proof of `own`, the user's identifier of `channel`.
async fn proof_by(test: &Texting, user: &User, channel: &str, own: &str) -> String {
    test.ask(Some(user), own).await;
    let proved = test
        .app
        .post(
            user,
            "/v1/me/identifiers/proof",
            json!({ "channel": channel, "code": test.code(own) }),
        )
        .await
        .ok();
    proved["proof"].as_str().unwrap().to_owned()
}

/// Replaces the user's identifier of that kind with `identifier`, with its
/// code and a proof of the user's own.
async fn replace(test: &Texting, user: &User, identifier: &str) -> Reply {
    let proof = own_proof(test, user).await;
    test.ask(Some(user), identifier).await;
    test.app
        .post(
            user,
            "/v1/me/identifiers",
            json!({ "identifier": identifier, "code": test.code(identifier), "proof": proof }),
        )
        .await
}

/// The email notices queued for `account`: what each says, to whom, and in
/// which language, decrypted.
async fn notices(test: &Texting, account: Uuid) -> Vec<(String, String, String)> {
    let rows: Vec<(i64, String, Vec<u8>, String)> = sqlx::query_as(
        "SELECT id, kind, email_encrypted, language FROM account_notice
         WHERE account_id = $1 ORDER BY id",
    )
    .bind(account)
    .fetch_all(&test.app.owner)
    .await
    .unwrap();
    rows.into_iter()
        .map(|(id, kind, email, language)| {
            let email = common::open(
                yuppers_backend::contact::Field::ACCOUNT_NOTICE_EMAIL.row(id),
                &email,
            );
            (kind, email, language)
        })
        .collect()
}

/// The notice the app shows on the account, if any.
async fn in_app(test: &Texting, user: &User) -> Value {
    test.app.get(user, "/v1/me").await.ok()["notice"]["kind"].clone()
}

async fn remove(test: &Texting, user: &User, kind: &str, code: &str, key: &str) -> Reply {
    test.app
        .call(
            Some(user),
            Method::DELETE,
            &format!("/v1/me/identifiers/{kind}"),
            Some(json!({ "code": code })),
            &[("idempotency-key", key)],
        )
        .await
}

fn key() -> String {
    Uuid::new_v4().to_string()
}

// ---- Adding, changing, removing ----------------------------------------------

#[tokio::test]
async fn an_identifier_is_added_changed_and_removed_with_codes_and_one_always_stays() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let email = address();
    let ana = test.sign_in(&email, "Ana").await;

    // The only identifier cannot go, whatever the code.
    remove(&test, &ana, "email", "000000", &key())
        .await
        .refused(StatusCode::CONFLICT, "LAST_IDENTIFIER");

    // Add a phone; turn text updates on for an agreement.
    let phone = number();
    assert_eq!(test.add(&ana, &phone).await["phone"], phone);
    let other = test.sign_in(&address(), "Ben").await;
    let deal = app.active_between(ana.clone(), other).await;
    let path = format!("/v1/exchanges/{}/sms-updates", deal.exchange);
    let on = json!({ "on": true, "consent": { "version": CONSENT_VERSION, "language": "en" } });
    app.call(Some(&ana), Method::PUT, &path, Some(on), &[])
        .await
        .ok();

    // Changing the email takes a proof of one of the account's own: without
    // one it is refused before its code is looked at.
    let changed = address();
    test.ask(Some(&ana), &changed).await;
    let code = test.code(&changed);
    app.post(
        &ana,
        "/v1/me/identifiers",
        json!({ "identifier": changed, "code": code }),
    )
    .await
    .refused(StatusCode::CONFLICT, "PROOF_REQUIRED");
    // A proof from a code to the phone; another account's is no good.
    let stranger = test.sign_in(&address(), "Sam").await;
    let theirs = own_proof(&test, &stranger).await;
    app.post(
        &ana,
        "/v1/me/identifiers",
        json!({ "identifier": changed, "code": code, "proof": theirs }),
    )
    .await
    .refused(StatusCode::CONFLICT, "PROOF_REQUIRED");
    assert_eq!(app.get(&ana, "/v1/me").await.ok()["email"], email);
    let proof = proof_by(&test, &ana, "PHONE", &phone).await;
    test.ask(Some(&ana), &changed).await;
    let me = app
        .post(
            &ana,
            "/v1/me/identifiers",
            json!({ "identifier": changed, "code": test.code(&changed), "proof": proof }),
        )
        .await
        .ok();
    assert_eq!(me["email"], changed);
    // The address replaced is told, by email, in the account's language;
    // nothing is shown in the app for it.
    assert_eq!(
        notices(&test, ana.id).await,
        vec![("EMAIL_CHANGED".to_owned(), email.clone(), "en".to_owned())]
    );
    assert!(in_app(&test, &ana).await.is_null());
    // The proof was good once.
    let again = address();
    test.ask(Some(&ana), &again).await;
    app.post(
        &ana,
        "/v1/me/identifiers",
        json!({ "identifier": again, "code": test.code(&again), "proof": proof }),
    )
    .await
    .refused(StatusCode::CONFLICT, "PROOF_REQUIRED");

    // Removing the phone needs a code sent to the email that stays: a code
    // sent to the phone itself, or a wrong one, is refused.
    test.ask(Some(&ana), &phone).await;
    remove(&test, &ana, "phone", &test.code(&phone), &key())
        .await
        .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    test.ask(Some(&ana), &changed).await;
    let removing = key();
    let me = remove(&test, &ana, "phone", &test.code(&changed), &removing)
        .await
        .ok();
    assert_eq!(
        (me["email"].clone(), me["phone"].clone()),
        (json!(changed), Value::Null)
    );
    // A repeat, with the same key or without the code it spent, changes
    // nothing and succeeds.
    remove(&test, &ana, "phone", &test.code(&changed), &removing)
        .await
        .ok();
    remove(&test, &ana, "phone", "123456", &key()).await.ok();

    // Its text updates ended, with the ending recorded as any other.
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM sms_update WHERE account_id = $1",
            ana.id
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM sms_consent
             WHERE account_id = $1 AND action = 'OPT_OUT' AND source = 'PHONE_REMOVED'",
            ana.id
        )
        .await,
        1
    );
    assert_eq!(app.get(&ana, &path).await.ok()["on"], false);
    // A number is not texted about it: the app shows it, until dismissed.
    assert_eq!(in_app(&test, &ana).await, "PHONE_REMOVED");
    let me = app
        .call(
            Some(&ana),
            Method::PATCH,
            "/v1/me",
            Some(json!({ "dismiss_notice": true })),
            &[],
        )
        .await
        .ok();
    assert!(me["notice"].is_null(), "{me}");

    // Now the email is the only one again.
    remove(&test, &ana, "email", "000000", &key())
        .await
        .refused(StatusCode::CONFLICT, "LAST_IDENTIFIER");
    // The number removed is free: someone else can sign up with it.
    let fresh = test.sign_in(&phone, "Cleo").await;
    assert_ne!(fresh.id, ana.id);
}

#[tokio::test]
async fn removing_the_email_is_proved_by_a_code_to_the_phone_and_leaves_invitations_alone() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let phone = number();
    let ben = test.sign_in(&phone, "Ben").await;
    let email = address();
    test.add(&ben, &email).await;

    // Someone invites Ben by his email address.
    let ana = test.sign_in(&address(), "Ana").await;
    let exchange = app.draft(&ana).await;
    let sent = app
        .post(
            &ana,
            &format!("/v1/exchanges/{exchange}/revisions"),
            json!({
                "expected_version": 0,
                "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
                "consent": common::consent(),
                "invitation": { "bound_to": email },
            }),
        )
        .await
        .ok();
    let token = sent["invitation_token"].as_str().unwrap().to_owned();

    test.ask(Some(&ben), &phone).await;
    let me = remove(&test, &ben, "email", &test.code(&phone), &key())
        .await
        .ok();
    assert_eq!(me["email"], Value::Null);
    // The address removed is told, by email.
    assert_eq!(
        notices(&test, ben.id).await,
        vec![("EMAIL_REMOVED".to_owned(), email.clone(), "en".to_owned())]
    );

    // The invitation still names the address, and is not Ben's to take now;
    // he is told only that it went to an email address, to type it back.
    let (bound, revoked): (bool, bool) = sqlx::query_as(
        "SELECT bound_email_index IS NOT NULL, revoked_at IS NOT NULL FROM invitation
         WHERE exchange_id = $1",
    )
    .bind(Uuid::parse_str(&exchange).unwrap())
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!((bound, revoked), (true, false));
    let preview = app
        .post(&ben, "/v1/invitations/preview", json!({ "token": token }))
        .await
        .ok();
    assert_eq!(
        preview["sent_to"],
        json!({ "kind": "EMAIL", "replaces": false })
    );
}

#[tokio::test]
async fn changing_the_phone_takes_a_proof_and_is_shown_in_the_app_not_texted() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let phone = number();
    let ana = test.sign_in(&phone, "Ana").await;
    // Adding a kind the account has none of takes no proof.
    let email = address();
    assert_eq!(test.add(&ana, &email).await["email"], email);
    assert!(notices(&test, ana.id).await.is_empty());

    // Replacing the number: without a proof, refused before the code; with
    // one from a code to the email, done.
    let changed = number();
    test.ask(Some(&ana), &changed).await;
    app.post(
        &ana,
        "/v1/me/identifiers",
        json!({ "identifier": changed, "code": test.code(&changed) }),
    )
    .await
    .refused(StatusCode::CONFLICT, "PROOF_REQUIRED");
    assert_eq!(replace(&test, &ana, &changed).await.ok()["phone"], changed);
    // The number replaced is not texted; the app shows it, and no email
    // goes about a number.
    assert_eq!(in_app(&test, &ana).await, "PHONE_CHANGED");
    assert!(notices(&test, ana.id).await.is_empty());
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM outbox WHERE recipient_account_id = $1 AND kind = 'SMS'",
            ana.id
        )
        .await,
        0
    );
    // The same number again is no change, and takes no proof.
    test.ask(Some(&ana), &changed).await;
    app.post(
        &ana,
        "/v1/me/identifiers",
        json!({ "identifier": changed, "code": test.code(&changed) }),
    )
    .await
    .ok();
}

// ---- Combining --------------------------------------------------------------

/// Ben's account, by phone and email, with a yup in force with Cleo, text
/// updates on for it, payment options shown on it, and a device; and an
/// offer to Ana, signed in by email alone, to take it over.
struct Setup {
    ana: User,
    ben: User,
    cleo: User,
    ben_phone: String,
    ben_email: String,
    exchange: String,
    repair: Uuid,
    revision: String,
}

async fn setup(test: &Texting) -> Setup {
    let app = &test.app;
    let ben_phone = number();
    let ben = test.sign_in(&ben_phone, "Ben").await;
    let ben_email = address();
    test.add(&ben, &ben_email).await;
    let cleo = test.sign_in(&address(), "Cleo").await;
    // Ben starts it; Cleo takes the link; both sign.
    let deal = app.active_between(ben.clone(), cleo.clone()).await;
    let path = format!("/v1/exchanges/{}/sms-updates", deal.exchange);
    let on = json!({ "on": true, "consent": { "version": CONSENT_VERSION, "language": "en" } });
    app.call(Some(&ben), Method::PUT, &path, Some(on), &[])
        .await
        .ok();
    app.call(
        Some(&ben),
        Method::PUT,
        "/v1/me/payment-handles",
        Some(json!({ "venmo": "ben-fixes" })),
        &[],
    )
    .await
    .ok();
    app.call(
        Some(&ben),
        Method::PUT,
        &format!("/v1/exchanges/{}/payment-options", deal.exchange),
        Some(json!({ "on": true })),
        &[],
    )
    .await
    .ok();
    let session: Uuid =
        sqlx::query_scalar("SELECT id FROM account_session WHERE account_id = $1 LIMIT 1")
            .bind(ben.id)
            .fetch_one(&app.owner)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO device (account_id, session_id, service, token, platform, app_version, language)
         VALUES ($1, $2, 'EXPO', $3, 'ios', '1.0.0', 'en')",
    )
    .bind(ben.id)
    .bind(session)
    .bind(format!("ExponentPushToken[{}]", Uuid::new_v4().simple()))
    .execute(&app.owner)
    .await
    .unwrap();
    let ana = test.sign_in(&address(), "Ana").await;
    Setup {
        ana,
        ben,
        cleo,
        ben_phone,
        ben_email,
        exchange: deal.exchange,
        repair: deal.repair,
        revision: deal.revision,
    }
}

#[tokio::test]
async fn combining_moves_the_yups_number_updates_payment_options_and_ends_the_other_account() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let s = setup(&test).await;
    let (ana, ben) = (&s.ana, &s.ben);

    // Proving Ben's number offers to combine, and says what Ben has.
    let offered = offer(&test, ana, &s.ben_phone).await;
    let last4 = &s.ben_phone[s.ben_phone.len() - 4..];
    assert_eq!(offered["proved"], "PHONE");
    assert_eq!(offered["other"]["phone"], format!("(•••) •••-{last4}"));
    assert_eq!(offered["other"]["display_name"], "Ben");
    assert_eq!(
        offered["other"]["yups"],
        json!({ "drafts": 0, "negotiating": 0, "in_force": 1, "closed": 0 })
    );
    assert_eq!(offered["other"]["payment_options"], true);
    assert_eq!(offered["other"]["text_updates"], true);
    assert_eq!(offered["other"]["devices"], true);
    // Ana has an email of her own, so she keeps it and Ben's is dropped;
    // she has no phone, so Ben's comes; she has no payment options, so his
    // come; his number comes, so his text updates go on.
    assert_eq!(
        (offered["email"].clone(), offered["phone"].clone()),
        (json!("THEIRS_DROPPED"), json!("ADDED"))
    );
    assert_eq!(offered["payment_options_move"], true);
    assert_eq!(offered["text_updates_end"], false);
    // Ana has no number of her own to lose, so no proof of hers is needed.
    assert_eq!(offered["proof_required"], false);
    // Nothing has moved yet.
    assert_eq!(status(&test, ben.id).await.0, "ACTIVE");
    // Ben reads Spanish: he is told in it.
    app.call(
        Some(ben),
        Method::PATCH,
        "/v1/me",
        Some(json!({ "language": "es" })),
        &[],
    )
    .await
    .ok();

    let me = combine_with(&test, ana, &offered["token"], &key())
        .await
        .ok();
    assert_eq!(me["phone"], s.ben_phone);
    assert_ne!(me["email"], s.ben_email);

    // Ben is combined into Ana, can no longer be reached or signed in to.
    assert_eq!(
        status(&test, ben.id).await,
        ("MERGED".to_owned(), Some(ana.id))
    );
    let left: i32 = sqlx::query_scalar(
        "SELECT num_nonnulls(email_encrypted, email_index, phone_encrypted, phone_index)
         FROM account WHERE id = $1",
    )
    .bind(ben.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(left, 0);
    app.get(ben, "/v1/me")
        .await
        .refused(StatusCode::UNAUTHORIZED, "UNAUTHENTICATED");
    // His dropped email is free: signing in with it makes a new account.
    let fresh = test.sign_in(&s.ben_email, "New").await;
    assert_ne!(fresh.id, ben.id);
    assert_ne!(fresh.id, ana.id);

    // Ana holds Ben's place, and acts in it.
    let view = app.view(ana, &s.exchange).await;
    assert_eq!(view["you"], "A");
    assert_eq!(view["state"], "ACTIVE");
    app.act(ana, &s.exchange, s.repair, "CLAIM").await.ok();
    // Cleo still sees the same agreement, with the same names.
    let theirs = app.view(&s.cleo, &s.exchange).await;
    assert_eq!(theirs["other_party_left"], false);

    // The record still shows both signatures standing, none void, and the
    // name as written.
    let record = app
        .get(ana, &format!("/v1/exchanges/{}/record", s.exchange))
        .await
        .ok();
    let text = record.to_string();
    assert!(!text.contains("void_since"), "no void signature: {text}");
    let signed = record["revisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|revision| revision["id"] == s.revision)
        .unwrap();
    assert_eq!(
        signed["signatures"].as_array().unwrap().len(),
        2,
        "{signed}"
    );
    // What Ben signed is still under Ben, as written.
    let signers: Vec<Uuid> = sqlx::query_scalar(
        "SELECT account_id FROM acceptance WHERE revision_id = $1 ORDER BY slot",
    )
    .bind(Uuid::parse_str(&s.revision).unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(signers, vec![ben.id, s.cleo.id]);

    // Text updates moved with the number; the consent record is untouched.
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM sms_update WHERE account_id = $1",
            ana.id
        )
        .await,
        1
    );
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM sms_consent WHERE account_id = $1",
            ben.id
        )
        .await,
        1
    );
    let path = format!("/v1/exchanges/{}/sms-updates", s.exchange);
    assert_eq!(app.get(ana, &path).await.ok()["on"], true);

    // Payment options moved, marked changed now, so Cleo, who owes the
    // money, is warned.
    let handles = app.get(ana, "/v1/me/payment-handles").await.ok();
    assert_eq!(handles["venmo"], "ben-fixes");
    let cleo_sees = app.view(&s.cleo, &s.exchange).await;
    assert_eq!(cleo_sees["payment_options"]["theirs"]["venmo"], "ben-fixes");
    assert!(
        cleo_sees["payment_options"]["theirs_changed"]["venmo"].is_string(),
        "{}",
        cleo_sees["payment_options"]
    );

    // Ben's sessions ended and his device went with them.
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM account_session WHERE account_id = $1 AND revoked_at IS NULL",
            ben.id
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM device WHERE account_id = $1",
            ben.id
        )
        .await,
        0
    );

    // Recorded on both, and in the deletion log for a restore.
    let (merged, into, by): (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT merged_account_id, into_account_id, identifier_kind FROM account_merge
         WHERE merged_account_id = $1",
    )
    .bind(ben.id)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!((merged, into, by.as_str()), (ben.id, ana.id, "PHONE"));
    let (logged_into, logged_by): (Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT merged_into, merged_by FROM deletion_log WHERE account_id = $1")
            .bind(ben.id)
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(
        (logged_into, logged_by.as_deref()),
        (Some(ana.id), Some("PHONE"))
    );

    // Every email address either had is told, by email; no number is
    // texted. Nothing in the queue says who.
    let queued: Vec<(String, Value)> = sqlx::query_as(
        "SELECT kind, payload FROM outbox WHERE recipient_account_id = $1
           AND payload ? 'account_notice' ORDER BY id",
    )
    .bind(ana.id)
    .fetch_all(&app.owner)
    .await
    .unwrap();
    let kinds: Vec<&str> = queued.iter().map(|(kind, _)| kind.as_str()).collect();
    assert_eq!(kinds, ["EMAIL", "EMAIL"]);
    // Each in its own account's language: Ben's in Spanish.
    let mut told = notices(&test, ana.id).await;
    told.sort();
    let mut expected = vec![
        (
            "ACCOUNTS_COMBINED".to_owned(),
            ana.email.clone(),
            "en".to_owned(),
        ),
        (
            "ACCOUNTS_COMBINED".to_owned(),
            s.ben_email.clone(),
            "es".to_owned(),
        ),
    ];
    expected.sort();
    assert_eq!(told, expected);
}

#[tokio::test]
async fn an_offer_is_good_once_for_its_account_and_a_retry_with_its_key_is_harmless() {
    let (test, _turn) = start().await;
    let s = setup(&test).await;
    let offered = offer(&test, &s.ana, &s.ben_email).await;
    // The proof is Ana's: Cleo cannot use the token.
    combine_with(&test, &s.cleo, &offered["token"], &key())
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_EXPIRED");
    // Ben's address would replace Ana's own: that takes a proof of hers, as
    // replacing it directly does. Without one, nothing happens.
    assert_eq!(offered["email"], "REPLACED");
    assert_eq!(offered["proof_required"], true);
    combine_with(&test, &s.ana, &offered["token"], &key())
        .await
        .refused(StatusCode::CONFLICT, "PROOF_REQUIRED");
    let cleos = own_proof(&test, &s.cleo).await;
    combine_proved(&test, &s.ana, &offered["token"], Some(&cleos), &key())
        .await
        .refused(StatusCode::CONFLICT, "PROOF_REQUIRED");
    assert_eq!(status(&test, s.ben.id).await.0, "ACTIVE");
    let proof = own_proof(&test, &s.ana).await;
    let attempt = key();
    combine_proved(&test, &s.ana, &offered["token"], Some(&proof), &attempt)
        .await
        .ok();
    // The same request again, as a client retrying would: done already.
    combine_proved(&test, &s.ana, &offered["token"], Some(&proof), &attempt)
        .await
        .ok();
    // A new request with the spent token: refused.
    combine_with(&test, &s.ana, &offered["token"], &key())
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_EXPIRED");
    // A made-up token, and an expired one.
    combine_with(&test, &s.ana, &json!("f".repeat(64)), &key())
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_EXPIRED");
    // Ana kept her own email: Ben's, the one proved, replaced it.
    let me = test.app.get(&s.ana, "/v1/me").await.ok();
    assert_eq!(me["email"], s.ben_email);
    assert_eq!(me["phone"], s.ben_phone);
}

#[tokio::test]
async fn an_expired_offer_is_refused() {
    let (test, _turn) = start().await;
    let s = setup(&test).await;
    let offered = offer(&test, &s.ana, &s.ben_email).await;
    sqlx::query(
        "UPDATE account_combine_offer SET created_at = now() - interval '1 hour',
                expires_at = now() - interval '1 minute' WHERE account_id = $1",
    )
    .bind(s.ana.id)
    .execute(&test.app.owner)
    .await
    .unwrap();
    combine_with(&test, &s.ana, &offered["token"], &key())
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_EXPIRED");
    assert_eq!(status(&test, s.ben.id).await.0, "ACTIVE");
}

#[tokio::test]
async fn suspended_reviewers_and_the_two_sides_of_one_yup_are_never_combined() {
    let (test, _turn) = start().await;
    let app = &test.app;

    // The two sides of one yup: Cleo proves Ben's address.
    let s = setup(&test).await;
    let reply = prove(&test, &s.cleo, &s.ben_email).await;
    reply.refused(StatusCode::CONFLICT, "COMBINE_SHARED_EXCHANGE");

    // A suspended account, refused when offered and when the offer is used.
    let offered = offer(&test, &s.ana, &s.ben_email).await;
    sqlx::query("UPDATE account SET status = 'SUSPENDED' WHERE id = $1")
        .bind(s.ben.id)
        .execute(&app.owner)
        .await
        .unwrap();
    combine_with(&test, &s.ana, &offered["token"], &key())
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_SUSPENDED");
    let dora = test.sign_in(&address(), "Dora").await;
    prove(&test, &dora, &s.ben_phone)
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_SUSPENDED");

    // A reviewer's account is not combined into another.
    let reviewer_email = address();
    let reviewer = test.sign_in(&reviewer_email, "Rae").await;
    sqlx::query("INSERT INTO staff_member (account_id) VALUES ($1)")
        .bind(reviewer.id)
        .execute(&app.owner)
        .await
        .unwrap();
    prove(&test, &dora, &reviewer_email)
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_REVIEWER");

    // An unconfirmed claimant and the initiator are both sides too.
    let ed = test.sign_in(&address(), "Ed").await;
    let fay_email = address();
    let fay = test.sign_in(&fay_email, "Fay").await;
    let deal = app.negotiating_between(ed.clone(), fay.clone()).await;
    app.post(
        &fay,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();
    prove(&test, &ed, &fay_email)
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_SHARED_EXCHANGE");
    // Nothing moved in any of these.
    for account in [s.ben.id, reviewer.id, fay.id] {
        assert_ne!(status(&test, account).await.0, "MERGED");
    }
}

#[tokio::test]
async fn a_block_between_the_two_goes_and_one_against_the_other_account_carries_over() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let s = setup(&test).await;
    // Ana and Ben blocked each other through a yup they had once, closed.
    sqlx::query(
        "INSERT INTO account_block (blocker_account_id, blocked_account_id) VALUES ($1, $2), ($2, $1)",
    )
    .bind(s.ana.id)
    .bind(s.ben.id)
    .execute(&app.owner)
    .await
    .unwrap();
    // Cleo blocks Ben.
    app.call(
        Some(&s.cleo),
        Method::PUT,
        &format!("/v1/exchanges/{}/block", s.exchange),
        None,
        &[],
    )
    .await;
    let offered = offer(&test, &s.ana, &s.ben_email).await;
    accept_offer(&test, &s.ana, &offered, &key()).await.ok();
    let blocks: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT blocker_account_id, blocked_account_id FROM account_block
         WHERE $1 IN (blocker_account_id, blocked_account_id)
            OR $2 IN (blocker_account_id, blocked_account_id) ORDER BY 1",
    )
    .bind(s.ana.id)
    .bind(s.ben.id)
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(blocks, vec![(s.cleo.id, s.ana.id)]);
}

#[tokio::test]
async fn a_signature_waiting_when_the_accounts_are_combined_still_counts() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let ben_email = address();
    let ben = test.sign_in(&ben_email, "Ben").await;
    let cleo = test.sign_in(&address(), "Cleo").await;
    // Cleo proposes; Ben takes the link, is confirmed, and counters, which
    // signs his version.
    let deal = app.negotiating_between(cleo.clone(), ben.clone()).await;
    app.post(
        &ben,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();
    app.command(
        &cleo,
        &deal.exchange,
        json!({ "type": "CONFIRM_COUNTERPARTY" }),
    )
    .await
    .ok();
    let countered = app
        .send(&ben, &deal.exchange, fence_job(deal.repair, deal.payment))
        .await;
    // A counter needs different terms; the same terms are refused, so send
    // a changed one.
    let countered = if countered.status == StatusCode::OK {
        countered.ok()
    } else {
        let mut terms = fence_job(deal.repair, deal.payment);
        terms["terms"] = json!("Repair the back fence, gate included.");
        app.send(&ben, &deal.exchange, terms).await.ok()
    };
    let revision = countered["exchange"]["open_revision"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let ana = test.sign_in(&address(), "Ana").await;
    let offered = offer(&test, &ana, &ben_email).await;
    accept_offer(&test, &ana, &offered, &key()).await.ok();

    // Cleo signs Ben's version: Ben's signature, given before, is Ana's
    // place's, and the agreement comes into force.
    let view = app
        .command(&cleo, &deal.exchange, accept(&revision))
        .await
        .ok();
    assert_eq!(view["state"], "ACTIVE");
    let holdings: Vec<(i32, Uuid, bool, bool)> = sqlx::query_as(
        "SELECT holding, account_id, ended_at IS NULL, ended_by_merge FROM slot_holding
         WHERE exchange_id = $1 AND slot = 'B' ORDER BY holding",
    )
    .bind(Uuid::parse_str(&deal.exchange).unwrap())
    .fetch_all(&app.owner)
    .await
    .unwrap();
    assert_eq!(
        holdings,
        vec![(1, ben.id, false, true), (2, ana.id, true, false)]
    );
}

#[tokio::test]
async fn the_database_lets_a_place_pass_only_to_the_account_its_holder_was_combined_into() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let deal = app.active().await;
    let stranger = app.user("Sam").await;
    // Neither role may hand a place to someone else outright.
    for pool in [&app.db, &app.owner] {
        let refused = sqlx::query(
            "UPDATE participant SET account_id = $2 WHERE exchange_id = $1 AND slot = 'A'",
        )
        .bind(Uuid::parse_str(&deal.exchange).unwrap())
        .bind(stranger.id)
        .execute(pool)
        .await
        .unwrap_err();
        assert!(refused.to_string().contains("emptied before"), "{refused}");
    }
}

#[tokio::test]
async fn the_service_role_cannot_fake_a_combination_to_move_a_place() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let deal = app.active().await;
    let exchange = Uuid::parse_str(&deal.exchange).unwrap();
    let thief = app.user("Sam").await;
    // As the service's own role: mark the holder combined into another
    // account, move the place, and mark it live again. The place does not
    // move: no combination of the two is recorded.
    let mut tx = app.db.begin().await.unwrap();
    sqlx::query("UPDATE account SET status = 'MERGED', merged_into = $2, merged_at = now(), email_encrypted = NULL,
                email_index = NULL, phone_encrypted = NULL, phone_index = NULL
         WHERE id = $1")
        .bind(deal.ben.id)
        .bind(thief.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let refused =
        sqlx::query("UPDATE participant SET account_id = $2 WHERE exchange_id = $1 AND slot = 'B'")
            .bind(exchange)
            .bind(thief.id)
            .execute(&mut *tx)
            .await
            .unwrap_err();
    assert!(refused.to_string().contains("emptied before"), "{refused}");
    drop(tx);

    // Nor can the mark be taken back once made, or pointed elsewhere.
    let mut tx = app.db.begin().await.unwrap();
    sqlx::query("UPDATE account SET status = 'MERGED', merged_into = $2, merged_at = now(), email_encrypted = NULL,
                email_index = NULL, phone_encrypted = NULL, phone_index = NULL
         WHERE id = $1")
        .bind(deal.ben.id)
        .bind(thief.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut savepoint = sqlx::Acquire::begin(&mut *tx).await.unwrap();
    let refused = sqlx::query("UPDATE account SET status = 'ACTIVE' WHERE id = $1")
        .bind(deal.ben.id)
        .execute(&mut *savepoint)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("stays combined"), "{refused}");
    savepoint.rollback().await.unwrap();
    let elsewhere = app.user("Other").await;
    let refused = sqlx::query("UPDATE account SET merged_into = $2 WHERE id = $1")
        .bind(deal.ben.id)
        .bind(elsewhere.id)
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("follows only"), "{refused}");
    drop(tx);

    // Nothing changed.
    assert_eq!(
        status(&test, deal.ben.id).await,
        ("ACTIVE".to_owned(), None)
    );
    let holder: Uuid = sqlx::query_scalar(
        "SELECT account_id FROM participant WHERE exchange_id = $1 AND slot = 'B'",
    )
    .bind(exchange)
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(holder, deal.ben.id);
}

#[tokio::test]
async fn a_place_beside_the_other_account_taken_after_the_offer_stops_the_combination() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let ben_phone = number();
    let ben = test.sign_in(&ben_phone, "Ben").await;
    let ana = test.sign_in(&address(), "Ana").await;
    let offered = offer(&test, &ana, &ben_phone).await;
    // After the offer, Ana takes the place beside Ben in his yup.
    let deal = app.negotiating_between(ben.clone(), ana.clone()).await;
    app.post(
        &ana,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();
    combine_with(&test, &ana, &offered["token"], &key())
        .await
        .refused(StatusCode::CONFLICT, "COMBINE_SHARED_EXCHANGE");
    assert_eq!(status(&test, ben.id).await.0, "ACTIVE");

    // Had the check been passed, the database refuses one account in both
    // places, by the constraint the service reads as this refusal.
    let exchange = Uuid::parse_str(&deal.exchange).unwrap();
    let mut tx = app.db.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO account_merge
             (merged_account_id, into_account_id, identifier_kind, source, merged_at)
         VALUES ($1, $2, 'PHONE', 'PERSON', now())",
    )
    .bind(ben.id)
    .bind(ana.id)
    .execute(&mut *tx)
    .await
    .unwrap();
    let refused = sqlx::query(
        "UPDATE participant SET account_id = $2 WHERE exchange_id = $1 AND account_id = $3",
    )
    .bind(exchange)
    .bind(ana.id)
    .bind(ben.id)
    .execute(&mut *tx)
    .await
    .unwrap_err();
    let failure = refused.as_database_error().unwrap();
    assert_eq!(failure.code().as_deref(), Some("23505"));
    assert_eq!(
        failure.constraint(),
        Some("participant_exchange_id_account_id_key")
    );
}

#[tokio::test]
async fn a_block_made_through_a_place_the_other_account_left_is_listed_and_lifted_by_the_new_one() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let cleo = test.sign_in(&address(), "Cleo").await;
    let ben_phone = number();
    let ben = test.sign_in(&ben_phone, "Ben").await;
    // Ben takes Cleo's link, unconfirmed, and blocks her through it, which
    // takes him out of it.
    let deal = app.negotiating_between(cleo.clone(), ben.clone()).await;
    app.post(
        &ben,
        "/v1/invitations/claim",
        json!({ "token": deal.invitation }),
    )
    .await
    .ok();
    let path = format!("/v1/exchanges/{}/block", deal.exchange);
    let blocked = app.call(Some(&ben), Method::PUT, &path, None, &[]).await;
    assert_eq!(blocked.status, StatusCode::NO_CONTENT, "{}", blocked.body);

    let ana = test.sign_in(&address(), "Ana").await;
    let offered = offer(&test, &ana, &ben_phone).await;
    accept_offer(&test, &ana, &offered, &key()).await.ok();

    // The block is Ana's now, and so is the place it was made through.
    let people = app.get(&ana, "/v1/blocks").await.ok();
    assert_eq!(people.as_array().unwrap().len(), 1, "{people}");
    assert_eq!(people[0]["exchange_id"], deal.exchange);
    assert_eq!(people[0]["left"], true);
    let lifted = app.call(Some(&ana), Method::DELETE, &path, None, &[]).await;
    assert_eq!(lifted.status, StatusCode::NO_CONTENT, "{}", lifted.body);
    assert_eq!(app.get(&ana, "/v1/blocks").await.ok(), json!([]));
}

#[tokio::test]
async fn two_combinations_at_once_happen_once() {
    let (test, _turn) = start().await;
    let s = setup(&test).await;
    // Ana proves Ben's address and Ben proves Ana's: each offered the other.
    let ana_email = test.app.get(&s.ana, "/v1/me").await.ok()["email"]
        .as_str()
        .unwrap()
        .to_owned();
    let to_ana = offer(&test, &s.ana, &s.ben_email).await;
    let to_ben = offer(&test, &s.ben, &ana_email).await;
    // Each would replace the other's own address: each proves their own.
    let (ana_proof, ben_proof) = (
        own_proof(&test, &s.ana).await,
        own_proof(&test, &s.ben).await,
    );
    let (k1, k2) = (key(), key());
    let (first, second) = tokio::join!(
        combine_proved(&test, &s.ana, &to_ana["token"], Some(&ana_proof), &k1),
        combine_proved(&test, &s.ben, &to_ben["token"], Some(&ben_proof), &k2),
    );
    let worked = [&first, &second]
        .iter()
        .filter(|reply| reply.status == StatusCode::OK)
        .count();
    assert_eq!(worked, 1, "{} / {}", first.body, second.body);
    let merged = [s.ana.id, s.ben.id];
    let mut statuses = Vec::new();
    for account in merged {
        statuses.push(status(&test, account).await.0);
    }
    statuses.sort();
    assert_eq!(statuses, ["ACTIVE", "MERGED"]);

    // And the same offer used twice at once.
    let s = setup(&test).await;
    let offered = offer(&test, &s.ana, &s.ben_email).await;
    let proof = own_proof(&test, &s.ana).await;
    let (k1, k2) = (key(), key());
    let (first, second) = tokio::join!(
        combine_proved(&test, &s.ana, &offered["token"], Some(&proof), &k1),
        combine_proved(&test, &s.ana, &offered["token"], Some(&proof), &k2),
    );
    let mut codes = [first.code().to_owned(), second.code().to_owned()];
    codes.sort();
    assert_eq!(codes, ["".to_owned(), "COMBINE_EXPIRED".to_owned()]);
}

// ---- Deleting and replaying ---------------------------------------------------

#[tokio::test]
async fn deleting_the_combined_account_clears_what_referred_to_the_other() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let s = setup(&test).await;
    let offered = offer(&test, &s.ana, &s.ben_phone).await;
    combine_with(&test, &s.ana, &offered["token"], &key())
        .await
        .ok();

    deletion::delete_account(&app.db, &app.rules, s.ana.id)
        .await
        .unwrap();
    assert_eq!(status(&test, s.ana.id).await.0, "DELETED");
    assert_eq!(
        status(&test, s.ben.id).await,
        ("MERGED".to_owned(), Some(s.ana.id))
    );
    for (what, query) in [
        (
            "Ben's sessions",
            "SELECT count(*) FROM account_session WHERE account_id = $1",
        ),
        (
            "offers",
            "SELECT count(*) FROM account_combine_offer WHERE other_account_id = $1",
        ),
        (
            "text updates",
            "SELECT count(*) FROM sms_update WHERE account_id = $1",
        ),
    ] {
        assert_eq!(scalar_i64(&test, query, s.ben.id).await, 0, "{what}");
    }
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM account_notice WHERE account_id = $1",
            s.ana.id
        )
        .await,
        0
    );
    // Ben's number is free again, as Ana's is.
    let fresh = test.sign_in(&s.ben_phone, "New").await;
    assert_ne!(fresh.id, s.ben.id);
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

#[tokio::test]
async fn replaying_the_log_combines_again_and_then_deletes() {
    let (test, _turn) = start().await;
    let app = &test.app;
    // As a database restored from a backup made before Ana combined Ben's
    // account into hers and then deleted hers: both live and apart.
    let s = setup(&test).await;
    let combined_at = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap();
    let deleted_at = combined_at + time::Duration::seconds(1);
    let stranger = Uuid::new_v4();
    let log = format!(
        "# Yuppers deletion log\n{ben}\t{at}\t{ana}\tPHONE\n{stranger}\t{at}\t{ana}\tEMAIL\n{ana}\t{later}\t\t\n",
        ben = s.ben.id,
        ana = s.ana.id,
        at = rfc3339(combined_at),
        later = rfc3339(deleted_at),
    );
    let entries = deletion_log::parse(&log).unwrap();
    let summary = deletion_log::replay(&app.db, &app.rules, &entries, |_| {}).await;
    assert_eq!(
        (summary.combined, summary.deleted, summary.not_here),
        (1, 1, 1),
        "{summary}"
    );
    assert!(summary.complete());
    assert_eq!(
        status(&test, s.ben.id).await,
        ("MERGED".to_owned(), Some(s.ana.id))
    );
    assert_eq!(status(&test, s.ana.id).await.0, "DELETED");
    // Nobody is told again by a replay.
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM account_notice WHERE account_id = $1",
            s.ana.id
        )
        .await,
        0
    );
    // The log in this database says both, with the times they happened.
    let logged: (Option<Uuid>, OffsetDateTime) =
        sqlx::query_as("SELECT merged_into, deleted_at FROM deletion_log WHERE account_id = $1")
            .bind(s.ben.id)
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(logged, (Some(s.ana.id), combined_at));
    // Again: nothing more.
    let again = deletion_log::replay(&app.db, &app.rules, &entries, |_| {}).await;
    assert_eq!((again.already_combined, again.already_deleted), (1, 1));

    // Alone, a line whose account it went into is not in this copy leaves
    // the account as it is.
    let lone = test.sign_in(&address(), "Lone").await;
    let replayed = combine::replay(
        &app.db,
        &app.rules,
        lone.id,
        Uuid::new_v4(),
        IdentifierKind::Email,
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert_eq!(replayed, combine::Replayed::TargetNotHere);
    assert_eq!(status(&test, lone.id).await.0, "ACTIVE");
}

#[tokio::test]
async fn an_account_combined_into_one_made_after_the_backup_follows_the_log_to_where_that_went() {
    let (test, _turn) = start().await;
    let app = &test.app;
    // The backup was taken before A was made: Ben was combined into A, and
    // A was deleted. Ben goes too, at the time A did.
    let ben = test.sign_in(&number(), "Ben").await;
    let at = OffsetDateTime::now_utc().replace_nanosecond(0).unwrap() + time::Duration::seconds(1);
    let later = at + time::Duration::seconds(1);
    let a = Uuid::new_v4();
    let log = format!(
        "{ben}\t{at}\t{a}\tPHONE\n{a}\t{later}\n",
        ben = ben.id,
        at = rfc3339(at),
        later = rfc3339(later),
    );
    let entries = deletion_log::parse(&log).unwrap();
    let summary = deletion_log::replay(&app.db, &app.rules, &entries, |_| {}).await;
    assert!(summary.complete(), "{summary}");
    assert_eq!((summary.deleted, summary.not_here), (1, 1), "{summary}");
    assert_eq!(status(&test, ben.id).await.0, "DELETED");
    let deleted: OffsetDateTime =
        sqlx::query_scalar("SELECT deleted_at FROM deletion_log WHERE account_id = $1")
            .bind(ben.id)
            .fetch_one(&app.owner)
            .await
            .unwrap();
    assert_eq!(deleted, later);

    // Ben was combined into A, made after the backup, and A into Cleo,
    // who is here: Ben goes into Cleo.
    let ben = test.sign_in(&number(), "Ben").await;
    let cleo = test.sign_in(&number(), "Cleo").await;
    let a = Uuid::new_v4();
    let log = format!(
        "{ben}\t{at}\t{a}\tPHONE\n{a}\t{later}\t{cleo}\tEMAIL\n",
        ben = ben.id,
        cleo = cleo.id,
        at = rfc3339(at),
        later = rfc3339(later),
    );
    let entries = deletion_log::parse(&log).unwrap();
    let mut said = Vec::new();
    let summary = deletion_log::replay(&app.db, &app.rules, &entries, |line| {
        said.push(line.to_owned())
    })
    .await;
    assert!(summary.complete(), "{summary}");
    assert_eq!((summary.combined, summary.not_here), (1, 1), "{summary}");
    assert_eq!(
        status(&test, ben.id).await,
        ("MERGED".to_owned(), Some(cleo.id))
    );
    assert!(said[0].contains(&format!("through {a}")), "{said:?}");
    // A replay tells nobody, in the app or outside.
    assert!(in_app(&test, &cleo).await.is_null());
    assert!(notices(&test, cleo.id).await.is_empty());

    // Where no later line says where A went, Ben is left as he is, and the
    // replay says so and is not complete.
    let dan = test.sign_in(&number(), "Dan").await;
    let log = format!(
        "{dan}\t{at}\t{a}\tPHONE\n",
        dan = dan.id,
        a = Uuid::new_v4(),
        at = rfc3339(at),
    );
    let entries = deletion_log::parse(&log).unwrap();
    let mut said = Vec::new();
    let summary = deletion_log::replay(&app.db, &app.rules, &entries, |line| {
        said.push(line.to_owned())
    })
    .await;
    assert!(!summary.complete(), "{summary}");
    assert_eq!(summary.unresolved, vec![dan.id]);
    assert!(said[0].contains("UNRESOLVED"), "{said:?}");
    assert!(summary.to_string().contains("UNRESOLVED"), "{summary}");
    assert_eq!(status(&test, dan.id).await.0, "ACTIVE");
}

// ---- Invitations sent to another address ---------------------------------------

/// Ana's yup, its invitation bound to `bound`. Returns its exchange and token.
async fn invite(test: &Texting, ana: &User, bound: &str) -> (String, String) {
    let app = &test.app;
    let exchange = app.draft(ana).await;
    let sent = app
        .post(
            ana,
            &format!("/v1/exchanges/{exchange}/revisions"),
            json!({
                "expected_version": 0,
                "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
                "consent": common::consent(),
                "invitation": { "bound_to": bound },
            }),
        )
        .await
        .ok();
    (
        exchange,
        sent["invitation_token"].as_str().unwrap().to_owned(),
    )
}

/// Asks for a code to `typed`, as the address `token`'s invitation was sent to.
async fn ask_invitation_code(test: &Texting, user: &User, token: &str, typed: &str) -> Reply {
    test.app
        .post(
            user,
            "/v1/invitations/address/codes",
            json!({ "token": token, "identifier": typed, "sms_consent": common::sms_consent() }),
        )
        .await
}

/// Adds `typed` with its code, and opens the invitation.
async fn add_invitation_address(
    test: &Texting,
    user: &User,
    token: &str,
    typed: &str,
    extra: Value,
) -> Reply {
    let mut body = json!({ "token": token, "identifier": typed, "code": test.code(typed) });
    for (name, value) in extra.as_object().unwrap() {
        body[name] = value.clone();
    }
    test.app.post(user, "/v1/invitations/address", body).await
}

/// How many codes were ever stored for `identifier`.
async fn codes_sent(test: &Texting, identifier: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM one_time_code WHERE identifier_index = $1")
        .bind(common::index(identifier))
        .fetch_one(&test.app.owner)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_invitation_sent_to_an_address_nobody_has_adds_it_and_opens() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let ana = test.sign_in(&address(), "Ana").await;
    // Its exchange begins with 2 to 9, so it can be typed without +1.
    let bound = format!("+1999555{:04}", Uuid::new_v4().as_u128() % 10_000);
    let (exchange, token) = invite(&test, &ana, &bound).await;
    // Ben signed in by email; the invitation was sent to a phone number.
    let ben = test.sign_in(&address(), "Ben").await;
    let preview = app
        .post(&ben, "/v1/invitations/preview", json!({ "token": token }))
        .await
        .ok();
    // Only the kind is said: not the number, in full or in part.
    assert_eq!(
        preview["sent_to"],
        json!({ "kind": "PHONE", "replaces": false })
    );
    // (Its last digits alone could turn up by chance in an ID or a time.)
    let text = preview.to_string();
    assert!(!text.contains(&bound[2..]) && !text.contains("•"), "{text}");
    // Claiming as it is still names someone else.
    app.post(&ben, "/v1/invitations/claim", json!({ "token": token }))
        .await
        .refused(StatusCode::FORBIDDEN, "INVITATION_NOT_FOR_YOU");

    // He types the number. Another one is refused, saying nothing more, and
    // nothing is sent to it.
    let wrong = number();
    ask_invitation_code(&test, &ben, &token, &wrong)
        .await
        .refused(StatusCode::UNPROCESSABLE_ENTITY, "NOT_INVITED_ADDRESS");
    assert_eq!(codes_sent(&test, &wrong).await, 0);
    // The right one, typed as people in the US write it, gets a code; a
    // wrong code is refused as any wrong code is.
    let typed = format!("({}) {}-{}", &bound[2..5], &bound[5..8], &bound[8..]);
    let reply = ask_invitation_code(&test, &ben, &token, &typed).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    app.post(
        &ben,
        "/v1/invitations/address",
        json!({ "token": token, "identifier": bound, "code": "000000" }),
    )
    .await
    .refused(StatusCode::UNAUTHORIZED, "INVALID_CODE");
    let view = add_invitation_address(&test, &ben, &token, &bound, json!({}))
        .await
        .ok();
    assert_eq!(view["id"], exchange);
    assert_eq!(view["you"], "B");
    // Named, so nobody has to confirm Ben.
    assert_eq!(view["counterparty"], "CONFIRMED");
    assert_eq!(app.get(&ben, "/v1/me").await.ok()["phone"], bound);
}

#[tokio::test]
async fn an_invitation_sent_to_another_accounts_address_offers_to_combine_then_opens() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let ana = test.sign_in(&address(), "Ana").await;
    // Ben has an account by email; he signs in on this phone by number.
    let ben_email = address();
    let ben_by_email = test.sign_in(&ben_email, "Ben").await;
    let (exchange, token) = invite(&test, &ana, &ben_email).await;
    let ben = test.sign_in(&number(), "Ben").await;

    // Before the code, asking is answered as for any address.
    let reply = ask_invitation_code(&test, &ben, &token, &ben_email).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let reply = add_invitation_address(&test, &ben, &token, &ben_email, json!({})).await;
    reply.refused(StatusCode::CONFLICT, "IDENTIFIER_ON_OTHER_ACCOUNT");
    let offered = reply.body["combine"].clone();
    assert_eq!(offered["proved"], "EMAIL");
    // Nothing was added, nor the invitation opened.
    assert_eq!(app.get(&ben, "/v1/me").await.ok()["email"], Value::Null);
    combine_with(&test, &ben, &offered["token"], &key())
        .await
        .ok();
    assert_eq!(
        status(&test, ben_by_email.id).await,
        ("MERGED".to_owned(), Some(ben.id))
    );
    // Now the invitation is his.
    let view = app
        .post(&ben, "/v1/invitations/claim", json!({ "token": token }))
        .await
        .ok();
    assert_eq!(view["id"], exchange);
    assert_eq!(view["counterparty"], "CONFIRMED");
}

#[tokio::test]
async fn an_invitation_to_another_address_of_a_kind_the_account_has_replaces_it_only_with_a_proof()
{
    let (test, _turn) = start().await;
    let app = &test.app;
    let ana = test.sign_in(&address(), "Ana").await;
    let bound = address();
    let (exchange, token) = invite(&test, &ana, &bound).await;
    let ben_email = address();
    let ben = test.sign_in(&ben_email, "Ben").await;
    let preview = app
        .post(&ben, "/v1/invitations/preview", json!({ "token": token }))
        .await
        .ok();
    assert_eq!(preview["sent_to"]["replaces"], true);
    ask_invitation_code(&test, &ben, &token, &bound).await;
    // Refused before the code is looked at, so the code still works:
    // without saying to replace, and without a proof of his own.
    add_invitation_address(&test, &ben, &token, &bound, json!({}))
        .await
        .refused(StatusCode::CONFLICT, "IDENTIFIER_KIND_TAKEN");
    add_invitation_address(&test, &ben, &token, &bound, json!({ "replace": true }))
        .await
        .refused(StatusCode::CONFLICT, "PROOF_REQUIRED");
    let proof = own_proof(&test, &ben).await;
    let view = add_invitation_address(
        &test,
        &ben,
        &token,
        &bound,
        json!({ "replace": true, "proof": proof }),
    )
    .await
    .ok();
    assert_eq!(view["id"], exchange);
    assert_eq!(app.get(&ben, "/v1/me").await.ok()["email"], bound);
    // The address replaced is told.
    assert_eq!(
        notices(&test, ben.id).await,
        vec![("EMAIL_CHANGED".to_owned(), ben_email, "en".to_owned())]
    );
}

#[tokio::test]
async fn a_claim_that_fails_leaves_the_invitation_address_unadded() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let ana = test.sign_in(&address(), "Ana").await;
    let bound = number();
    let (exchange, token) = invite(&test, &ana, &bound).await;
    let ben = test.sign_in(&address(), "Ben").await;
    ask_invitation_code(&test, &ben, &token, &bound).await;
    // The claim breaks, after the address was put on the account.
    let exchange_id = Uuid::parse_str(&exchange).unwrap();
    let trigger = format!("fail_claim_{}", exchange_id.simple());
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {trigger}() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF NEW.exchange_id = '{exchange_id}' AND NEW.claimed_by IS NOT NULL THEN
                 RAISE EXCEPTION 'the claim breaks';
             END IF;
             RETURN NEW;
         END; $$"
    )))
    .execute(&app.owner)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TRIGGER {trigger} BEFORE UPDATE ON invitation
         FOR EACH ROW EXECUTE FUNCTION {trigger}()"
    )))
    .execute(&app.owner)
    .await
    .unwrap();
    let reply = add_invitation_address(&test, &ben, &token, &bound, json!({})).await;
    for statement in [
        format!("DROP TRIGGER {trigger} ON invitation"),
        format!("DROP FUNCTION {trigger}()"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&app.owner)
            .await
            .unwrap();
    }
    assert_eq!(
        reply.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        reply.body
    );
    // Nothing changed: the number is not his, and the place is open.
    assert_eq!(app.get(&ben, "/v1/me").await.ok()["phone"], Value::Null);
    assert_eq!(
        app.post(&ben, "/v1/invitations/preview", json!({ "token": token }))
            .await
            .ok()["sent_to"]["kind"],
        "PHONE"
    );
}

#[tokio::test]
async fn tries_at_an_invitation_address_are_limited_and_say_nothing_of_it() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let ana = test.sign_in(&address(), "Ana").await;
    let bound = number();
    let (_, token) = invite(&test, &ana, &bound).await;

    // The number replied STOP: whoever asks is told only that no code was
    // sent, as for any address a code cannot go to.
    test.inbound(&bound, "STOP").await;
    let ben = test.sign_in(&address(), "Ben").await;
    let reply = ask_invitation_code(&test, &ben, &token, &bound).await;
    reply.refused(StatusCode::CONFLICT, "CODE_NOT_SENT");
    assert!(!reply.body.to_string().contains("OPTED_OUT"));

    // Ten tries an hour for an account, whatever is typed; then it is
    // refused, the right address included.
    for _ in 0..9 {
        ask_invitation_code(&test, &ben, &token, &number())
            .await
            .refused(StatusCode::UNPROCESSABLE_ENTITY, "NOT_INVITED_ADDRESS");
    }
    ask_invitation_code(&test, &ben, &token, &number())
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
    app.post(
        &ben,
        "/v1/invitations/address",
        json!({ "token": token, "identifier": bound, "code": "000000" }),
    )
    .await
    .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");

    // And ten an hour for an invitation, whoever tries.
    let (_, other) = invite(&test, &ana, &number()).await;
    for _ in 0..10 {
        let cleo = test.sign_in(&address(), "Cleo").await;
        ask_invitation_code(&test, &cleo, &other, &number())
            .await
            .refused(StatusCode::UNPROCESSABLE_ENTITY, "NOT_INVITED_ADDRESS");
    }
    let dora = test.sign_in(&address(), "Dora").await;
    ask_invitation_code(&test, &dora, &other, &number())
        .await
        .refused(StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS");
}

#[tokio::test]
async fn nothing_about_an_invitation_address_is_said_to_the_wrong_people() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let ana = test.sign_in(&address(), "Ana").await;
    // A link for anyone: nothing to add.
    let deal = app
        .negotiating_between(ana.clone(), app.user("X").await)
        .await;
    let ben = test.sign_in(&address(), "Ben").await;
    let preview = app
        .post(
            &ben,
            "/v1/invitations/preview",
            json!({ "token": deal.invitation }),
        )
        .await
        .ok();
    assert_eq!(preview["sent_to"], Value::Null);
    ask_invitation_code(&test, &ben, &deal.invitation, &address())
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    // A made-up link: the same dead link as everywhere.
    ask_invitation_code(&test, &ben, &"0".repeat(64), &address())
        .await
        .refused(StatusCode::NOT_FOUND, "INVITATION_UNAVAILABLE");
    // Its own sender is shown nothing to add either.
    let bound = address();
    let (_, token) = invite(&test, &ana, &bound).await;
    let own = app
        .post(&ana, "/v1/invitations/preview", json!({ "token": token }))
        .await
        .ok();
    assert_eq!(own["sent_to"], Value::Null);
    ask_invitation_code(&test, &ana, &token, &bound)
        .await
        .refused(StatusCode::CONFLICT, "ACTION_NOT_ALLOWED");
    // The address is kept nowhere but as its blind index.
    let kept: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
         WHERE table_name = 'invitation' AND column_name LIKE '%encrypted%'",
    )
    .fetch_one(&app.owner)
    .await
    .unwrap();
    assert_eq!(kept, 0);
}

#[tokio::test]
async fn staff_see_that_a_reported_account_was_combined_and_act_on_the_one_it_went_into() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let s = setup(&test).await;
    // Cleo reports Ben.
    app.post(
        &s.cleo,
        &format!("/v1/exchanges/{}/reports", s.exchange),
        json!({ "reason": "SCAM", "details": null }),
    )
    .await;
    let report: Uuid = sqlx::query_scalar("SELECT id FROM report WHERE subject_account_id = $1")
        .bind(s.ben.id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
    let offered = offer(&test, &s.ana, &s.ben_email).await;
    accept_offer(&test, &s.ana, &offered, &key()).await.ok();

    let rae = test.sign_in(&address(), "Rae").await;
    sqlx::query("INSERT INTO staff_member (account_id) VALUES ($1)")
        .bind(rae.id)
        .execute(&app.owner)
        .await
        .unwrap();
    let opened = app
        .get(&rae, &format!("/v1/staff/reports/{report}"))
        .await
        .ok();
    assert_eq!(opened["subject"]["status"], "MERGED");
    assert_eq!(opened["subject"]["merged_into"], json!(s.ana.id));
    assert_eq!(opened["subject"]["party"], "A");
    // Suspending the person reported suspends the account they are now.
    let resolved = app
        .post(
            &rae,
            &format!("/v1/staff/reports/{report}/resolution"),
            json!({ "outcome": "ACCOUNT_SUSPENDED", "note": "Scam." }),
        )
        .await;
    assert_eq!(resolved.status, StatusCode::NO_CONTENT, "{}", resolved.body);
    assert_eq!(status(&test, s.ana.id).await.0, "SUSPENDED");
}

#[tokio::test]
async fn the_combined_accounts_notices_go_to_every_email_address_and_never_by_text() {
    let (test, _turn) = start().await;
    let s = setup(&test).await;
    let offered = offer(&test, &s.ana, &s.ben_phone).await;
    combine_with(&test, &s.ana, &offered["token"], &key())
        .await
        .ok();
    // Each notice holds its email address encrypted, bound to its row.
    let mut told: Vec<String> = notices(&test, s.ana.id)
        .await
        .into_iter()
        .map(|(_, email, _)| email)
        .collect();
    told.sort();
    let mut expected = vec![s.ana.email.clone(), s.ben_email.clone()];
    expected.sort();
    assert_eq!(told, expected);
    // Nothing texted, and nothing shown in the app: the emails told them.
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM outbox WHERE recipient_account_id = $1 AND kind = 'SMS'
               AND payload ? 'account_notice'",
            s.ana.id
        )
        .await,
        0
    );
    assert!(in_app(&test, &s.ana).await.is_null());
}

#[tokio::test]
async fn two_phone_only_accounts_combined_are_told_in_the_app_once() {
    let (test, _turn) = start().await;
    let app = &test.app;
    let (ana_phone, ben_phone) = (number(), number());
    let ana = test.sign_in(&ana_phone, "Ana").await;
    let ben = test.sign_in(&ben_phone, "Ben").await;
    let offered = offer(&test, &ana, &ben_phone).await;
    accept_offer(&test, &ana, &offered, &key()).await.ok();
    assert_eq!(
        status(&test, ben.id).await,
        ("MERGED".to_owned(), Some(ana.id))
    );
    // No email to tell, and no text: nothing queued outside.
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM outbox WHERE recipient_account_id = $1
               AND payload ? 'account_notice'",
            ana.id
        )
        .await,
        0
    );
    assert_eq!(
        scalar_i64(
            &test,
            "SELECT count(*) FROM account_notice WHERE account_id = $1",
            ana.id
        )
        .await,
        0
    );
    // Instead the app is told to show it, until it is dismissed.
    let me = app.get(&ana, "/v1/me").await.ok();
    assert_eq!(me["notice"]["kind"], "ACCOUNTS_COMBINED", "{me}");
    assert!(me["notice"]["at"].is_string(), "{me}");
    let me = app
        .call(
            Some(&ana),
            Method::PATCH,
            "/v1/me",
            Some(json!({ "dismiss_notice": true })),
            &[],
        )
        .await
        .ok();
    assert!(me["notice"].is_null(), "{me}");
}
