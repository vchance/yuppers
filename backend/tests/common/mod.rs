//! Shared by the exchange API tests: a throwaway database, and helpers that
//! drive the HTTP router as signed-in people.
//!
//! Agreement history cannot be deleted, by design, so these tests do not
//! clean up after themselves. Each test binary instead gets its own database,
//! dropped and recreated at the start of every run. The schema owner needs
//! the CREATEDB privilege for that (see the README).

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, Method, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::postgres::{PgPool, PgPoolOptions};
use tokio::sync::OnceCell;
use tower::ServiceExt;
use uuid::Uuid;
use yuppers_backend::auth::{AuthRules, CodeSender, LogSender, generate_token, token_hash};
use yuppers_backend::client_version::MinimumClientVersions;
use yuppers_backend::db;
use yuppers_backend::domain::Rules;
use yuppers_backend::http::{self, AppState, Settings, TrustedProxies};
use yuppers_backend::metrics::HttpMetrics;
use yuppers_backend::notifications::smtp::Secret;
use yuppers_backend::wallet::Wallet;

pub const CONSENT_VERSION: &str = "test-1";

/// The address every test request appears to come from.
pub const PEER: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, 23)),
    51234,
);

fn env(name: &str) -> String {
    dotenvy::dotenv().ok();
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set; see .env.example"))
}

fn with_database(url: &str, database: &str) -> String {
    let (base, _) = url
        .rsplit_once('/')
        .expect("a connection URL ends in /database");
    format!("{base}/{database}")
}

async fn connect(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .unwrap_or_else(|error| panic!("cannot connect to {url}: {error}"))
}

/// A short tag for this checkout. Several working copies of the repository
/// can run their tests against one PostgreSQL at the same time; the tag keeps
/// each from dropping the database another is using.
fn checkout_tag() -> String {
    // FNV-1a over the path of this copy of the backend.
    let hash = env!("CARGO_MANIFEST_DIR")
        .bytes()
        .fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
        });
    format!("{hash:08x}")
}

/// Creates the binary's database once per run and returns the owner and
/// application connection strings for it.
async fn database(name: &'static str) -> &'static (String, String) {
    static URLS: OnceCell<(String, String)> = OnceCell::const_new();
    URLS.get_or_init(|| async move {
        let name = &format!("{name}_{}", checkout_tag());
        let owner_url = env("MIGRATION_DATABASE_URL");
        let admin = connect(&owner_url).await;
        // A run that has just ended may still be closing its application
        // connections to the database. The schema owner may not end another
        // role's connections, so `FORCE` is refused until they are gone,
        // which takes moments: running a test many times in a row would
        // otherwise fail now and then before the test even starts.
        let drop = format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)");
        for tries_left in (0..50).rev() {
            match sqlx::query(sqlx::AssertSqlSafe(drop.clone()))
                .execute(&admin)
                .await
            {
                Ok(_) => break,
                Err(error)
                    if tries_left > 0
                        && error
                            .as_database_error()
                            .is_some_and(|e| e.code().as_deref() == Some("42501")) =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Err(error) => panic!("the schema owner can drop its test database: {error}"),
            }
        }
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("the schema owner can create databases (ALTER ROLE exchange CREATEDB)");
        admin.close().await;

        let owner_url = with_database(&owner_url, name);
        let owner = connect(&owner_url).await;
        db::MIGRATOR.run(&owner).await.expect("migrations apply");
        owner.close().await;

        (owner_url, with_database(&env("DATABASE_URL"), name))
    })
    .await
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
    /// The body as it came, for one that is not JSON.
    pub bytes: Vec<u8>,
}

impl Reply {
    pub fn code(&self) -> &str {
        self.body["code"].as_str().unwrap_or("")
    }

    /// The body of a successful reply.
    pub fn ok(self) -> Value {
        assert_eq!(self.status, StatusCode::OK, "{}", self.body);
        self.body
    }

    #[track_caller]
    pub fn refused(&self, status: StatusCode, code: &str) {
        assert_eq!((self.status, self.code()), (status, code), "{}", self.body);
    }
}

/// A signed-in person.
#[derive(Clone)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    token: String,
}

pub struct App {
    router: Router,
    /// Connected as the application role, like the service itself.
    pub db: PgPool,
    /// Connected as the schema owner, for looking at what was stored.
    pub owner: PgPool,
    /// The schema owner's connection string, for a command run as the owner.
    pub owner_url: String,
    /// The application role's connection string, for a process run as the
    /// service.
    pub app_url: String,
    pub rules: Rules,
    /// What the router counted of the requests made through it.
    pub metrics: Arc<HttpMetrics>,
}

