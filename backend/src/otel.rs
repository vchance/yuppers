//! OpenTelemetry export: traces, logs and metrics, pushed over OTLP/HTTP to
//! a collector (docs/operations.md, "Telemetry").
//!
//! # Off unless asked for
//!
//! Nothing here runs unless `OTEL_EXPORTER_OTLP_ENDPOINT` is set: no layer
//! on the log subscriber, no task, no allocation per request or event. The
//! log on standard output is the same either way, and stays the
//! authoritative record; this is the copy that reaches a collector.
//!
//! # The shape, and why
//!
//! OTLP over HTTP with the JSON encoding, written by hand rather than
//! through the `opentelemetry` crates, as the owner's historian does and
//! for the same reasons: the protocol surface used is a POST of one
//! document shape per signal, the SDK is a large dependency tree for the
//! weekly audit to watch, and JSON on the wire can be read with `curl` when
//! a collector rejects something. The collector is the `grafana/otel-lgtm`
//! stack, reached through a Cloudflare Tunnel whose Access service token
//! travels in `OTEL_EXPORTER_OTLP_HEADERS`; the requests are made with the
//! HTTPS client the service already uses for its providers
//! (`crate::outbound`), so the headers go with every signal.
//!
//! Conventions shared with the historian: resource attributes
//! `service.name`, `service.namespace`, `service.version` (and here
//! `deployment.environment`, `service.instance.id` and `vcs.commit`), metric
//! names dotted and lowercase under the project's namespace with UCUM units
//! (`crate::metrics::Name`), cumulative temporality, a push every fifteen
//! seconds for metrics, bounded buffers of 4,096 records sent 256 to a
//! POST, records dropped and counted when the buffer is full or the
//! collector refuses them, never requeued.
//!
//! # Traces and logs
//!
//! A [`Layer`] on the `tracing` subscriber turns every span into an
//! OpenTelemetry span and every event into a log record, with the same
//! fields the log on standard output has. So the rule for what may be
//! logged (`crate::telemetry`) is the rule for what is exported: no body,
//! no query string, no token, no code, no address, no number, no payment
//! handle, decided where each span and event is written. The one field a
//! span has that is not exported is a request's `path`, which can name an
//! exchange; the route's template (`http.route`) goes instead. A span's
//! `trace_id` and `span_id` fields, when it declares them, are its
//! identifiers, so a line on standard output and the exported span agree.
//!
//! # Never blocking a request
//!
//! Recording is a push onto a buffer behind a mutex; exporting happens on a
//! task of its own, every two seconds for spans and logs. A collector that
//! is slow, down or refusing costs a warning at most every five minutes and
//! the records of that interval, nothing else. Shutdown flushes what is
//! buffered, within a few seconds.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, bail};
use hyper::http::{HeaderName, HeaderValue};
use serde_json::{Value as Json, json};
use tokio::sync::oneshot;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::{LookupSpan, Registry, SpanData as _};

use crate::build_info::{BuildInfo, UNKNOWN};
use crate::metrics::{Kind, Name, Text};
use crate::outbound;

/// The collector's base address, such as `http://localhost:4318` or
/// `https://otel.example.com`; the signals' paths (`/v1/traces`,
/// `/v1/logs`, `/v1/metrics`) are added. Unset: nothing is exported.
pub const ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
/// Headers sent with every request, `name=value,name=value`, values
/// percent-encoded if need be: here Cloudflare Access's
/// `CF-Access-Client-Id` and `CF-Access-Client-Secret`. Secrets: never
/// logged, never in an error.
pub const HEADERS: &str = "OTEL_EXPORTER_OTLP_HEADERS";
/// Only `http/json` is spoken; set to anything else, the process refuses to
/// start rather than send what the collector will not read.
pub const PROTOCOL: &str = "OTEL_EXPORTER_OTLP_PROTOCOL";
/// Milliseconds a request to the collector may take. Default 10,000.
pub const TIMEOUT: &str = "OTEL_EXPORTER_OTLP_TIMEOUT";
/// `service.name`. Default `yuppers-api` or `yuppers-worker`.
pub const SERVICE_NAME: &str = "OTEL_SERVICE_NAME";
/// More resource attributes, `key=value,key=value`, merged over the ones
/// the process sets.
pub const RESOURCE_ATTRIBUTES: &str = "OTEL_RESOURCE_ATTRIBUTES";
/// `deployment.environment`: `production`, `development` (the default).
pub const DEPLOYMENT_ENVIRONMENT: &str = "DEPLOYMENT_ENVIRONMENT";

/// How often buffered spans and log records are sent.
pub const FLUSH_EVERY: Duration = Duration::from_secs(2);
/// How often the metrics page is sent.
pub const METRICS_EVERY: Duration = Duration::from_secs(15);
/// The most spans, and the most log records, held between two sends.
pub const QUEUE_MAX: usize = 4_096;
/// The most records in one POST.
pub const BATCH_MAX: usize = 256;
/// How long shutdown waits for the last send.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// How often at most a failing collector is warned about.
const WARN_EVERY: Duration = Duration::from_secs(300);
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Records dropped because the buffer was full or the collector refused
/// them, by signal. Exported with the metrics, so a gap in the traces or
/// logs can be told from a quiet service.
pub const TELEMETRY_DROPPED: Name = Name::new("yuppers.telemetry.dropped", "{record}");

// ---- Configuration -------------------------------------------------------------

/// Which signal a document carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Traces,
    Logs,
    Metrics,
}

impl Signal {
    fn path(self) -> &'static str {
        match self {
            Self::Traces => "/v1/traces",
            Self::Logs => "/v1/logs",
            Self::Metrics => "/v1/metrics",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Traces => "traces",
            Self::Logs => "logs",
            Self::Metrics => "metrics",
        }
    }
}

