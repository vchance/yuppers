//! A service that texts, with codes by email and by a stand-in for Twilio
//! Verify kept for the test to read, and a whole life of two people through
//! the API: what the contact-data tests run (`tests/contact.rs`,
//! `tests/contact_scan.rs`).

use std::sync::{Arc, Mutex};

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
use yuppers_backend::auth::{
    AuthRules, CheckFuture, CodeMessage, CodeSender, CodeVerifier, Purpose, SendFuture,
};
use yuppers_backend::domain::identity::Identifier;
use yuppers_backend::notifications::sms::{CodeRouter, PhoneCodes, twilio_signature};
use yuppers_backend::notifications::sms_updates::{
    self, CONSENT_VERSION, DEFAULT_TEXTS_PER_PERSON_PER_DAY,
};

use super::{App, User, accept, consent, fence_job};

/// The auth token Twilio signs the webhook's requests with, here.
const TOKEN: &str = "test-auth-token";
const WEBHOOK: &str = "https://app.test/v1/sms/inbound";

pub fn number() -> String {
    format!("+1999{:07}", Uuid::new_v4().as_u128() % 10_000_000)
}

pub fn address() -> String {
    format!("{}@contact.example.test", Uuid::new_v4().simple())
}

/// Keeps the codes sent by email.
#[derive(Default)]
pub struct Mailbox(Mutex<Vec<(String, String)>>);

impl CodeSender for Mailbox {
    fn send<'a>(&'a self, message: CodeMessage<'a>) -> SendFuture<'a> {
        Box::pin(async move {
            self.0
                .lock()
                .unwrap()
                .push((message.to.as_str().to_owned(), message.code.to_owned()));
            Ok(())
        })
    }
}

/// Stands in for Twilio Verify: keeps the code it texts each number.
#[derive(Default)]
pub struct Verify(Mutex<Vec<(String, Purpose, String)>>);

impl CodeVerifier for Verify {
    fn start<'a>(&'a self, to: &'a str, purpose: Purpose, _language: &'a str) -> SendFuture<'a> {
        Box::pin(async move {
            let code = format!("{:06}", Uuid::new_v4().as_u128() % 1_000_000);
            self.0.lock().unwrap().push((to.to_owned(), purpose, code));
            Ok(())
        })
    }

    fn check<'a>(&'a self, to: &'a str, purpose: Purpose, code: &'a str) -> CheckFuture<'a> {
        Box::pin(async move {
            let sent = self.0.lock().unwrap();
            Ok(sent
                .iter()
                .any(|each| each.0 == to && each.1 == purpose && each.2 == code))
        })
    }
}

pub struct Texting {
    pub app: App,
    mailbox: Arc<Mailbox>,
    verify: Arc<Verify>,
}

/// The service, texting, on `database`.
pub async fn texting(database: &'static str) -> Texting {
    sms_updates::configure(true, DEFAULT_TEXTS_PER_PERSON_PER_DAY);
    let (mailbox, verify) = (Arc::new(Mailbox::default()), Arc::new(Verify::default()));
    let router = Arc::new(CodeRouter::new(
        mailbox.clone(),
        PhoneCodes::Verify(verify.clone()),
    ));
    let rules = AuthRules {
        code_requests_per_address_per_hour: 1_000_000,
        sms_codes_per_prefix_per_hour: 1_000_000,
        ..AuthRules::default()
    };
    let app = App::start_texting(database, rules, router, TOKEN).await;
    Texting {
        app,
        mailbox,
        verify,
    }
}

