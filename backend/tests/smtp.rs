//! The SMTP sender against a real SMTP conversation: a small server in this
//! process that speaks the protocol and keeps what it was given.
//!
//! Delivery acts on every queued message in the database, so the tests that
//! use the outbox take turns, and each starts with an empty outbox.

mod common;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{App, Deal, User};
use mail_parser::{MessageParser, MimeHeaders};
use time::OffsetDateTime;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::MutexGuard;
use tracing_subscriber::EnvFilter;
use yuppers_backend::auth::{CodeMessage, CodeSender, Purpose};
use yuppers_backend::domain::identity::Identifier;
use yuppers_backend::domain::notification::Notice;
use yuppers_backend::notifications::outbox::{Delivery, DeliveryRules, deliver_due};
use yuppers_backend::notifications::smtp::{Secret, SmtpSender, SmtpSettings, TlsMode};
use yuppers_backend::notifications::wording::{Links, Rendered, Wording};
use yuppers_backend::notifications::{Email, EmailSender};
use yuppers_backend::telemetry::{self, LogFormat};

const DATABASE: &str = "yuppers_test_smtp";
const WEB_ORIGIN: &str = "https://app.test";
const FROM: &str = "Yuppers <no-reply@example.test>";
const USERNAME: &str = "yuppers-user";
const PASSWORD: &str = "hunter2-not-for-logs";

static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---- A test SMTP server -----------------------------------------------------

/// One message as the server received it.
#[derive(Clone, Debug, Default)]
struct Received {
    /// The `AUTH PLAIN` credential, base64 as sent.
    auth: Option<String>,
    from: String,
    to: Vec<String>,
    /// The message, headers and body, as transferred.
    data: Vec<u8>,
}

impl Received {
    fn parsed(&self) -> mail_parser::Message<'_> {
        MessageParser::default()
            .parse(&self.data)
            .expect("the data is a mail message")
    }

    fn subject(&self) -> String {
        self.parsed().subject().unwrap_or("").to_owned()
    }

    fn body(&self) -> String {
        self.parsed().body_text(0).unwrap_or_default().into_owned()
    }
}

#[derive(Clone, Copy)]
enum Behavior {
    /// Speaks SMTP without TLS and accepts everything.
    Serve,
    /// Accepts the connection and never says a word.
    Hang,
    /// Speaks SMTP and refuses every recipient, quoting the address back
    /// the way real servers do.
    Reject,
    /// Speaks SMTP and refuses the credentials.
    WrongPassword,
}

struct Server {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<Received>>>,
}

impl Server {
    async fn start(behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let store = received.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let store = store.clone();
                tokio::spawn(async move {
                    match behavior {
                        Behavior::Serve | Behavior::Reject | Behavior::WrongPassword => {
                            let _ = converse(stream, store, behavior).await;
                        }
                        Behavior::Hang => {
                            // Hold the connection open and silent.
                            let _keep = stream;
                            std::future::pending::<()>().await;
                        }
                    }
                });
            }
        });
        Self { addr, received }
    }

    /// An address nothing listens on.
    async fn refused() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    }

    fn received(&self) -> Vec<Received> {
        self.received.lock().unwrap().clone()
    }
}

/// The address between the angle brackets of a `MAIL FROM` or `RCPT TO`.
fn angle(line: &str) -> String {
    line.split_once('<')
        .and_then(|(_, rest)| rest.split_once('>'))
        .map(|(address, _)| address.to_owned())
        .unwrap_or_default()
}

async fn converse(
    stream: TcpStream,
    store: Arc<Mutex<Vec<Received>>>,
    behavior: Behavior,
) -> std::io::Result<()> {
    let reject = matches!(behavior, Behavior::Reject);
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    writer.write_all(b"220 test.invalid ESMTP\r\n").await?;
    let mut current = Received::default();
    while let Some(line) = lines.next_line().await? {
        let upper = line.to_ascii_uppercase();
        let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            // No STARTTLS: a client that insists on it must give up here.
            b"250-test.invalid\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n"
        } else if line.starts_with("AUTH PLAIN ") && matches!(behavior, Behavior::WrongPassword) {
            b"535 5.7.8 Authentication credentials invalid\r\n"
        } else if let Some(credential) = line.strip_prefix("AUTH PLAIN ") {
            current.auth = Some(credential.to_owned());
            b"235 2.7.0 ok\r\n"
        } else if upper.starts_with("MAIL FROM:") {
            current.from = angle(&line);
            b"250 ok\r\n"
        } else if upper.starts_with("RCPT TO:") && reject {
            let to = angle(&line);
            let reply = format!("550 5.1.1 <{to}>: Recipient address rejected: {to} unknown\r\n");
            writer.write_all(reply.as_bytes()).await?;
            continue;
        } else if upper.starts_with("RCPT TO:") {
            current.to.push(angle(&line));
            b"250 ok\r\n"
        } else if upper == "DATA" {
            writer.write_all(b"354 go ahead\r\n").await?;
            let mut data = Vec::new();
            while let Some(line) = lines.next_line().await? {
                if line == "." {
                    break;
                }
                // Transparency: a line starting with a dot had one added.
                let line = line.strip_prefix('.').unwrap_or(&line);
                data.extend_from_slice(line.as_bytes());
                data.extend_from_slice(b"\r\n");
            }
            current.data = data;
            store.lock().unwrap().push(std::mem::take(&mut current));
            b"250 queued\r\n"
        } else if upper == "QUIT" {
            writer.write_all(b"221 bye\r\n").await?;
            return Ok(());
        } else if upper == "RSET" || upper == "NOOP" {
            b"250 ok\r\n"
        } else {
            b"500 what?\r\n"
        };
        writer.write_all(reply).await?;
    }
    Ok(())
}