/// Where and how to export.
#[derive(Clone)]
pub struct Config {
    /// The collector's base address, without a trailing slash.
    pub endpoint: String,
    headers: Vec<(HeaderName, HeaderValue)>,
    /// The resource every signal is attributed to.
    pub resource: Vec<(String, String)>,
    pub timeout: Duration,
}

impl fmt::Debug for Config {
    /// The header names, never their values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("endpoint", &self.endpoint)
            .field(
                "headers",
                &self
                    .headers
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .field("resource", &self.resource)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl Config {
    /// A configuration for `endpoint` with the given resource attributes
    /// and no headers, as the tests build one.
    pub fn new(endpoint: &str, resource: Vec<(String, String)>) -> anyhow::Result<Self> {
        Ok(Self {
            endpoint: parse_endpoint(endpoint)?,
            headers: Vec::new(),
            resource,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// With a header on every request.
    pub fn with_header(mut self, name: &str, value: &str) -> anyhow::Result<Self> {
        self.headers.push(header(name, value)?);
        Ok(self)
    }

    /// With this time limit on every request.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Reads the process's environment. `None` when no endpoint is set, or
    /// `OTEL_SDK_DISABLED=true`: nothing is exported.
    pub fn from_env(process: &str) -> anyhow::Result<Option<Self>> {
        Self::from_lookup(
            &|name| std::env::var(name).ok(),
            process,
            BuildInfo::current(),
        )
    }

    /// Reads the settings from `get`, naming the process (`api`, `worker`)
    /// and the build they describe.
    pub fn from_lookup(
        get: &dyn Fn(&str) -> Option<String>,
        process: &str,
        build: &BuildInfo,
    ) -> anyhow::Result<Option<Self>> {
        let optional = |name: &str| {
            get(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        };
        if optional("OTEL_SDK_DISABLED").is_some_and(|value| value.eq_ignore_ascii_case("true")) {
            return Ok(None);
        }
        let Some(endpoint) = optional(ENDPOINT) else {
            return Ok(None);
        };
        let endpoint = parse_endpoint(&endpoint).with_context(|| ENDPOINT.to_owned())?;
        match optional(PROTOCOL).as_deref() {
            None | Some("http/json") => {}
            Some(other) => bail!(
                "{PROTOCOL}={other} is not spoken: only http/json is (OTLP over HTTP, JSON \
                 encoding), so leave it unset"
            ),
        }
        let headers = parse_headers(optional(HEADERS).as_deref())?;
        let timeout = match optional(TIMEOUT) {
            Some(millis) => Duration::from_millis(
                millis
                    .parse::<u64>()
                    .ok()
                    .filter(|millis| (1..=600_000).contains(millis))
                    .with_context(|| {
                        format!("{TIMEOUT} must be a number of milliseconds, 1 to 600000")
                    })?,
            ),
            None => DEFAULT_TIMEOUT,
        };

        let environment =
            optional(DEPLOYMENT_ENVIRONMENT).unwrap_or_else(|| "development".to_owned());
        let instance = optional("RENDER_INSTANCE_ID")
            .or_else(|| optional("HOSTNAME"))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut resource: Vec<(String, String)> = vec![
            ("service.name".to_owned(), format!("yuppers-{process}")),
            ("service.namespace".to_owned(), "yuppers".to_owned()),
            ("service.version".to_owned(), build.version.to_owned()),
            ("service.instance.id".to_owned(), instance),
            ("deployment.environment".to_owned(), environment),
        ];
        if build.commit() != UNKNOWN {
            resource.push(("vcs.commit".to_owned(), build.commit().to_owned()));
        }
        for (key, value) in parse_pairs(
            optional(RESOURCE_ATTRIBUTES).as_deref(),
            RESOURCE_ATTRIBUTES,
        )? {
            set(&mut resource, key, value);
        }
        if let Some(name) = optional(SERVICE_NAME) {
            set(&mut resource, "service.name".to_owned(), name);
        }

        Ok(Some(Self {
            endpoint,
            headers,
            resource,
            timeout,
        }))
    }

    fn url(&self, signal: Signal) -> String {
        format!("{}{}", self.endpoint, signal.path())
    }

    /// `service.name`.
    pub fn service_name(&self) -> &str {
        self.resource
            .iter()
            .find(|(key, _)| key == "service.name")
            .map_or("", |(_, value)| value.as_str())
    }
}

fn set(resource: &mut Vec<(String, String)>, key: String, value: String) {
    match resource.iter_mut().find(|(found, _)| *found == key) {
        Some(entry) => entry.1 = value,
        None => resource.push((key, value)),
    }
}

/// The base address: `http://` or `https://`, a host, maybe a port and a
/// prefix, no trailing slash. A signal's own path, which the OpenTelemetry
/// convention of per-signal URLs makes somebody write, is taken off, so
/// that `.../v1/traces` does not become `.../v1/traces/v1/logs`.
fn parse_endpoint(endpoint: &str) -> anyhow::Result<String> {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        bail!("{endpoint:?} must start with http:// or https://");
    }
    let base = [Signal::Traces, Signal::Logs, Signal::Metrics]
        .iter()
        .find_map(|signal| trimmed.strip_suffix(signal.path()))
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    let uri: hyper::Uri = base
        .parse()
        .with_context(|| format!("{endpoint:?} is not a URL"))?;
    if uri.host().is_none_or(str::is_empty) {
        bail!("{endpoint:?} names no host");
    }
    if uri.query().is_some() {
        bail!("{endpoint:?} must not have a query string");
    }
    Ok(base.to_owned())
}

/// `name=value,name=value`, as the OpenTelemetry specification writes
/// headers and resource attributes (the W3C Baggage shape, values
/// percent-encoded where they need to be).
fn parse_pairs(list: Option<&str>, setting: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut pairs = Vec::new();
    for (index, entry) in list.unwrap_or("").split(',').enumerate() {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // The value is not quoted in the error: a header's is a secret.
        let (name, value) = entry
            .split_once('=')
            .with_context(|| format!("{setting}: entry {} is not name=value", index + 1))?;
        let name = percent_decode(name.trim());
        if name.is_empty() {
            bail!("{setting}: entry {} has no name", index + 1);
        }
        pairs.push((name, percent_decode(value.trim())));
    }
    Ok(pairs)
}

fn parse_headers(list: Option<&str>) -> anyhow::Result<Vec<(HeaderName, HeaderValue)>> {
    parse_pairs(list, HEADERS)?
        .iter()
        .map(|(name, value)| header(name, value))
        .collect()
}

/// One header, its value marked sensitive so that nothing that prints the
/// request shows it.
fn header(name: &str, value: &str) -> anyhow::Result<(HeaderName, HeaderValue)> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .with_context(|| format!("{HEADERS}: {name:?} is not a header name"))?;
    let mut value = HeaderValue::from_str(value)
        .with_context(|| format!("{HEADERS}: the value of {name} is not valid in a header"))?;
    value.set_sensitive(true);
    Ok((name, value))
}

/// `%41` to `A`. Anything that is not a well-formed escape stays as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (
                (bytes[index + 1] as char).to_digit(16),
                (bytes[index + 2] as char).to_digit(16),
            )
        {
            out.push((high * 16 + low) as u8);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_owned())
}

// ---- Identifiers ---------------------------------------------------------------

/// A trace's identifier and one span's within it, as random bytes; written
/// as lowercase hexadecimal, 32 and 16 characters, the way OTLP's JSON
/// encoding and a log line both carry them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ids {
    pub trace: [u8; 16],
    pub span: [u8; 8],
}

impl Ids {
    /// A new trace, and its first span.
    pub fn new() -> Self {
        Self {
            trace: uuid::Uuid::new_v4().into_bytes(),
            span: span_id(),
        }
    }

