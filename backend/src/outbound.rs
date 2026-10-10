//! Requests the service makes to other services over HTTPS: the push service
//! and the SMS provider (`crate::notifications`).
//!
//! One small client over hyper, with rustls, ring and the webpki roots: the
//! same TLS library, crypto provider and root certificates that SMTP
//! (lettre) and the database (sqlx) already use, so the image needs no
//! OpenSSL and no system certificate store. It does only what the adapters
//! need: a POST with a body, read back whole, within a time limit. It
//! follows no redirect and keeps no cookie.
//!
//! What it never does: write a request or response body, or a header, to a
//! log. Callers decide what of an answer is safe to keep.

use std::time::Duration;

use anyhow::Context;
use axum::body::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::http::{HeaderValue, Method, Request, StatusCode, header};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use tracing::Instrument;

use crate::otel::{SpanExt, SpanKind};

/// The most of an answer that is read. The providers' answers are a few
/// kilobytes; anything far larger is not one of them.
const MAX_RESPONSE: usize = 1024 * 1024;

/// An answer, read whole.
#[derive(Clone, Debug)]
pub struct Answer {
    pub status: StatusCode,
    pub body: Bytes,
}

/// A client for one kind of provider. Cheap to clone; connections are kept
/// and reused between requests.
#[derive(Clone)]
pub struct Client {
    inner: HyperClient<HttpsConnector<HttpConnector>, Full<Bytes>>,
    timeout: Duration,
}

impl Client {
    /// A client whose every request, from connecting to the last byte of the
    /// answer, must be done within `timeout`.
    ///
    /// It speaks HTTPS, and plain HTTP too, which only the tests use: each
    /// adapter is given its provider's `https://` address by the
    /// configuration, and a stand-in on `http://127.0.0.1` by a test.
    pub fn new(timeout: Duration) -> Self {
        let tls = rustls::ClientConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_root_certificates(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        })
        .with_no_client_auth();
        let connector = HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_or_http()
            .enable_http1()
            .build();
        Self {
            inner: HyperClient::builder(TokioExecutor::new()).build(connector),
            timeout,
        }
    }

    /// POSTs `body` to `url` with the given headers, and reads the answer.
    /// An error says what went wrong in general terms and never quotes the
    /// URL's query, the headers or either body.
    ///
    /// In a span of its own (`crate::otel`): the provider's host, the
    /// method and the status it answered with, nothing of the path, which
    /// can name an account at the provider, nor of the bodies.
    pub async fn post(
        &self,
        url: &str,
        headers: &[(header::HeaderName, HeaderValue)],
        body: Vec<u8>,
    ) -> anyhow::Result<Answer> {
        let span = tracing::info_span!("provider request");
        span.otel_kind(SpanKind::Client);
        span.otel_attr("http.request.method", "POST");
        if let Some(host) = url
            .parse::<hyper::Uri>()
            .ok()
            .and_then(|uri| uri.host().map(str::to_owned))
        {
            span.otel_attr("server.address", host);
        }
        let answer = self.send(url, headers, body).instrument(span.clone()).await;
        match &answer {
            Ok(answer) => {
                span.otel_attr("http.response.status_code", answer.status.as_u16());
                if answer.status.is_server_error() {
                    span.otel_error();
                }
            }
            Err(_) => span.otel_error(),
        }
        answer
    }

    /// [`post`](Self::post) without the span: what the telemetry exporter
    /// uses, so that exporting is not itself something to export.
    pub(crate) async fn send(
        &self,
        url: &str,
        headers: &[(header::HeaderName, HeaderValue)],
        body: Vec<u8>,
    ) -> anyhow::Result<Answer> {
        let mut request = Request::builder().method(Method::POST).uri(url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let request = request
            .body(Full::new(Bytes::from(body)))
            .context("the request could not be built")?;
        let exchange = async {
            let response = self
                .inner
                .request(request)
                .await
                .map_err(|_| anyhow::anyhow!("the provider could not be reached"))?;
            let status = response.status();
            let body = Limited::new(response.into_body(), MAX_RESPONSE)
                .collect()
                .await
                .map_err(|_| anyhow::anyhow!("the provider's answer could not be read"))?
                .to_bytes();
            Ok(Answer { status, body })
        };
        tokio::time::timeout(self.timeout, exchange)
            .await
            .map_err(|_| anyhow::anyhow!("the provider did not answer in time"))?
    }
}