// ---- The sender under test --------------------------------------------------

fn settings(addr: SocketAddr, tls: TlsMode) -> SmtpSettings {
    SmtpSettings {
        host: addr.ip().to_string(),
        port: addr.port(),
        tls,
        credentials: Some((USERNAME.to_owned(), Secret::new(PASSWORD.to_owned()))),
        from: FROM.to_owned(),
        timeout: Duration::from_secs(10),
    }
}

fn sender(addr: SocketAddr, tls: TlsMode) -> Arc<SmtpSender> {
    Arc::new(SmtpSender::new(settings(addr, tls), Wording::embedded().unwrap()).unwrap())
}

fn delivery(sender: Arc<SmtpSender>, rules: DeliveryRules) -> Delivery {
    Delivery {
        sender,
        wording: Wording::embedded().unwrap(),
        web_origin: WEB_ORIGIN.to_owned(),
        rules,
    }
}

fn email(to: &str, reference: i64) -> Email {
    Email {
        to: to.to_owned(),
        subject: "A test".to_owned(),
        body: "Nothing to see.".to_owned(),
        html: None,
        reference,
        key: uuid::Uuid::new_v4(),
    }
}

/// The guard keeps the other outbox tests waiting until this one is done.
async fn app() -> (App, MutexGuard<'static, ()>) {
    app_in_turn(TURN.lock().await).await
}

/// [`app`], for a test that already has its turn.
async fn app_in_turn(turn: MutexGuard<'static, ()>) -> (App, MutexGuard<'static, ()>) {
    let app = App::start(DATABASE).await;
    sqlx::query("DELETE FROM outbox")
        .execute(&app.db)
        .await
        .unwrap();
    (app, turn)
}

async fn speaks(app: &App, user: &User, language: &str) {
    sqlx::query("UPDATE account SET language = $2 WHERE id = $1")
        .bind(user.id)
        .bind(language)
        .execute(&app.db)
        .await
        .unwrap();
}

/// What the outbox holds: (attempts, last_error) per row, oldest first.
async fn outbox(app: &App) -> Vec<(i32, Option<String>)> {
    sqlx::query_as("SELECT attempts, last_error FROM outbox ORDER BY id")
        .fetch_all(&app.db)
        .await
        .unwrap()
}

/// The email a notice makes for an exchange, in a language.
fn rendered(language: &str, notice: Notice, deal: &Deal, code: &str) -> Rendered {
    let link = format!("{WEB_ORIGIN}/exchanges/{}", deal.exchange);
    let links = Links {
        exchange: &link,
        record: &format!("{link}/record"),
    };
    Wording::embedded()
        .unwrap()
        .email(language, notice, code, links)
}

/// The subject a notice has for an exchange, in a language.
fn subject(language: &str, notice: Notice, deal: &Deal, code: &str) -> String {
    rendered(language, notice, deal, code).subject
}

/// Checks that a message is `multipart/alternative` with a UTF-8 plain text
/// part and then a UTF-8 HTML part, and returns the two.
fn alternatives(message: &Received) -> (String, String) {
    let parsed = message.parsed();
    let ctype = parsed.content_type().expect("a content type");
    assert_eq!(
        (ctype.ctype(), ctype.subtype()),
        ("multipart", Some("alternative"))
    );
    // The root, then the two alternatives, text first.
    assert_eq!(parsed.parts.len(), 3);
    for (index, subtype) in [(1, "plain"), (2, "html")] {
        let part = parsed.part(index).unwrap();
        let ctype = part.content_type().expect("a part's content type");
        assert_eq!((ctype.ctype(), ctype.subtype()), ("text", Some(subtype)));
        assert_eq!(
            ctype.attribute("charset").map(str::to_ascii_lowercase),
            Some("utf-8".to_owned())
        );
    }
    let text = parsed.body_text(0).unwrap().replace("\r\n", "\n");
    let html = parsed.body_html(0).unwrap().replace("\r\n", "\n");
    (text.trim_end().to_owned(), html)
}

