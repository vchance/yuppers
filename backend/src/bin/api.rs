use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use yuppers_backend::build_info::BuildInfo;
use yuppers_backend::config::ApiConfig;
use yuppers_backend::domain::Rules;
use yuppers_backend::http::{self, AppState, Settings, WebApp};
use yuppers_backend::metrics::{self, HttpMetrics, Text};
use yuppers_backend::notifications::outbox::DeliveryRules;
use yuppers_backend::notifications::sms_updates;
use yuppers_backend::wallet::Wallet;
use yuppers_backend::{db, shutdown, telemetry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    telemetry::init()?;
    BuildInfo::current().log_start("api");
    let config = ApiConfig::from_env()?;
    if !config.proxies.trusts_a_header() {
        tracing::warn!(
            "TRUSTED_PROXY_HEADER is not set, so each connection's peer is taken as the \
             requester; behind a reverse proxy or CDN every user then shares the proxy's \
             address and its sign-in limits. Name the proxy's header if there is one."
        );
    }

    let app_link_files = config.app_links.served();
    if !app_link_files.is_empty() {
        tracing::info!(files = ?app_link_files, "serving app link association files");
    }

    let (sms, sms_cap) = (config.sms, config.auth.sms_codes_per_hour);
    // Agreement updates are queued wherever an exchange changes.
    sms_updates::configure(sms, config.sms_updates_per_day);
    if sms {
        tracing::info!(
            cap = sms_cap,
            "codes for phone numbers, and agreement updates, go by text message, at most this many an hour"
        );
        if config.sms_webhook_token.is_none() {
            tracing::warn!(
                "no auth token checks Twilio's requests to /v1/sms/inbound, so STOP and START \
                 replies are refused there; set SMS_WEBHOOK_AUTH_TOKEN (docs/deploy-render.md)"
            );
        }
    }

    let wallet =
        Arc::new(Wallet::new(&config.wallet, &config.web_origin).context("Wallet passes")?);
    if !wallet.platforms().is_empty() {
        tracing::info!(platforms = ?wallet.platforms(), "issuing Wallet passes");
    }

    let state = AppState {
        db: db::pool(&config.database_url)?,
        settings: Arc::new(Settings {
            app_secret: config.app_secret,
            web_origin: config.web_origin,
            auth: config.auth,
            rules: Rules::default(),
            // A stand-in until counsel-approved consent wording exists
            // (DESIGN.md §14.1); the real version replaces it then.
            consent_version: "draft-1".to_owned(),
            proxies: config.proxies,
            min_client_versions: config.min_client_versions,
            app_links: config.app_links,
            push_notifications: config.push_notifications,
            build: BuildInfo::current().clone(),
            sms_updates: sms,
            sms_webhook_token: config.sms_webhook_token,
        }),
        code_sender: config.code_sender,
        metrics: Arc::new(HttpMetrics::default()),
    };

    // Never on a restored copy whose deletion log is still to be replayed.
    // A database that cannot be reached yet is not refused: readiness says
    // so until it can be, and the check there covers the mark too.
    match db::replay_pending(&state.db).await {
        Ok(true) => anyhow::bail!(db::REPLAY_PENDING),
        Ok(false) => {}
        Err(error) => tracing::warn!(
            error = %yuppers_backend::error::Redacted(&error),
            "could not check whether this database waits for a deletion replay; not ready until it can"
        ),
    }

    let web = match &config.web_dir {
        Some(directory) => {
            let web = WebApp::open(directory)
                .with_context(|| format!("WEB_DIR={}", directory.display()))?;
            tracing::info!(
                directory = %directory.display(),
                languages = ?web.languages(),
                legal = ?web.legal_pages().collect::<Vec<_>>(),
                "serving the web app"
            );
            Some(web)
        }
        None => None,
    };

    // On a listener of its own, and only when asked for (docs/operations.md).
    if let Some(addr) = config.metrics_addr {
        let (requests, db) = (state.metrics.clone(), state.db.clone());
        let wallet = wallet.clone();
        let max_attempts = DeliveryRules::default().max_attempts;
        // Text messages are counted only while they are sent.
        let sms_cap = sms.then_some(sms_cap);
        metrics::serve(addr, move || {
            let (requests, db, wallet) = (requests.clone(), db.clone(), wallet.clone());
            async move {
                let mut text = Text::new();
                BuildInfo::current().render_metrics(&mut text);
                requests.render(&mut text);
                wallet.render_metrics(&mut text);
                metrics::render_pool(&mut text, &db);
                metrics::render_outbox(&mut text, &db, max_attempts).await;
                metrics::render_reports(&mut text, &db).await;
                if let Some(cap) = sms_cap {
                    metrics::render_sms(&mut text, &db, cap).await;
                }
                text.finish()
            }
        })
        .await?;
    }

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!(addr = %config.bind_addr, "api listening");

    // With the peer address of each connection, which is the client's unless
    // a trusted proxy header says otherwise.
    let app = http::router_with_wallet(state, web, wallet)
        .into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            shutdown::signal().await;
            tracing::info!("api shutting down");
        })
        .await?;
    Ok(())
}