impl App {
    pub async fn start(database_name: &'static str) -> Self {
        Self::start_with(database_name, Rules::default()).await
    }

    pub async fn start_with(database_name: &'static str, rules: Rules) -> Self {
        Self::start_sending(database_name, rules, Arc::new(LogSender)).await
    }

    /// With one-time codes handed to `code_sender`, for tests that read them.
    pub async fn start_sending(
        database_name: &'static str,
        rules: Rules,
        code_sender: Arc<dyn CodeSender>,
    ) -> Self {
        Self::start_configured(
            database_name,
            rules,
            code_sender,
            TrustedProxies::none(),
            MinimumClientVersions::default(),
            None,
        )
        .await
    }

    /// Issuing Wallet passes for the platforms `wallet` has.
    pub async fn start_with_wallet(database_name: &'static str, wallet: Arc<Wallet>) -> Self {
        Self::start_configured(
            database_name,
            Rules::default(),
            Arc::new(LogSender),
            TrustedProxies::none(),
            MinimumClientVersions::default(),
            Some(wallet),
        )
        .await
    }

    /// With the given view of proxy headers. Every request the helpers make
    /// arrives from [`PEER`].
    pub async fn start_behind(
        database_name: &'static str,
        rules: Rules,
        code_sender: Arc<dyn CodeSender>,
        proxies: TrustedProxies,
    ) -> Self {
        Self::start_configured(
            database_name,
            rules,
            code_sender,
            proxies,
            MinimumClientVersions::default(),
            None,
        )
        .await
    }

    /// Requiring clients to be at least the given versions.
    pub async fn start_requiring(
        database_name: &'static str,
        min_client_versions: MinimumClientVersions,
    ) -> Self {
        Self::start_configured(
            database_name,
            Rules::default(),
            Arc::new(LogSender),
            TrustedProxies::none(),
            min_client_versions,
            None,
        )
        .await
    }

    /// With the given sign-in rules and code sender, and saying whether push
    /// notifications are sent (`GET /v1/meta`).
    pub async fn start_messaging(
        database_name: &'static str,
        auth: AuthRules,
        code_sender: Arc<dyn CodeSender>,
        push_notifications: bool,
    ) -> Self {
        Self::start_with_settings(
            database_name,
            Rules::default(),
            code_sender,
            TrustedProxies::none(),
            MinimumClientVersions::default(),
            (auth, push_notifications),
            None,
        )
        .await
    }

    /// Texting agreement updates, with these sign-in rules and code sender,
    /// and checking Twilio's signature on the inbound-message webhook with
    /// `webhook_token`. The service's web origin is `https://app.test`.
    pub async fn start_texting(
        database_name: &'static str,
        auth: AuthRules,
        code_sender: Arc<dyn CodeSender>,
        webhook_token: &str,
    ) -> Self {
        Self::start_full(
            database_name,
            Rules::default(),
            code_sender,
            TrustedProxies::none(),
            MinimumClientVersions::default(),
            (auth, false),
            None,
            Some(webhook_token),
        )
        .await
    }

    async fn start_configured(
        database_name: &'static str,
        rules: Rules,
        code_sender: Arc<dyn CodeSender>,
        proxies: TrustedProxies,
        min_client_versions: MinimumClientVersions,
        wallet: Option<Arc<Wallet>>,
    ) -> Self {
        Self::start_with_settings(
            database_name,
            rules,
            code_sender,
            proxies,
            min_client_versions,
            (AuthRules::default(), false),
            wallet,
        )
        .await
    }

    async fn start_with_settings(
        database_name: &'static str,
        rules: Rules,
        code_sender: Arc<dyn CodeSender>,
        proxies: TrustedProxies,
        min_client_versions: MinimumClientVersions,
        messaging: (AuthRules, bool),
        wallet: Option<Arc<Wallet>>,
    ) -> Self {
        Self::start_full(
            database_name,
            rules,
            code_sender,
            proxies,
            min_client_versions,
            messaging,
            wallet,
            None,
        )
        .await
    }