impl Texting {
    /// The latest code sent to an identifier, by email or by text.
    pub fn code(&self, to: &str) -> String {
        let by_email = self
            .mailbox
            .0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(sent, _)| sent == to)
            .map(|(_, code)| code.clone());
        by_email.unwrap_or_else(|| {
            self.verify
                .0
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(sent, ..)| sent == to)
                .map(|(.., code)| code.clone())
                .unwrap_or_else(|| panic!("no code was sent to {to}"))
        })
    }

    /// Asks for a code for `identifier`, as signed-in `user` or signed out.
    pub async fn ask(&self, user: Option<&User>, identifier: &str) {
        let reply = self
            .app
            .call(
                user,
                Method::POST,
                "/v1/auth/codes",
                Some(json!({ "identifier": identifier, "sms_consent": super::sms_consent() })),
                &[],
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
    }

    /// Signs up or in with `typed`, as typed, through the API.
    pub async fn sign_in(&self, typed: &str, name: &str) -> User {
        self.ask(None, typed).await;
        let normalized = Identifier::parse(typed).unwrap().as_str().to_owned();
        let body =
            json!({ "identifier": typed, "code": self.code(&normalized), "delivery": "TOKEN" });
        let reply = self
            .app
            .call(None, Method::POST, "/v1/auth/sessions", Some(body), &[])
            .await
            .ok();
        let user = User::signed_in(
            reply["account"]["id"].as_str().unwrap().parse().unwrap(),
            reply["account"]["email"].as_str().unwrap_or("").to_owned(),
            reply["token"].as_str().unwrap().to_owned(),
        );
        self.app
            .call(
                Some(&user),
                Method::PATCH,
                "/v1/me",
                Some(json!({ "display_name": name, "adult_confirmed": true })),
                &[],
            )
            .await
            .ok();
        user
    }

    /// Adds an identifier to a signed-in account, with the code sent to it.
    pub async fn add(&self, user: &User, identifier: &str) -> Value {
        self.ask(Some(user), identifier).await;
        self.app
            .post(
                user,
                "/v1/me/identifiers",
                json!({ "identifier": identifier, "code": self.code(identifier) }),
            )
            .await
            .ok()
    }

    /// Posts a text from `from` to the webhook, signed as Twilio signs it.
    pub async fn inbound(&self, from: &str, body: &str) {
        let params = vec![
            ("AccountSid".to_owned(), "AC0123".to_owned()),
            (
                "MessageSid".to_owned(),
                format!("SM{}", Uuid::new_v4().simple()),
            ),
            ("From".to_owned(), from.to_owned()),
            ("To".to_owned(), "+15550000000".to_owned()),
            ("Body".to_owned(), body.to_owned()),
        ];
        let signature = twilio_signature(TOKEN, WEBHOOK, &params);
        let form: String = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&params)
            .finish();
        let reply = self
            .app
            .raw(
                Method::POST,
                "/v1/sms/inbound",
                "application/x-www-form-urlencoded",
                form.into_bytes(),
                Some(("x-twilio-signature", &signature)),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
    }

    /// A whole life of two people through the API: signing up by email (as
    /// typed, with capitals) and by phone, adding the other kind, an
    /// invitation bound to an address and refused to someone else, an
    /// agreement, text updates turned on, STOP, START and STOP again, a code
    /// refused to the number that said STOP, and one account deleted with a
    /// code by text. Returns every address and number used, and the numbers
    /// without their plus sign, to look for afterwards.
    pub async fn whole_life(&self) -> Vec<String> {
        let app = &self.app;
        let (ana_email, ana_phone) = (address(), number());
        let (ben_email, ben_phone) = (address(), number());
        let cleo_email = address();

        let ana = self.sign_in(&ana_email.to_uppercase(), "Ana").await;
        assert_eq!(ana.email, ana_email, "shown to its owner, as normalized");
        assert_eq!(self.add(&ana, &ana_phone).await["phone"], ana_phone);
        let by_phone = self.sign_in(&ben_phone, "Ben").await;
        let me = self.add(&by_phone, &ben_email).await;
        assert_eq!(
            (me["email"].as_str(), me["phone"].as_str()),
            (Some(ben_email.as_str()), Some(ben_phone.as_str()))
        );
        // Signing in by the address added reaches the same account.
        let ben = self.sign_in(&ben_email, "Ben").await;
        assert_eq!(ben.id, by_phone.id);

        // Ana invites Ben by his address; he takes it and signs.
        let exchange = app.draft(&ana).await;
        let sent = app
            .post(
                &ana,
                &format!("/v1/exchanges/{exchange}/revisions"),
                json!({
                    "expected_version": 0,
                    "terms": fence_job(Uuid::new_v4(), Uuid::new_v4()),
                    "consent": consent(),
                    "invitation": { "bound_to": ben_email.to_uppercase() },
                }),
            )
            .await
            .ok();
        let revision = sent["exchange"]["open_revision"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let token = sent["invitation_token"].as_str().unwrap().to_owned();
        let cleo = self.sign_in(&cleo_email, "Cleo").await;
        app.post(&cleo, "/v1/invitations/claim", json!({ "token": token }))
            .await
            .refused(StatusCode::FORBIDDEN, "INVITATION_NOT_FOR_YOU");
        app.post(&ben, "/v1/invitations/claim", json!({ "token": token }))
            .await
            .ok();
        let view = app.command(&ben, &exchange, accept(&revision)).await.ok();
        assert_eq!(view["state"], "ACTIVE");

        // Ana turns text updates on, replies STOP, START, and STOP again.
        let path = format!("/v1/exchanges/{exchange}/sms-updates");
        let on = json!({ "on": true, "consent": { "version": CONSENT_VERSION, "language": "en" } });
        let reply = app
            .call(Some(&ana), Method::PUT, &path, Some(on.clone()), &[])
            .await
            .ok();
        assert_eq!(
            (reply["on"].clone(), reply["phone"].clone()),
            (json!(true), json!(ana_phone))
        );
        self.inbound(&ana_phone, "STOP").await;
        let reply = app.get(&ana, &path).await.ok();
        assert_eq!(
            (reply["on"].clone(), reply["opted_out"].clone()),
            (json!(false), json!(true))
        );
        self.inbound(&ana_phone, "START").await;
        assert_eq!(app.get(&ana, &path).await.ok()["opted_out"], false);
        app.call(Some(&ana), Method::PUT, &path, Some(on), &[])
            .await
            .ok();
        self.inbound(&ana_phone, "stop").await;
        assert_eq!(app.get(&ana, &path).await.ok()["opted_out"], true);
        let reply = app
            .call(
                None,
                Method::POST,
                "/v1/auth/codes",
                Some(json!({ "identifier": ana_phone, "sms_consent": super::sms_consent() })),
                &[],
            )
            .await;
        reply.refused(StatusCode::CONFLICT, "PHONE_OPTED_OUT");

        // Ben deletes his account with a code by text.
        let reply = app
            .post(
                &ben,
                "/v1/me/deletion/codes",
                json!({ "channel": "PHONE", "sms_consent": super::sms_consent() }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
        let code = self.code(&ben_phone);
        let reply = app
            .post(
                &ben,
                "/v1/me/deletion",
                json!({ "channel": "PHONE", "code": code }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT, "{}", reply.body);
        let ben_left: i32 = sqlx::query_scalar(
            "SELECT num_nonnulls(email_encrypted, email_index, phone_encrypted, phone_index)
             FROM account WHERE id = $1",
        )
        .bind(ben.id)
        .fetch_one(&app.owner)
        .await
        .unwrap();
        assert_eq!(
            ben_left, 0,
            "a deleted account keeps no address in any form"
        );

        // What the service did is all there, under the indexes.
        let (listed, records): (bool, i64) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM sms_opt_out WHERE phone_index = $1),
                    (SELECT count(*) FROM sms_consent WHERE phone_index = $1)",
        )
        .bind(super::index(&ana_phone))
        .fetch_one(&app.owner)
        .await
        .unwrap();
        assert!(listed);
        assert!(records >= 6, "{records}");

        [&ana_email, &ben_email, &cleo_email, &ana_phone, &ben_phone]
            .into_iter()
            .cloned()
            .chain([&ana_phone, &ben_phone].map(|phone| phone[1..].to_owned()))
            .collect()
    }
}

/// Every place in the database, owner's view, where any of `needles`
/// appears, case aside: in any column of any table as text, and in a bytea
/// column as raw bytes too.
pub async fn found_anywhere(owner: &PgPool, needles: &[String]) -> Vec<String> {
    let columns: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.table_name, c.column_name, c.data_type
         FROM information_schema.columns c
         JOIN information_schema.tables t
           ON t.table_schema = c.table_schema AND t.table_name = c.table_name
         WHERE c.table_schema = 'public' AND t.table_type = 'BASE TABLE'
         ORDER BY 1, 2",
    )
    .fetch_all(owner)
    .await
    .unwrap();
    let mut found = Vec::new();
    for (table, column, data_type) in columns {
        for needle in needles {
            let condition = if data_type == "bytea" {
                format!(
                    "strpos(encode(\"{column}\", 'hex'), encode(convert_to($1, 'UTF8'), 'hex')) > 0"
                )
            } else {
                format!("strpos(lower(\"{column}\"::text), lower($1)) > 0")
            };
            let hits: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM \"{table}\" WHERE {condition}"
            )))
            .bind(needle)
            .fetch_one(owner)
            .await
            .unwrap();
            if hits > 0 {
                found.push(format!("{table}.{column} holds {needle} ({hits} rows)"));
            }
        }
    }
    found
}