// ---- Tests ------------------------------------------------------------------

#[tokio::test]
async fn a_notification_arrives_as_a_message_in_the_recipients_language() {
    let (app, _turn) = app().await;
    let server = Server::start(Behavior::Serve).await;

    // Ana reads Spanish; Ben keeps the default. Getting to an active deal
    // tells Ana someone claimed the link, tells Ben he was confirmed, and
    // tells both that the agreement is in force.
    let ana = app.user("Ana").await;
    speaks(&app, &ana, "es").await;
    let deal = app.active_between(ana, app.user("Ben").await).await;
    let code = app.view(&deal.ana, &deal.exchange).await["display_code"]
        .as_str()
        .unwrap()
        .to_owned();

    let delivered = deliver_due(
        &app.db,
        &delivery(sender(server.addr, TlsMode::None), DeliveryRules::default()),
        OffsetDateTime::now_utc() + time::Duration::seconds(5),
    )
    .await
    .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (4, 0));
    let keys: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT delivery_key FROM outbox")
        .fetch_all(&app.db)
        .await
        .unwrap();

    let received = server.received();
    let got: BTreeSet<(String, String)> = received
        .iter()
        .map(|message| (message.to.join(","), message.subject()))
        .collect();
    let want: BTreeSet<(String, String)> = [
        (
            deal.ana.email.clone(),
            subject("es", Notice::InvitationClaimedUnconfirmed, &deal, &code),
        ),
        (
            deal.ana.email.clone(),
            subject("es", Notice::AgreementInForce, &deal, &code),
        ),
        (
            deal.ben.email.clone(),
            subject("en", Notice::CounterpartyConfirmed, &deal, &code),
        ),
        (
            deal.ben.email.clone(),
            subject("en", Notice::AgreementInForce, &deal, &code),
        ),
    ]
    .into_iter()
    .collect();
    assert_eq!(got, want);

    // The envelope and the headers agree, the body is the wording's, and
    // each message is identified by the outbox row it is.
    for message in &received {
        let parsed = message.parsed();
        assert_eq!(message.from, "no-reply@example.test");
        assert_eq!(
            parsed.from().and_then(|a| a.first()?.address()),
            Some("no-reply@example.test")
        );
        assert_eq!(
            parsed.to().and_then(|a| a.first()?.address()),
            Some(message.to[0].as_str())
        );
        assert!(
            message
                .body()
                .contains(&format!("{WEB_ORIGIN}/exchanges/{}", deal.exchange)),
            "{}",
            message.body()
        );
        // The outbox row's own random key, never its ID.
        let id = parsed.message_id().unwrap();
        let key = id
            .strip_prefix("outbox-")
            .and_then(|rest| rest.strip_suffix("@example.test"))
            .unwrap_or_else(|| panic!("{id}"));
        assert!(keys.contains(&key.parse().unwrap()), "{id}");
        // The credentials went to the server, in the one place they belong.
        assert_eq!(
            message.auth.as_deref(),
            Some(base64(&format!("\0{USERNAME}\0{PASSWORD}")).as_str())
        );
    }
    // And the one with Spanish in it is Spanish in the body too.
    let in_force = received
        .iter()
        .find(|m| m.to[0] == deal.ana.email && m.subject().contains(&code))
        .unwrap();
    assert!(
        in_force.body().contains("Abre el yup"),
        "{}",
        in_force.body()
    );

    // Each message carries the text exactly as the wording has it, and the
    // same message as HTML beside it.
    let sent = [
        ("es", Notice::InvitationClaimedUnconfirmed),
        ("es", Notice::AgreementInForce),
        ("en", Notice::CounterpartyConfirmed),
        ("en", Notice::AgreementInForce),
    ];
    for message in &received {
        let (text, html) = alternatives(message);
        let want = sent
            .iter()
            .map(|(language, notice)| rendered(language, *notice, &deal, &code))
            .find(|want| want.subject == message.subject())
            .expect("a message that was meant to be sent");
        assert_eq!(text, want.body);
        assert_eq!(html.trim_end(), want.html.trim_end());
    }
    let (_, html) = alternatives(in_force);
    assert!(html.contains("<html lang=\"es\""), "{html}");
}