    /// With every setting. `webhook_token` set means text messages are sent:
    /// agreement updates can be turned on, and the inbound webhook checks
    /// signatures with it.
    #[allow(clippy::too_many_arguments)]
    async fn start_full(
        database_name: &'static str,
        rules: Rules,
        code_sender: Arc<dyn CodeSender>,
        proxies: TrustedProxies,
        min_client_versions: MinimumClientVersions,
        (auth, push_notifications): (AuthRules, bool),
        wallet: Option<Arc<Wallet>>,
        webhook_token: Option<&str>,
    ) -> Self {
        let (owner_url, app_url) = database(database_name).await;
        let db = connect(app_url).await;
        let metrics = Arc::new(HttpMetrics::default());
        let state = AppState {
            db: db.clone(),
            settings: Arc::new(Settings {
                app_secret: b"test-secret-test-secret-test-secret".to_vec(),
                web_origin: "https://app.test".to_owned(),
                auth,
                rules: rules.clone(),
                consent_version: CONSENT_VERSION.to_owned(),
                proxies,
                min_client_versions,
                app_links: Default::default(),
                push_notifications,
                build: Default::default(),
                sms_updates: webhook_token.is_some(),
                sms_webhook_token: webhook_token.map(|token| Secret::new(token.to_owned())),
            }),
            code_sender,
            metrics: metrics.clone(),
        };
        let router = match wallet {
            Some(wallet) => http::router_with_wallet(state, None, wallet),
            None => http::router(state, None),
        };
        Self {
            router,
            db,
            metrics,
            owner: connect(owner_url).await,
            owner_url: owner_url.clone(),
            app_url: app_url.clone(),
            rules,
        }
    }

    /// An account with a name and confirmed age, and a session for it.
    pub async fn user(&self, name: &str) -> User {
        self.user_with(name, true).await
    }

    pub async fn user_with(&self, name: &str, adult: bool) -> User {
        let email = format!("{}@example.test", Uuid::new_v4().simple());
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO account (email, display_name, adult_confirmed_at)
             VALUES ($1, $2, CASE WHEN $3 THEN now() END)
             RETURNING id",
        )
        .bind(&email)
        .bind(name)
        .bind(adult)
        .fetch_one(&self.db)
        .await
        .unwrap();

        let token = generate_token();
        sqlx::query(
            "INSERT INTO account_session
                (account_id, token_hash, auth_method, authenticated_at, expires_at)
             VALUES ($1, $2, 'EMAIL_OTP', now(), now() + interval '1 day')",
        )
        .bind(id)
        .bind(token_hash(&token).as_slice())
        .execute(&self.db)
        .await
        .unwrap();