    /// Another span of the same trace.
    pub fn child(&self) -> Self {
        Self {
            trace: self.trace,
            span: span_id(),
        }
    }

    pub fn trace_hex(&self) -> String {
        hex(&self.trace)
    }

    pub fn span_hex(&self) -> String {
        hex(&self.span)
    }
}

impl Default for Ids {
    fn default() -> Self {
        Self::new()
    }
}

/// Eight random bytes, not all zero (which OTLP takes as no span).
fn span_id() -> [u8; 8] {
    loop {
        let bytes = uuid::Uuid::new_v4().into_bytes();
        let id: [u8; 8] = bytes[..8].try_into().expect("eight of sixteen bytes");
        if id != [0; 8] {
            return id;
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != N * 2 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; N];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    (out != [0u8; N]).then_some(out)
}

// ---- Spans and records -----------------------------------------------------------

/// An attribute's value.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl From<&str> for AttrValue {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

impl From<String> for AttrValue {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<i64> for AttrValue {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<u16> for AttrValue {
    fn from(value: u16) -> Self {
        Self::Int(i64::from(value))
    }
}

impl From<f64> for AttrValue {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<bool> for AttrValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl AttrValue {
    fn json(&self) -> Json {
        match self {
            Self::Str(value) => json!({ "stringValue": value }),
            Self::Int(value) => json!({ "intValue": value.to_string() }),
            Self::Float(value) => json!({ "doubleValue": value }),
            Self::Bool(value) => json!({ "boolValue": value }),
        }
    }
}

/// What kind of span: the OpenTelemetry numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind {
    Internal = 1,
    Server = 2,
    Client = 3,
}

/// A span while it is open, kept in the span's extensions.
#[derive(Debug)]
struct OpenSpan {
    ids: Ids,
    parent: Option<[u8; 8]>,
    name: String,
    kind: SpanKind,
    started: SystemTime,
    attributes: Vec<(String, AttrValue)>,
    error: bool,
}

/// A span that has closed, waiting to be sent.
#[derive(Debug)]
pub struct FinishedSpan {
    ids: Ids,
    parent: Option<[u8; 8]>,
    name: String,
    kind: SpanKind,
    started: SystemTime,
    ended: SystemTime,
    attributes: Vec<(String, AttrValue)>,
    error: bool,
}

/// A log record waiting to be sent.
#[derive(Debug)]
pub struct LogRecord {
    time: SystemTime,
    level: Level,
    body: String,
    attributes: Vec<(String, AttrValue)>,
    ids: Option<Ids>,
}

/// Fields a span has that are not exported as attributes: `trace_id` and
/// `span_id` are its identifiers, and a request's `path` can name an
/// exchange, where its `http.route` names only the template.
const NOT_ATTRIBUTES: [&str; 3] = ["trace_id", "span_id", "path"];

/// Collects a span's or an event's fields.
#[derive(Default)]
struct Fields {
    attributes: Vec<(String, AttrValue)>,
    message: Option<String>,
    trace_id: Option<[u8; 16]>,
    span_id: Option<[u8; 8]>,
}

impl Fields {
    fn put(&mut self, field: &Field, value: AttrValue) {
        let name = field.name();
        match name {
            "message" => {
                self.message = Some(match value {
                    AttrValue::Str(text) => text,
                    other => format!("{other:?}"),
                })
            }
            "trace_id" => {
                if let AttrValue::Str(text) = &value {
                    self.trace_id = from_hex(text);
                }
            }
            "span_id" => {
                if let AttrValue::Str(text) = &value {
                    self.span_id = from_hex(text);
                }
            }
            _ if NOT_ATTRIBUTES.contains(&name) => {}
            _ => match self.attributes.iter_mut().find(|(found, _)| found == name) {
                Some(entry) => entry.1 = value,
                None => self.attributes.push((name.to_owned(), value)),
            },
        }
    }
}

impl Visit for Fields {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, AttrValue::Float(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, AttrValue::Int(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        match i64::try_from(value) {
            Ok(value) => self.put(field, AttrValue::Int(value)),
            Err(_) => self.put(field, AttrValue::Str(value.to_string())),
        }
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        self.put(field, AttrValue::Str(value.to_string()));
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        self.put(field, AttrValue::Str(value.to_string()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, AttrValue::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, AttrValue::Str(value.to_owned()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, AttrValue::Str(format!("{value:?}")));
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        self.put(field, AttrValue::Str(value.to_string()));
    }
}

/// The buffers between the layer and the exporter.
#[derive(Default)]
struct Queues {
    spans: Mutex<Vec<FinishedSpan>>,
    logs: Mutex<Vec<LogRecord>>,
    dropped_spans: AtomicU64,
    dropped_logs: AtomicU64,
}

fn push_bounded<T>(queue: &Mutex<Vec<T>>, dropped: &AtomicU64, item: T) {
    let mut queue = queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if queue.len() >= QUEUE_MAX {
        dropped.fetch_add(1, Ordering::Relaxed);
    } else {
        queue.push(item);
    }
}

fn take<T>(queue: &Mutex<Vec<T>>) -> Vec<T> {
    std::mem::take(
        &mut *queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

// ---- The layer -------------------------------------------------------------------

/// The `tracing` layer that records spans and events for export.
#[derive(Clone)]
pub struct Layer {
    queues: Arc<Queues>,
}

impl<S> tracing_subscriber::Layer<S> for Layer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        attrs.record(&mut fields);

        let parent = if attrs.is_root() {
            None
        } else if attrs.is_contextual() {
            ctx.lookup_current()
        } else {
            attrs.parent().and_then(|id| ctx.span(id))
        };
        let parent =
            parent.and_then(|span| span.extensions().get::<OpenSpan>().map(|open| open.ids));

        let ids = Ids {
            trace: fields
                .trace_id
                .or(parent.map(|ids| ids.trace))
                .unwrap_or_else(|| Ids::new().trace),
            span: fields.span_id.unwrap_or_else(span_id),
        };
        let Some(span) = ctx.span(id) else {
            return;
        };
        span.extensions_mut().insert(OpenSpan {
            ids,
            parent: parent.map(|ids| ids.span),
            name: attrs.metadata().name().to_owned(),
            kind: SpanKind::Internal,
            started: SystemTime::now(),
            attributes: fields.attributes,
            error: false,
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        let Some(open) = extensions.get_mut::<OpenSpan>() else {
            return;
        };
        let mut fields = Fields::default();
        values.record(&mut fields);
        for (key, value) in fields.attributes {
            set_attribute(&mut open.attributes, key, value);
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let mut attributes = fields.attributes;
        set_attribute(
            &mut attributes,
            "target".to_owned(),
            AttrValue::Str(event.metadata().target().to_owned()),
        );

        // The span it happened in: its identifiers, and its fields, as the
        // line on standard output carries them.
        let mut ids = None;
        if let Some(span) = ctx.event_span(event) {
            let extensions = span.extensions();
            if let Some(open) = extensions.get::<OpenSpan>() {
                ids = Some(open.ids);
                for (key, value) in &open.attributes {
                    if !attributes.iter().any(|(found, _)| found == key) {
                        attributes.push((key.clone(), value.clone()));
                    }
                }
            }
        }
        if *event.metadata().level() == Level::ERROR
            && let Some(span) = ctx.event_span(event)
            && let Some(open) = span.extensions_mut().get_mut::<OpenSpan>()
        {
            open.error = true;
        }

        push_bounded(
            &self.queues.logs,
            &self.queues.dropped_logs,
            LogRecord {
                time: SystemTime::now(),
                level: *event.metadata().level(),
                body: fields.message.unwrap_or_default(),
                attributes,
                ids,
            },
        );
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else {
            return;
        };
        let Some(open) = span.extensions_mut().remove::<OpenSpan>() else {
            return;
        };
        push_bounded(
            &self.queues.spans,
            &self.queues.dropped_spans,
            FinishedSpan {
                ids: open.ids,
                parent: open.parent,
                name: open.name,
                kind: open.kind,
                started: open.started,
                ended: SystemTime::now(),
                attributes: open.attributes,
                error: open.error,
            },
        );
    }
}

fn set_attribute(attributes: &mut Vec<(String, AttrValue)>, key: String, value: AttrValue) {
    match attributes.iter_mut().find(|(found, _)| *found == key) {
        Some(entry) => entry.1 = value,
        None => attributes.push((key, value)),
    }
}

/// What a span may say about itself beyond its fields, without the words
/// reaching the log on standard output: its name as OpenTelemetry shows it
/// (`GET /v1/exchanges/{id}`), its kind, attributes, and that it failed.
/// Each is nothing when no [`Layer`] is installed.
pub trait SpanExt {
    fn otel_name(&self, name: impl Into<String>);
    fn otel_kind(&self, kind: SpanKind);
    fn otel_attr(&self, key: &'static str, value: impl Into<AttrValue>);
    fn otel_error(&self);
}

fn with_open_span(span: &tracing::Span, f: impl FnOnce(&mut OpenSpan)) {
    span.with_subscriber(|(id, dispatch)| {
        let Some(registry) = dispatch.downcast_ref::<Registry>() else {
            return;
        };
        let Some(data) = registry.span_data(id) else {
            return;
        };
        let mut extensions = data.extensions_mut();
        if let Some(open) = extensions.get_mut::<OpenSpan>() {
            f(open);
        }
    });
}

impl SpanExt for tracing::Span {
    fn otel_name(&self, name: impl Into<String>) {
        let name = name.into();
        with_open_span(self, |open| open.name = name);
    }

    fn otel_kind(&self, kind: SpanKind) {
        with_open_span(self, |open| open.kind = kind);
    }

    fn otel_attr(&self, key: &'static str, value: impl Into<AttrValue>) {
        let value = value.into();
        with_open_span(self, |open| {
            set_attribute(&mut open.attributes, key.to_owned(), value)
        });
    }

    fn otel_error(&self) {
        with_open_span(self, |open| open.error = true);
    }
}

// ---- Documents -----------------------------------------------------------------

fn nanos(time: SystemTime) -> String {
    time.duration_since(UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or(0)
        .to_string()
}

fn attributes_json(attributes: &[(String, AttrValue)]) -> Json {
    Json::Array(
        attributes
            .iter()
            .map(|(key, value)| json!({ "key": key, "value": value.json() }))
            .collect(),
    )
}

fn resource_json(resource: &[(String, String)]) -> Json {
    json!({
        "attributes": resource
            .iter()
            .map(|(key, value)| json!({ "key": key, "value": { "stringValue": value } }))
            .collect::<Vec<_>>()
    })
}

fn scope_json() -> Json {
    json!({ "name": "yuppers", "version": env!("CARGO_PKG_VERSION") })
}

/// An `ExportTraceServiceRequest`.
fn traces_document(resource: &[(String, String)], spans: &[FinishedSpan]) -> Json {
    let spans: Vec<Json> = spans
        .iter()
        .map(|span| {
            let mut out = json!({
                "traceId": span.ids.trace_hex(),
                "spanId": span.ids.span_hex(),
                "name": span.name,
                "kind": span.kind as u8,
                "startTimeUnixNano": nanos(span.started),
                "endTimeUnixNano": nanos(span.ended),
                "attributes": attributes_json(&span.attributes),
            });
            if let Some(parent) = span.parent {
                out["parentSpanId"] = json!(hex(&parent));
            }
            if span.error {
                out["status"] = json!({ "code": 2 });
            }
            out
        })
        .collect();
    json!({
        "resourceSpans": [{
            "resource": resource_json(resource),
            "scopeSpans": [{ "scope": scope_json(), "spans": spans }],
        }]
    })
}

/// OpenTelemetry's severity number and text for a level. The numbers are
/// the specification's: each name owns a range of four.
fn severity(level: Level) -> (u8, &'static str) {
    match level {
        Level::TRACE => (1, "TRACE"),
        Level::DEBUG => (5, "DEBUG"),
        Level::INFO => (9, "INFO"),
        Level::WARN => (13, "WARN"),
        Level::ERROR => (17, "ERROR"),
    }
}

/// An `ExportLogsServiceRequest`.
fn logs_document(resource: &[(String, String)], records: &[LogRecord]) -> Json {
    let records: Vec<Json> = records
        .iter()
        .map(|record| {
            let (number, text) = severity(record.level);
            let time = nanos(record.time);
            let mut out = json!({
                "timeUnixNano": time,
                "observedTimeUnixNano": time,
                "severityNumber": number,
                "severityText": text,
                "body": { "stringValue": record.body },
                "attributes": attributes_json(&record.attributes),
            });
            if let Some(ids) = record.ids {
                out["traceId"] = json!(ids.trace_hex());
                out["spanId"] = json!(ids.span_hex());
            }
            out
        })
        .collect();
    json!({
        "resourceLogs": [{
            "resource": resource_json(resource),
            "scopeLogs": [{ "scope": scope_json(), "logRecords": records }],
        }]
    })
}

/// An `ExportMetricsServiceRequest`.
fn metrics_document(resource: &[(String, String)], metrics: Vec<Json>) -> Json {
    json!({
        "resourceMetrics": [{
            "resource": resource_json(resource),
            "scopeMetrics": [{ "scope": scope_json(), "metrics": metrics }],
        }]
    })
}

// ---- The exporter ---------------------------------------------------------------

/// Renders the metrics page the process serves, for the exporter to send:
/// the same closure `crate::metrics::serve` is given.
pub type MetricsSource = Arc<
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Text> + Send>> + Send + Sync,
>;

/// When the process started, for the cumulative metrics.
fn started_at() -> SystemTime {
    static STARTED: OnceLock<SystemTime> = OnceLock::new();
    *STARTED.get_or_init(SystemTime::now)
}

struct Exporter {
    config: Config,
    queues: Arc<Queues>,
    client: outbound::Client,
    metrics: Arc<Mutex<Option<MetricsSource>>>,
    /// Whether the collector refused or could not be reached last time, and
    /// when that was last said.
    failing: bool,
    last_warning: Option<Instant>,
}

impl Exporter {
    async fn post(&mut self, signal: Signal, document: &Json) -> bool {
        let mut headers = self.config.headers.clone();
        headers.push((
            hyper::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        ));
        let body = serde_json::to_vec(document).unwrap_or_default();
        let outcome = match self
            .client
            .send(&self.config.url(signal), &headers, body)
            .await
        {
            Ok(answer) if answer.status.is_success() => Ok(()),
            Ok(answer) => Err(format!("the collector answered {}", answer.status.as_u16())),
            Err(error) => Err(error.to_string()),
        };
        match outcome {
            Ok(()) => {
                if self.failing {
                    tracing::info!(signal = signal.as_str(), "telemetry export recovered");
                }
                self.failing = false;
                true
            }
            Err(error) => {
                // Once at first, then at most every five minutes while it
                // lasts. The collector's answer is never quoted beyond its
                // status: an access gateway's refusal is a whole page.
                let due = self
                    .last_warning
                    .is_none_or(|last| last.elapsed() >= WARN_EVERY);
                if !self.failing || due {
                    tracing::warn!(
                        signal = signal.as_str(),
                        error,
                        "telemetry export failed; records are dropped until the collector answers"
                    );
                    self.last_warning = Some(Instant::now());
                }
                self.failing = true;
                false
            }
        }
    }

    /// Sends the buffered spans and log records, in batches. A batch the
    /// collector does not take is dropped with the rest of its signal,
    /// and counted.
    async fn flush_signals(&mut self) {
        let spans = take(&self.queues.spans);
        let mut sent = 0;
        for batch in spans.chunks(BATCH_MAX) {
            if !self
                .post(
                    Signal::Traces,
                    &traces_document(&self.config.resource, batch),
                )
                .await
            {
                self.queues
                    .dropped_spans
                    .fetch_add((spans.len() - sent) as u64, Ordering::Relaxed);
                break;
            }
            sent += batch.len();
        }

        let logs = take(&self.queues.logs);
        let mut sent = 0;
        for batch in logs.chunks(BATCH_MAX) {
            if !self
                .post(Signal::Logs, &logs_document(&self.config.resource, batch))
                .await
            {
                self.queues
                    .dropped_logs
                    .fetch_add((logs.len() - sent) as u64, Ordering::Relaxed);
                break;
            }
            sent += batch.len();
        }
    }

    /// Sends the metrics page, with this exporter's own counts.
    async fn export_metrics(&mut self) {
        let source = self
            .metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(source) = source else {
            return;
        };
        let mut text = source().await;
        text.family(
            TELEMETRY_DROPPED,
            Kind::Counter,
            "Spans and log records not exported: the buffer was full, or the collector refused them.",
        );
        for (signal, dropped) in [
            ("traces", &self.queues.dropped_spans),
            ("logs", &self.queues.dropped_logs),
        ] {
            text.sample(
                TELEMETRY_DROPPED,
                &[("signal", signal)],
                dropped.load(Ordering::Relaxed) as f64,
            );
        }
        let start = started_at();
        let now = SystemTime::now();
        let metrics = text.otlp(
            start.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()),
            now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()),
        );
        self.post(
            Signal::Metrics,
            &metrics_document(&self.config.resource, metrics),
        )
        .await;
    }

    async fn run(mut self, mut stop: oneshot::Receiver<()>) {
        let mut signals = tokio::time::interval(FLUSH_EVERY);
        signals.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut metrics =
            tokio::time::interval_at(tokio::time::Instant::now() + METRICS_EVERY, METRICS_EVERY);
        metrics.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = signals.tick() => self.flush_signals().await,
                _ = metrics.tick() => self.export_metrics().await,
                _ = &mut stop => {
                    self.flush_signals().await;
                    self.export_metrics().await;
                    return;
                }
            }
        }
    }
}

/// The running export, held by the process to stop it cleanly.
pub struct Telemetry {
    running: Option<Running>,
}

struct Running {
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
    metrics: Arc<Mutex<Option<MetricsSource>>>,
}

impl Telemetry {
    /// Nothing is exported.
    pub fn off() -> Self {
        Self { running: None }
    }

    pub fn is_on(&self) -> bool {
        self.running.is_some()
    }

    /// Has the metrics page `source` renders sent every fifteen seconds:
    /// the same page `METRICS_ADDR` serves.
    pub fn metrics_from(&self, source: MetricsSource) {
        if let Some(running) = &self.running {
            *running
                .metrics
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(source);
        }
    }

    /// Sends what is buffered and stops, within [`SHUTDOWN_TIMEOUT`].
    pub async fn shutdown(self) {
        let Some(running) = self.running else {
            return;
        };
        let _ = running.stop.send(());
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, running.task)
            .await
            .is_err()
        {
            tracing::warn!("telemetry did not finish exporting in time; the rest is dropped");
        }
    }
}

/// Starts exporting to `config`: the layer to put on the subscriber, and
/// the handle that stops it. Needs the tokio runtime.
pub fn start(config: Config) -> (Layer, Telemetry) {
    started_at();
    let queues = Arc::new(Queues::default());
    let metrics = Arc::new(Mutex::new(None));
    let exporter = Exporter {
        client: outbound::Client::new(config.timeout),
        config,
        queues: queues.clone(),
        metrics: metrics.clone(),
        failing: false,
        last_warning: None,
    };
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(exporter.run(stopped));
    (
        Layer { queues },
        Telemetry {
            running: Some(Running {
                stop,
                task,
                metrics,
            }),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tracing_subscriber::layer::SubscriberExt;

    fn lookup(table: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let table: HashMap<String, String> = table
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        move |name| table.get(name).cloned()
    }

    fn build() -> BuildInfo {
        BuildInfo::resolve(
            Some("0123456789abcdef0123456789abcdef01234567"),
            None,
            &|_| None,
        )
    }

    fn attribute<'a>(resource: &'a [(String, String)], key: &str) -> Option<&'a str> {
        resource
            .iter()
            .find(|(found, _)| found == key)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn nothing_is_configured_without_an_endpoint() {
        assert!(
            Config::from_lookup(&lookup(&[]), "api", &build())
                .unwrap()
                .is_none()
        );
        assert!(
            Config::from_lookup(&lookup(&[(ENDPOINT, "  ")]), "api", &build())
                .unwrap()
                .is_none()
        );
        assert!(
            Config::from_lookup(
                &lookup(&[
                    (ENDPOINT, "http://localhost:4318"),
                    ("OTEL_SDK_DISABLED", "true")
                ]),
                "api",
                &build()
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn the_resource_names_the_service_the_build_and_the_environment() {
        let config = Config::from_lookup(
            &lookup(&[(ENDPOINT, "http://localhost:4318/")]),
            "worker",
            &build(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(config.endpoint, "http://localhost:4318");
        assert_eq!(
            config.url(Signal::Traces),
            "http://localhost:4318/v1/traces"
        );
        assert_eq!(config.url(Signal::Logs), "http://localhost:4318/v1/logs");
        assert_eq!(
            config.url(Signal::Metrics),
            "http://localhost:4318/v1/metrics"
        );
        assert_eq!(config.service_name(), "yuppers-worker");
        assert_eq!(
            attribute(&config.resource, "service.namespace"),
            Some("yuppers")
        );
        assert_eq!(
            attribute(&config.resource, "service.version"),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            attribute(&config.resource, "deployment.environment"),
            Some("development")
        );
        assert_eq!(
            attribute(&config.resource, "vcs.commit"),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert!(attribute(&config.resource, "service.instance.id").is_some());
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
        assert!(config.headers.is_empty());

        // A build without a commit says nothing about one.
        let config = Config::from_lookup(
            &lookup(&[(ENDPOINT, "http://localhost:4318")]),
            "api",
            &BuildInfo::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(attribute(&config.resource, "vcs.commit"), None);
        assert_eq!(config.service_name(), "yuppers-api");
    }

    #[test]
    fn the_environment_s_own_attributes_are_merged_over_the_defaults() {
        let config = Config::from_lookup(
            &lookup(&[
                (ENDPOINT, "https://otel.example.com"),
                (DEPLOYMENT_ENVIRONMENT, "production"),
                (SERVICE_NAME, "yuppers-api-blue"),
                (
                    RESOURCE_ATTRIBUTES,
                    "service.name=ignored,cloud.region=us-east,team=yup%20pers",
                ),
                ("RENDER_INSTANCE_ID", "srv-abc-xyz"),
                (TIMEOUT, "2500"),
            ]),
            "api",
            &build(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            config.service_name(),
            "yuppers-api-blue",
            "OTEL_SERVICE_NAME wins"
        );
        assert_eq!(
            attribute(&config.resource, "deployment.environment"),
            Some("production")
        );
        assert_eq!(attribute(&config.resource, "cloud.region"), Some("us-east"));
        assert_eq!(attribute(&config.resource, "team"), Some("yup pers"));
        assert_eq!(
            attribute(&config.resource, "service.instance.id"),
            Some("srv-abc-xyz")
        );
        assert_eq!(config.timeout, Duration::from_millis(2500));
    }

    #[test]
    fn endpoints_parse_the_way_people_write_them() {
        assert_eq!(
            parse_endpoint("http://localhost:4318").unwrap(),
            "http://localhost:4318"
        );
        assert_eq!(
            parse_endpoint("https://otel.yuppers.app/").unwrap(),
            "https://otel.yuppers.app"
        );
        // A per-signal URL, as the OpenTelemetry convention makes somebody
        // write, is taken as the collector's address.
        assert_eq!(
            parse_endpoint("http://otel:4318/v1/metrics").unwrap(),
            "http://otel:4318"
        );
        assert_eq!(
            parse_endpoint("http://gw:4318/otlp/v1/traces").unwrap(),
            "http://gw:4318/otlp"
        );
        for wrong in [
            "localhost:4318",
            "grpc://localhost:4317",
            "http://",
            "http://host:4318?x=1",
        ] {
            assert!(parse_endpoint(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn only_json_over_http_is_spoken() {
        for protocol in ["grpc", "http/protobuf"] {
            let error = Config::from_lookup(
                &lookup(&[(ENDPOINT, "http://localhost:4318"), (PROTOCOL, protocol)]),
                "api",
                &build(),
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("http/json"), "{error}");
        }
        assert!(
            Config::from_lookup(
                &lookup(&[(ENDPOINT, "http://localhost:4318"), (PROTOCOL, "http/json")]),
                "api",
                &build(),
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn headers_are_parsed_and_kept_secret() {
        let config = Config::from_lookup(
            &lookup(&[
                (ENDPOINT, "http://localhost:4318"),
                (
                    HEADERS,
                    "CF-Access-Client-Id=abc.access, CF-Access-Client-Secret=s%3Dcret%2C1",
                ),
            ]),
            "api",
            &build(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(config.headers.len(), 2);
        assert_eq!(config.headers[0].0.as_str(), "cf-access-client-id");
        assert_eq!(config.headers[0].1.to_str().unwrap(), "abc.access");
        assert_eq!(config.headers[1].0.as_str(), "cf-access-client-secret");
        assert_eq!(config.headers[1].1.to_str().unwrap(), "s=cret,1");
        assert!(config.headers[1].1.is_sensitive());
        let shown = format!("{config:?}");
        assert!(shown.contains("cf-access-client-secret"), "{shown}");
        assert!(!shown.contains("abc.access"), "{shown}");
        assert!(!shown.contains("s=cret"), "{shown}");

        // A malformed entry is refused without being quoted.
        let error = Config::from_lookup(
            &lookup(&[
                (ENDPOINT, "http://localhost:4318"),
                (HEADERS, "topsecretvalue"),
            ]),
            "api",
            &build(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("entry 1"), "{error}");
        assert!(!error.contains("topsecretvalue"), "{error}");
        let error = Config::from_lookup(
            &lookup(&[
                (ENDPOINT, "http://localhost:4318"),
                (HEADERS, "X-Key=line\nbreak"),
            ]),
            "api",
            &build(),
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("break"), "{error}");
    }

    #[test]
    fn identifiers_are_random_hexadecimal_of_the_right_length() {
        let ids = Ids::new();
        assert_eq!(ids.trace_hex().len(), 32);
        assert_eq!(ids.span_hex().len(), 16);
        assert_ne!(Ids::new().trace, ids.trace);
        let child = ids.child();
        assert_eq!(child.trace, ids.trace);
        assert_ne!(child.span, ids.span);
        assert_eq!(from_hex::<16>(&ids.trace_hex()), Some(ids.trace));
        assert_eq!(from_hex::<8>(&ids.span_hex()), Some(ids.span));
        assert_eq!(from_hex::<8>("0000000000000000"), None);
        assert_eq!(from_hex::<8>("not hex"), None);
    }

    /// Spans and events recorded through a real subscriber, read back from
    /// the queues.
    #[test]
    fn spans_and_events_are_recorded_with_their_identifiers_and_fields() {
        let queues = Arc::new(Queues::default());
        let layer = Layer {
            queues: queues.clone(),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let ids = Ids::new();
        let request = tracing::info_span!(
            "request",
            method = "GET",
            path = "/v1/exchanges/0f8b2c9e",
            request_id = "req-1",
            trace_id = %ids.trace_hex(),
            span_id = %ids.span_hex(),
        );
        request.otel_name("GET /v1/exchanges/{id}");
        request.otel_kind(SpanKind::Server);
        request.otel_attr("http.route", "/v1/exchanges/{id}");
        {
            let _entered = request.enter();
            let db = tracing::info_span!("db");
            db.otel_kind(SpanKind::Client);
            db.otel_attr("db.operation.name", "exchange.load");
            let _in_db = db.enter();
            tracing::info!(rows = 3u64, "loaded");
        }
        request.otel_attr("http.response.status_code", 500u16);
        request.otel_error();
        drop(request);

        let spans = take(&queues.spans);
        assert_eq!(spans.len(), 2);
        let db = &spans[0];
        let request = &spans[1];
        assert_eq!(
            request.ids, ids,
            "the span's own fields are its identifiers"
        );
        assert_eq!(request.parent, None);
        assert_eq!(request.name, "GET /v1/exchanges/{id}");
        assert_eq!(request.kind, SpanKind::Server);
        assert!(request.error);
        assert!(request.ended >= request.started);
        let attributes: HashMap<&str, &AttrValue> = request
            .attributes
            .iter()
            .map(|(key, value)| (key.as_str(), value))
            .collect();
        assert_eq!(attributes["method"], &AttrValue::Str("GET".to_owned()));
        assert_eq!(
            attributes["request_id"],
            &AttrValue::Str("req-1".to_owned())
        );
        assert_eq!(
            attributes["http.route"],
            &AttrValue::Str("/v1/exchanges/{id}".to_owned())
        );
        assert_eq!(
            attributes["http.response.status_code"],
            &AttrValue::Int(500)
        );
        assert!(!attributes.contains_key("path"), "{attributes:?}");
        assert!(!attributes.contains_key("trace_id"), "{attributes:?}");

        assert_eq!(db.ids.trace, ids.trace, "a child is in its parent's trace");
        assert_ne!(db.ids.span, ids.span);
        assert_eq!(db.parent, Some(ids.span));
        assert_eq!(db.kind, SpanKind::Client);
        assert!(!db.error);

        let logs = take(&queues.logs);
        assert_eq!(logs.len(), 1);
        let record = &logs[0];
        assert_eq!(record.body, "loaded");
        assert_eq!(record.level, Level::INFO);
        assert_eq!(record.ids, Some(db.ids));
        let attributes: HashMap<&str, &AttrValue> = record
            .attributes
            .iter()
            .map(|(key, value)| (key.as_str(), value))
            .collect();
        assert_eq!(attributes["rows"], &AttrValue::Int(3));
        assert_eq!(
            attributes["db.operation.name"],
            &AttrValue::Str("exchange.load".to_owned())
        );
        assert!(attributes.contains_key("target"), "{attributes:?}");

        // The documents hold the same.
        let traces = traces_document(&[("service.name".to_owned(), "t".to_owned())], &spans);
        let exported = &traces["resourceSpans"][0]["scopeSpans"][0]["spans"];
        assert_eq!(exported[1]["traceId"], ids.trace_hex());
        assert_eq!(exported[1]["spanId"], ids.span_hex());
        assert_eq!(exported[1]["kind"], 2);
        assert_eq!(exported[1]["status"]["code"], 2);
        assert!(exported[1].get("parentSpanId").is_none());
        assert_eq!(exported[0]["parentSpanId"], ids.span_hex());
        assert_eq!(exported[0]["status"], Json::Null);
        assert_eq!(
            traces["resourceSpans"][0]["resource"]["attributes"][0]["value"]["stringValue"],
            "t"
        );
        let logs = logs_document(&[], &logs);
        let record = &logs["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0];
        assert_eq!(record["severityNumber"], 9);
        assert_eq!(record["severityText"], "INFO");
        assert_eq!(record["body"]["stringValue"], "loaded");
        assert_eq!(record["traceId"], ids.trace_hex());
        assert_eq!(
            record["attributes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|attribute| attribute["key"] == "rows")
                .unwrap()["value"]["intValue"],
            "3"
        );
    }

    #[test]
    fn an_error_logged_in_a_span_marks_it_failed() {
        let queues = Arc::new(Queues::default());
        let layer = Layer {
            queues: queues.clone(),
        };
        let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(layer));
        {
            let span = tracing::info_span!("job", job = "timers");
            let _entered = span.enter();
            tracing::error!("timers failed");
        }
        let spans = take(&queues.spans);
        assert!(spans[0].error);
        assert!(spans[0].parent.is_none());
        assert_eq!(spans[0].name, "job");
        // A root span without identifiers of its own is given some.
        assert_ne!(spans[0].ids.trace, [0; 16]);
        let (number, text) = severity(Level::ERROR);
        assert_eq!((number, text), (17, "ERROR"));
    }

    #[test]
    fn the_buffers_are_bounded_and_the_loss_is_counted() {
        let queues = Queues::default();
        for index in 0..(QUEUE_MAX + 10) {
            push_bounded(
                &queues.logs,
                &queues.dropped_logs,
                LogRecord {
                    time: SystemTime::now(),
                    level: Level::INFO,
                    body: format!("line {index}"),
                    attributes: Vec::new(),
                    ids: None,
                },
            );
        }
        assert_eq!(queues.logs.lock().unwrap().len(), QUEUE_MAX);
        assert_eq!(queues.dropped_logs.load(Ordering::Relaxed), 10);
    }

    #[test]
    fn percent_encoding_is_undone_where_it_is_well_formed() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("plain"), "plain");
    }
}