#[tokio::test]
async fn the_tls_mode_is_honored() {
    // Every test here takes turns: one of them reads back what is logged,
    // and a log subscriber set for one test's thread misses events while
    // other threads are registering the same call sites.
    let _turn = TURN.lock().await;
    let server = Server::start(Behavior::Serve).await;
    let to = "someone@example.test";

    // Plain text is what this server speaks.
    EmailSender::send(&*sender(server.addr, TlsMode::None), &email(to, 1))
        .await
        .unwrap();
    assert_eq!(server.received().len(), 1);

    // STARTTLS required, and the server does not offer it: nothing is sent.
    let error = EmailSender::send(&*sender(server.addr, TlsMode::StartTls), &email(to, 2))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("STARTTLS"), "{error:#}");
    assert_eq!(server.received().len(), 1);

    // TLS from the first byte, and the server answers in plain text: the
    // handshake fails and nothing is sent.
    let error = EmailSender::send(&*sender(server.addr, TlsMode::Tls), &email(to, 3))
        .await
        .unwrap_err();
    assert!(!format!("{error:#}").contains(PASSWORD), "{error:#}");
    assert_eq!(server.received().len(), 1);
}

#[tokio::test]
async fn a_refused_connection_is_a_failed_send_and_the_outbox_retries() {
    let (app, _turn) = app().await;
    app.active().await;
    let queued = outbox(&app).await.len();
    assert_eq!(queued, 4);

    let nobody = Server::refused().await;
    let now = OffsetDateTime::now_utc();
    let delivered = deliver_due(
        &app.db,
        &delivery(sender(nobody, TlsMode::None), DeliveryRules::default()),
        now,
    )
    .await
    .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (0, queued));
    for (attempts, error) in outbox(&app).await {
        assert_eq!(attempts, 1);
        let error = error.expect("the failure is recorded");
        assert!(!error.contains(PASSWORD), "{error}");
    }

    // A server comes up; the next pass after the backoff sends them all.
    let server = Server::start(Behavior::Serve).await;
    let delivered = deliver_due(
        &app.db,
        &delivery(sender(server.addr, TlsMode::None), DeliveryRules::default()),
        now + time::Duration::minutes(2),
    )
    .await
    .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (queued, 0));
    assert_eq!(server.received().len(), queued);
    assert!(outbox(&app).await.iter().all(|(_, error)| error.is_none()));
}