        User { id, email, token }
    }

    pub async fn call(
        &self,
        user: Option<&User>,
        method: Method,
        path: &str,
        body: Option<Value>,
        headers: &[(&'static str, &str)],
    ) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            // What the listener would know about the connection.
            .extension(ConnectInfo(PEER));
        if let Some(user) = user {
            request = request.header(AUTHORIZATION, format!("Bearer {}", user.token));
        }
        for (name, value) in headers {
            request = request.header(HeaderName::from_static(name), *value);
        }
        let request = match body {
            Some(body) => request
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            bytes: bytes.to_vec(),
        }
    }

    /// A request with a body of any type and no session, such as a
    /// provider's webhook makes, with at most one extra header.
    pub async fn raw(
        &self,
        method: Method,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
        header: Option<(&'static str, &str)>,
    ) -> Reply {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .extension(ConnectInfo(PEER))
            .header(CONTENT_TYPE, content_type);
        if let Some((name, value)) = header {
            request = request.header(HeaderName::from_static(name), value);
        }
        let request = request.body(Body::from(body)).unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Reply {
            status,
            headers,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            bytes: bytes.to_vec(),
        }
    }

    pub async fn get(&self, user: &User, path: &str) -> Reply {
        self.call(Some(user), Method::GET, path, None, &[]).await
    }

    pub async fn post(&self, user: &User, path: &str, body: Value) -> Reply {
        self.call(Some(user), Method::POST, path, Some(body), &[])
            .await
    }

    pub async fn view(&self, user: &User, exchange: &str) -> Value {
        self.get(user, &format!("/v1/exchanges/{exchange}"))
            .await
            .ok()
    }

    /// Creates a draft and returns its ID.
    pub async fn draft(&self, user: &User) -> String {
        let view = self
            .post(
                user,
                "/v1/exchanges",
                json!({ "timezone": "America/Chicago" }),
            )
            .await
            .ok();
        view["id"].as_str().unwrap().to_owned()
    }

    /// Sends a revision at the exchange's current version.
    pub async fn send(&self, user: &User, exchange: &str, terms: Value) -> Reply {
        let version = self.view(user, exchange).await["version"].clone();
        self.post(
            user,
            &format!("/v1/exchanges/{exchange}/revisions"),
            json!({ "expected_version": version, "terms": terms, "consent": consent() }),
        )
        .await
    }

    /// Runs a command at the exchange's current version.
    pub async fn command(&self, user: &User, exchange: &str, command: Value) -> Reply {
        let version = self.view(user, exchange).await["version"].clone();
        self.post(
            user,
            &format!("/v1/exchanges/{exchange}/commands"),
            json!({ "expected_version": version, "command": command }),
        )
        .await
    }

    pub async fn act(
        &self,
        user: &User,
        exchange: &str,
        contribution: Uuid,
        action: &str,
    ) -> Reply {
        self.command(
            user,
            exchange,
            json!({ "type": "CONTRIBUTION", "contribution": contribution, "action": action }),
        )
        .await
    }

    /// Ana proposes the fence job to Ben; nobody has opened the link yet.
    pub async fn negotiating(&self) -> Deal {
        let ana = self.user("Ana").await;
        let ben = self.user("Ben").await;
        self.negotiating_between(ana, ben).await
    }

    /// [`negotiating`](Self::negotiating) between two given people.
    pub async fn negotiating_between(&self, ana: User, ben: User) -> Deal {
        let exchange = self.draft(&ana).await;
        let (repair, payment) = (Uuid::new_v4(), Uuid::new_v4());

        let sent = self
            .send(&ana, &exchange, fence_job(repair, payment))
            .await
            .ok();
        Deal {
            ana,
            ben,
            exchange,
            repair,
            payment,
            revision: sent["exchange"]["open_revision"]["id"]
                .as_str()
                .unwrap()
                .to_owned(),
            invitation: sent["invitation_token"].as_str().unwrap().to_owned(),
        }
    }

    /// Ben has claimed the link, Ana has confirmed him, and Ben has accepted.
    pub async fn active(&self) -> Deal {
        let ana = self.user("Ana").await;
        let ben = self.user("Ben").await;
        self.active_between(ana, ben).await
    }

    /// [`active`](Self::active) between two given people.
    pub async fn active_between(&self, ana: User, ben: User) -> Deal {
        let deal = self.negotiating_between(ana, ben).await;
        self.post(
            &deal.ben,
            "/v1/invitations/claim",
            json!({ "token": deal.invitation }),
        )
        .await
        .ok();
        self.command(
            &deal.ana,
            &deal.exchange,
            json!({ "type": "CONFIRM_COUNTERPARTY" }),
        )
        .await
        .ok();
        let view = self
            .command(&deal.ben, &deal.exchange, accept(&deal.revision))
            .await
            .ok();
        assert_eq!(view["state"], "ACTIVE");
        deal
    }
}

pub struct Deal {
    pub ana: User,
    pub ben: User,
    pub exchange: String,
    /// Owed by Ana.
    pub repair: Uuid,
    /// Owed by Ben once the repair is accepted.
    pub payment: Uuid,
    pub revision: String,
    pub invitation: String,
}

pub fn consent() -> Value {
    json!({ "language": "en", "version": CONSENT_VERSION })
}

pub fn accept(revision: &str) -> Value {
    json!({ "type": "ACCEPT", "revision": revision, "consent": consent() })
}

/// A fence repair by A, paid for by B once it is accepted.
pub fn fence_job(repair: Uuid, payment: Uuid) -> Value {
    json!({
        "party_a_name": "Ana Ruiz",
        "party_b_name": "Ben Ortiz",
        "terms": "Repair the back fence.",
        "contributions": [
            {
                "id": repair,
                "from": "A",
                "type": "SERVICE",
                "description": "Repair the back fence",
                "quantity": null,
                "due": { "kind": "ON_AGREEMENT" },
                "completion_criteria": null,
                "required": true,
                "amount_minor": null,
            },
            {
                "id": payment,
                "from": "B",
                "type": "MONEY",
                "description": "Payment on completion",
                "quantity": null,
                "due": { "kind": "AFTER_CONTRIBUTION", "contribution": repair },
                "completion_criteria": null,
                "required": true,
                "amount_minor": 40000,
            },
        ],
    })
}