#[tokio::test]
async fn a_send_that_hangs_is_cut_off_by_the_delivery_timeout() {
    let (app, _turn) = app().await;
    app.active().await;
    let silent = Server::start(Behavior::Hang).await;

    let rules = DeliveryRules {
        send_timeout: Duration::from_millis(300),
        ..DeliveryRules::default()
    };
    let started = Instant::now();
    let delivered = deliver_due(
        &app.db,
        &delivery(sender(silent.addr, TlsMode::None), rules),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (0, 4));
    assert!(started.elapsed() < Duration::from_secs(5));
    for (_, error) in outbox(&app).await {
        assert!(
            error
                .as_deref()
                .unwrap_or("")
                .starts_with("no answer within"),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn a_one_time_code_goes_by_email_in_the_language_asked_for_and_not_by_sms() {
    let _turn = TURN.lock().await;
    let server = Server::start(Behavior::Serve).await;
    let sender = sender(server.addr, TlsMode::None);
    let wording = Wording::embedded().unwrap();

    let ana = Identifier::parse("ana@example.test").unwrap();
    CodeSender::send(
        &*sender,
        CodeMessage {
            to: &ana,
            code: "123456",
            purpose: Purpose::SignIn,
            language: "es-MX",
        },
    )
    .await
    .unwrap();
    let delete = Identifier::parse("ben@example.test").unwrap();
    CodeSender::send(
        &*sender,
        CodeMessage {
            to: &delete,
            code: "654321",
            purpose: Purpose::DeleteAccount,
            language: "en",
        },
    )
    .await
    .unwrap();

    let received = server.received();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].to, ["ana@example.test"]);
    assert_eq!(
        received[0].subject(),
        wording.code_email("es", Purpose::SignIn, "123456").subject
    );
    assert_eq!(
        received[0].body().replace("\r\n", "\n").trim_end(),
        wording.code_email("es", Purpose::SignIn, "123456").body
    );
    assert_eq!(
        received[1].subject(),
        wording
            .code_email("en", Purpose::DeleteAccount, "654321")
            .subject
    );
    assert!(received[1].body().contains("654321"));
    // Both as text and as HTML, the text unchanged.
    for (message, language, purpose, code) in [
        (&received[0], "es", Purpose::SignIn, "123456"),
        (&received[1], "en", Purpose::DeleteAccount, "654321"),
    ] {
        let want = wording.code_email(language, purpose, code);
        let (text, html) = alternatives(message);
        assert_eq!(text, want.body);
        assert_eq!(html.trim_end(), want.html.trim_end());
        assert!(html.contains(&format!(">{code}</span>")), "{html}");
    }

    // A phone number cannot be reached this way, and the code is not sent
    // anywhere else either.
    let phone = Identifier::parse("+15551234567").unwrap();
    let error = CodeSender::send(
        &*sender,
        CodeMessage {
            to: &phone,
            code: "111111",
            purpose: Purpose::SignIn,
            language: "en",
        },
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("SMS"), "{error:#}");
    assert_eq!(server.received().len(), 2);
}

/// Standard base64 with padding, enough to check an `AUTH PLAIN` credential.
fn base64(text: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut word = [0u8; 3];
        word[..chunk.len()].copy_from_slice(chunk);
        let n = u32::from_be_bytes([0, word[0], word[1], word[2]]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Everything logged, as bytes.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn refused_credentials_are_an_outage_not_counted_against_the_message() {
    let turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    let subscriber =
        telemetry::subscriber(LogFormat::Text, EnvFilter::new("trace"), false, move || {
            writer.clone()
        });
    let _guard = tracing::subscriber::set_default(subscriber);
    let (app, _turn) = app_in_turn(turn).await;
    app.active().await;
    let server = Server::start(Behavior::WrongPassword).await;
    let rules = DeliveryRules::default();
    let now = OffsetDateTime::now_utc();
    let delivered = deliver_due(
        &app.db,
        &delivery(sender(server.addr, TlsMode::None), rules.clone()),
        now,
    )
    .await
    .unwrap();
    assert_eq!(
        (delivered.sent, delivered.failed, delivered.given_up),
        (0, 4, 0)
    );
    for (attempts, error) in outbox(&app).await {
        assert_eq!(attempts, 0);
        assert!(error.unwrap().contains("reply code 535"));
    }
    let log = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("refused the credentials"), "{log}");
    assert!(!log.contains(PASSWORD), "{log}");

    // A day on, given up.
    let delivered = deliver_due(
        &app.db,
        &delivery(sender(server.addr, TlsMode::None), rules.clone()),
        now + rules.outage_max_age + time::Duration::minutes(1),
    )
    .await
    .unwrap();
    assert_eq!(delivered.given_up, 4);
    for (attempts, _) in outbox(&app).await {
        assert_eq!(attempts, rules.max_attempts);
    }
}

#[tokio::test]
async fn a_refused_recipient_leaves_no_part_of_their_address_in_the_outbox_or_the_log() {
    let turn = TURN.lock().await;
    let log = Log::default();
    let writer = log.clone();
    let subscriber =
        telemetry::subscriber(LogFormat::Text, EnvFilter::new("trace"), false, move || {
            writer.clone()
        });
    // A test runs on one thread, so this holds for everything it does.
    let _guard = tracing::subscriber::set_default(subscriber);

    let (app, _turn) = app_in_turn(turn).await;
    let deal = app.active().await;
    let server = Server::start(Behavior::Reject).await;
    let sender = sender(server.addr, TlsMode::None);

    let delivered = deliver_due(
        &app.db,
        &delivery(sender.clone(), DeliveryRules::default()),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    assert_eq!((delivered.sent, delivered.failed), (0, 4));

    // A one-time code refused the same way.
    let code_error = CodeSender::send(
        &*sender,
        CodeMessage {
            to: &Identifier::parse(&deal.ana.email).unwrap(),
            code: "123456",
            purpose: Purpose::SignIn,
            language: "en",
        },
    )
    .await
    .unwrap_err();
    let code_error = format!("{code_error:#}");
    assert!(code_error.contains("reply code 550"), "{code_error}");

    let errors: Vec<String> = outbox(&app)
        .await
        .into_iter()
        .map(|(_, error)| error.expect("the failure is recorded"))
        .collect();
    let log = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("reply code 550"), "{log}");
    for error in &errors {
        assert!(error.contains("reply code 550"), "{error}");
    }
    for person in [&deal.ana, &deal.ben] {
        let (local, domain) = person.email.split_once('@').unwrap();
        for part in [person.email.as_str(), local, domain] {
            for error in &errors {
                assert!(!error.contains(part), "{part} in {error}");
            }
            assert!(!code_error.contains(part), "{part} in {code_error}");
            assert!(!log.contains(part), "{part} in the log:\n{log}");
        }
    }
}
