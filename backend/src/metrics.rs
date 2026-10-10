//! Metrics, in the Prometheus text format and as OpenTelemetry metrics
//! (docs/operations.md, "Metrics" and "Telemetry").
//!
//! The text format is served on `METRICS_ADDR` when it is set, on a listener
//! of its own, never on the public port: what the service is doing is not
//! for everyone who can reach the API. The same page is pushed as OTLP
//! metrics when `OTEL_EXPORTER_OTLP_ENDPOINT` is set (`crate::otel`). One
//! page, written once by the api or the worker ([`Text`]), two encodings.
//!
//! Written by hand rather than through a metrics library. There are a few
//! dozen families, both formats are small and stable, and keeping the counts
//! in plain structs that the API state and the worker own, instead of a
//! process-wide recorder, lets every test read its own numbers. It also adds
//! nothing to the dependency tree that the weekly audit has to watch.
//!
//! Labels are kept to bounded sets: an HTTP request is counted under the
//! route's template (`/v1/exchanges/{id}`), never the path it was called
//! with, which would grow a series for every exchange.
//!
//! Each family has one name, written the OpenTelemetry way ([`Name`]:
//! `yuppers.http.requests`, with a unit); its Prometheus name is derived from
//! it the way the OpenTelemetry collector and Prometheus's OTLP receiver
//! derive it (`yuppers_http_requests_total`), so a dashboard sees one name
//! whichever way the metric arrived.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::Router;
use axum::http::header::CONTENT_TYPE;
use axum::http::{Method, StatusCode};
use axum::routing::get;
use serde_json::{Value as Json, json};
use sqlx::PgPool;

use crate::auth::{SMS_FAILED, SMS_REFUSED, SMS_REFUSED_COUNTRY, SMS_REFUSED_PREFIX, SMS_SENT};
use crate::notifications::outbox::Delivered;
use crate::notifications::push::{PushDelivered, ReceiptsError, ReceiptsRead};

/// The media type of the text format, version 0.0.4.
pub const TEXT_FORMAT: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Upper bounds, in seconds, of the request latency histogram's buckets.
/// From a millisecond, under the load check's fastest requests, to ten
/// seconds, past which a request has failed for any practical purpose.
pub const LATENCY_BUCKETS: [f64; 13] = [
    0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// The label for a request that matched no API route: the web app's pages
/// and assets, and anything unknown.
pub const UNMATCHED: &str = "unmatched";

// ---- Names -----------------------------------------------------------------

/// A metric family's name and unit, as OpenTelemetry writes them: dotted,
/// lowercase, `yuppers.<subsystem>.<thing>`, with a UCUM unit (`s` seconds,
/// `By` bytes, `{request}` a count of requests, which adds no suffix).
///
/// The Prometheus name is derived ([`Name::prometheus`]), never written by
/// hand, so the two can never drift apart. `tests::every_prometheus_name_is_the_documented_one`
/// holds the derived names to the ones docs/operations.md lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Name {
    pub otel: &'static str,
    pub unit: &'static str,
}

impl Name {
    pub const fn new(otel: &'static str, unit: &'static str) -> Self {
        Self { otel, unit }
    }

    /// The name in the Prometheus text format: dots become underscores, the
    /// unit adds its suffix (`s` → `_seconds`, `By` → `_bytes`; a `{count}`
    /// unit or none adds nothing) and a counter ends in `_total`. This is the
    /// translation the OpenTelemetry collector and Prometheus's OTLP
    /// receiver apply by default, so Grafana shows one name for a metric
    /// whether it was scraped from `/metrics` or pushed over OTLP.
    pub fn prometheus(&self, kind: Kind) -> String {
        let mut name = self.otel.replace('.', "_");
        let suffix = match self.unit {
            "s" => "_seconds",
            "ms" => "_milliseconds",
            "By" => "_bytes",
            unit if unit.is_empty() || unit.starts_with('{') => "",
            other => unreachable!("no Prometheus suffix is known for the unit {other}"),
        };
        if !name.ends_with(suffix) {
            name.push_str(suffix);
        }
        if matches!(kind, Kind::Counter) {
            name.push_str("_total");
        }
        name
    }
}

/// The build, `yuppers_build_info`.
pub const BUILD_INFO: Name = Name::new("yuppers.build.info", "");
pub const HTTP_REQUESTS: Name = Name::new("yuppers.http.requests", "{request}");
pub const HTTP_REQUEST_DURATION: Name = Name::new("yuppers.http.request.duration", "s");
pub const DB_POOL_MAX: Name = Name::new("yuppers.db.pool.max", "{connection}");
pub const DB_POOL_SIZE: Name = Name::new("yuppers.db.pool.size", "{connection}");
pub const DB_POOL_IN_USE: Name = Name::new("yuppers.db.pool.in_use", "{connection}");
pub const DATABASE_UP: Name = Name::new("yuppers.database.up", "");
pub const OUTBOX_MESSAGES: Name = Name::new("yuppers.outbox.messages", "{message}");
pub const OUTBOX_OLDEST_PENDING_AGE: Name = Name::new("yuppers.outbox.oldest_pending_age", "s");
pub const REPORTS_OPEN: Name = Name::new("yuppers.reports.open", "{report}");
pub const REPORTS_OLDEST_OPEN_AGE: Name = Name::new("yuppers.reports.oldest_open_age", "s");
pub const SMS_CODES_HOURLY_CAP: Name = Name::new("yuppers.sms.codes.hourly_cap", "{code}");
pub const SMS_CODES_THIS_HOUR: Name = Name::new("yuppers.sms.codes.this_hour", "{code}");
pub const SMS_CODES_REFUSED_THIS_HOUR: Name =
    Name::new("yuppers.sms.codes.refused_this_hour", "{code}");
pub const OUTBOX_DELIVERIES: Name = Name::new("yuppers.outbox.deliveries", "{message}");
pub const PUSH_DELIVERIES: Name = Name::new("yuppers.push.deliveries", "{message}");
pub const PUSH_DEVICES_REMOVED: Name = Name::new("yuppers.push.devices_removed", "{device}");
pub const PUSH_RECEIPT_CHECKS: Name = Name::new("yuppers.push.receipt_checks", "{request}");
pub const WORKER_RUNS: Name = Name::new("yuppers.worker.runs", "{run}");
pub const WORKER_TIMER_CHANGES: Name = Name::new("yuppers.worker.timer_changes", "{change}");
pub const WORKER_REMINDERS_QUEUED: Name =
    Name::new("yuppers.worker.reminders_queued", "{reminder}");
pub const WORKER_LAST_PASS_TIMESTAMP: Name = Name::new("yuppers.worker.last_pass_timestamp", "s");
pub const WALLET_CERT_EXPIRY: Name = Name::new("yuppers.wallet.cert_expiry", "s");

// ---- A page of metrics -------------------------------------------------------

/// What kind of metric a family is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Gauge,
    Histogram,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

/// One histogram's observations: how many fell in each bucket (one more
/// bucket than there are bounds, the last for everything above the top
/// bound), their total and their sum.
#[derive(Clone, Debug, PartialEq)]
pub struct HistogramSample {
    pub bounds: &'static [f64],
    pub counts: Vec<u64>,
    pub count: u64,
    pub sum: f64,
}

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Number(f64),
    Histogram(HistogramSample),
}

#[derive(Clone, Debug, PartialEq)]
struct Sample {
    labels: Vec<(String, String)>,
    value: Value,
}

#[derive(Clone, Debug, PartialEq)]
struct Family {
    name: Name,
    kind: Kind,
    help: String,
    samples: Vec<Sample>,
}

/// A page of metrics being written: every family with its samples, in the
/// order they were added. Written out as Prometheus text ([`Text::finish`])
/// or as OpenTelemetry metrics ([`Text::otlp`]).
#[derive(Default)]
pub struct Text {
    families: Vec<Family>,
}

impl Text {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a family: its help line and type. Every sample of the family
    /// follows it.
    pub fn family(&mut self, name: Name, kind: Kind, help: &str) {
        self.families.push(Family {
            name,
            kind,
            help: help.to_owned(),
            samples: Vec::new(),
        });
    }

    fn family_mut(&mut self, name: Name) -> &mut Family {
        let found = self.families.iter().rposition(|family| family.name == name);
        let index = match found {
            Some(index) => index,
            None => {
                // Every family is started before its samples. Should one not
                // be, it is written as a gauge without help rather than lost.
                self.family(name, Kind::Gauge, "");
                self.families.len() - 1
            }
        };
        &mut self.families[index]
    }

    fn push(&mut self, name: Name, labels: &[(&str, &str)], value: Value) {
        let labels = labels
            .iter()
            .map(|(label, value)| ((*label).to_owned(), (*value).to_owned()))
            .collect();
        self.family_mut(name).samples.push(Sample { labels, value });
    }

    /// One sample.
    pub fn sample(&mut self, name: Name, labels: &[(&str, &str)], value: f64) {
        self.push(name, labels, Value::Number(value));
    }

    /// One histogram's observations.
    pub fn histogram(&mut self, name: Name, labels: &[(&str, &str)], sample: HistogramSample) {
        debug_assert_eq!(sample.counts.len(), sample.bounds.len() + 1);
        self.push(name, labels, Value::Histogram(sample));
    }

    /// A family with a single unlabeled sample.
    pub fn single(&mut self, name: Name, kind: Kind, help: &str, value: f64) {
        self.family(name, kind, help);
        self.sample(name, &[], value);
    }

    /// Whether anything has been written.
    pub fn is_empty(&self) -> bool {
        self.families.is_empty()
    }

    /// The page in the Prometheus text format.
    pub fn finish(self) -> String {
        let mut out = String::new();
        for family in &self.families {
            let name = family.name.prometheus(family.kind);
            let help = family.help.replace('\\', "\\\\").replace('\n', "\\n");
            let _ = writeln!(out, "# HELP {name} {help}");
            let _ = writeln!(out, "# TYPE {name} {}", family.kind.as_str());
            for sample in &family.samples {
                let labels: Vec<(&str, &str)> = sample
                    .labels
                    .iter()
                    .map(|(label, value)| (label.as_str(), value.as_str()))
                    .collect();
                match &sample.value {
                    Value::Number(value) => write_sample(&mut out, &name, &labels, *value),
                    Value::Histogram(histogram) => {
                        let mut cumulative = 0;
                        for (bound, count) in histogram.bounds.iter().zip(&histogram.counts) {
                            cumulative += count;
                            let le = number(*bound);
                            let labels = [labels.as_slice(), &[("le", le.as_str())]].concat();
                            write_sample(
                                &mut out,
                                &format!("{name}_bucket"),
                                &labels,
                                cumulative as f64,
                            );
                        }
                        let labels_inf = [labels.as_slice(), &[("le", "+Inf")]].concat();
                        write_sample(
                            &mut out,
                            &format!("{name}_bucket"),
                            &labels_inf,
                            histogram.count as f64,
                        );
                        write_sample(&mut out, &format!("{name}_sum"), &labels, histogram.sum);
                        write_sample(
                            &mut out,
                            &format!("{name}_count"),
                            &labels,
                            histogram.count as f64,
                        );
                    }
                }
            }
        }
        out
    }

    /// The page as the `metrics` array of an OTLP `scopeMetrics` entry
    /// (`crate::otel`): each family once, with its OpenTelemetry name, unit
    /// and description, cumulative since `start_unix_nano`, the process's
    /// start, and read at `time_unix_nano`. Counters and histograms are
    /// cumulative, so a push that is lost costs resolution, never a count.
    pub fn otlp(&self, start_unix_nano: u128, time_unix_nano: u128) -> Vec<Json> {
        let start = start_unix_nano.to_string();
        let time = time_unix_nano.to_string();
        self.families
            .iter()
            .map(|family| {
                let points: Vec<Json> = family
                    .samples
                    .iter()
                    .map(|sample| {
                        let mut point = json!({
                            "attributes": attributes(&sample.labels),
                            "startTimeUnixNano": start,
                            "timeUnixNano": time,
                        });
                        match &sample.value {
                            Value::Number(value) => {
                                let (key, value) = otlp_number(*value);
                                point[key] = value;
                            }
                            Value::Histogram(histogram) => {
                                point["count"] = json!(histogram.count.to_string());
                                point["sum"] = json!(histogram.sum);
                                point["bucketCounts"] = Json::Array(
                                    histogram
                                        .counts
                                        .iter()
                                        .map(|count| json!(count.to_string()))
                                        .collect(),
                                );
                                point["explicitBounds"] = json!(histogram.bounds);
                            }
                        }
                        point
                    })
                    .collect();
                let data = match family.kind {
                    Kind::Counter => json!({
                        "sum": {
                            "aggregationTemporality": 2,
                            "isMonotonic": true,
                            "dataPoints": points,
                        }
                    }),
                    Kind::Gauge => json!({ "gauge": { "dataPoints": points } }),
                    Kind::Histogram => json!({
                        "histogram": {
                            "aggregationTemporality": 2,
                            "dataPoints": points,
                        }
                    }),
                };
                let mut metric = json!({
                    "name": family.name.otel,
                    "unit": family.name.unit,
                    "description": family.help,
                });
                for (key, value) in data.as_object().into_iter().flatten() {
                    metric[key] = value.clone();
                }
                metric
            })
            .collect()
    }
}

/// Labels as OTLP attributes.
fn attributes(labels: &[(String, String)]) -> Json {
    Json::Array(
        labels
            .iter()
            .map(|(key, value)| json!({ "key": key, "value": { "stringValue": value } }))
            .collect(),
    )
}

/// A number as an OTLP data point holds it: a whole number as `asInt` (a
/// 64-bit integer, which OTLP's JSON encoding writes as a string), anything
/// else as `asDouble`.
fn otlp_number(value: f64) -> (&'static str, Json) {
    if value.fract() == 0.0 && value.abs() < 9.0e15 {
        ("asInt", json!((value as i64).to_string()))
    } else {
        ("asDouble", json!(value))
    }
}

fn write_sample(out: &mut String, name: &str, labels: &[(&str, &str)], value: f64) {
    out.push_str(name);
    if !labels.is_empty() {
        out.push('{');
        for (index, (label, value)) in labels.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(out, "{label}=\"{}\"", escape_label(value));
        }
        out.push('}');
    }
    let _ = writeln!(out, " {}", number(value));
}

/// A label value, escaped as the format requires.
fn escape_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn number(value: f64) -> String {
    if value == f64::INFINITY {
        "+Inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-Inf".to_owned()
    } else if value.is_nan() {
        "NaN".to_owned()
    } else {
        format!("{value}")
    }
}

// ---- HTTP requests ---------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct HttpSeries {
    route: String,
    method: &'static str,
    status: &'static str,
}

#[derive(Clone, Debug, Default)]
struct Histogram {
    /// Observations at or below each bound in [`LATENCY_BUCKETS`], each
    /// counted once, in the first bucket it fits; summed when written.
    buckets: [u64; LATENCY_BUCKETS.len()],
    count: u64,
    sum: f64,
}

impl Histogram {
    fn observe(&mut self, seconds: f64) {
        if let Some(bucket) = LATENCY_BUCKETS.iter().position(|bound| seconds <= *bound) {
            self.buckets[bucket] += 1;
        }
        self.count += 1;
        self.sum += seconds;
    }
}

/// Counts and latencies of the API's requests, by route template, method
/// and status class.
#[derive(Default)]
pub struct HttpMetrics {
    series: Mutex<BTreeMap<HttpSeries, Histogram>>,
}

/// A method as a label: the ones the API uses by name, anything else as
/// `OTHER`, so a client cannot invent series.
fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::PATCH => "PATCH",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        _ => "OTHER",
    }
}

fn status_class(status: StatusCode) -> &'static str {
    match status.as_u16() / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        _ => "5xx",
    }
}

impl HttpMetrics {
    /// Counts one request. `route` is the template of the route it matched,
    /// or none.
    pub fn observe(
        &self,
        method: &Method,
        route: Option<&str>,
        status: StatusCode,
        elapsed: Duration,
    ) {
        let series = HttpSeries {
            route: route.unwrap_or(UNMATCHED).to_owned(),
            method: method_label(method),
            status: status_class(status),
        };
        let mut all = self
            .series
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        all.entry(series)
            .or_default()
            .observe(elapsed.as_secs_f64());
    }

    pub fn render(&self, text: &mut Text) {
        let all = self
            .series
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();

        text.family(
            HTTP_REQUESTS,
            Kind::Counter,
            "HTTP requests answered, by route template, method and status class.",
        );
        for (series, histogram) in &all {
            let labels = [
                ("route", series.route.as_str()),
                ("method", series.method),
                ("status", series.status),
            ];
            text.sample(HTTP_REQUESTS, &labels, histogram.count as f64);
        }

        text.family(
            HTTP_REQUEST_DURATION,
            Kind::Histogram,
            "Time from receiving a request to answering it, by route template, method and status class.",
        );
        for (series, histogram) in &all {
            let labels = [
                ("route", series.route.as_str()),
                ("method", series.method),
                ("status", series.status),
            ];
            // Slower than every bound: counted in the last bucket only.
            let in_buckets: u64 = histogram.buckets.iter().sum();
            let mut counts = histogram.buckets.to_vec();
            counts.push(histogram.count - in_buckets);
            text.histogram(
                HTTP_REQUEST_DURATION,
                &labels,
                HistogramSample {
                    bounds: &LATENCY_BUCKETS,
                    counts,
                    count: histogram.count,
                    sum: histogram.sum,
                },
            );
        }
    }
}

// ---- The database ----------------------------------------------------------

/// The connection pool: its limit, how many connections are open and how
/// many of those are in use.
pub fn render_pool(text: &mut Text, pool: &PgPool) {
    let size = pool.size();
    let idle = u32::try_from(pool.num_idle()).unwrap_or(u32::MAX);
    text.single(
        DB_POOL_MAX,
        Kind::Gauge,
        "Connections the pool may open at most.",
        f64::from(pool.options().get_max_connections()),
    );
    text.single(
        DB_POOL_SIZE,
        Kind::Gauge,
        "Connections the pool has open.",
        f64::from(size),
    );
    text.single(
        DB_POOL_IN_USE,
        Kind::Gauge,
        "Open connections in use by a request or a job.",
        f64::from(size.saturating_sub(idle)),
    );
}

/// The outbox as it stands in the database: messages waiting, messages
/// given up on, and how long the oldest waiting one has waited. Read at each
/// scrape, so it is right whichever process, or how many workers, did the
/// sending. Both the API and the worker report it, so it can still be seen
/// while the worker is down.
///
/// Also `yuppers_database_up`, 0 when the database could not be read.
pub async fn render_outbox(text: &mut Text, pool: &PgPool, max_attempts: i32) {
    // Only rows not yet completed, which the partial index
    // `outbox_pending_idx` holds; the sent ones are never read.
    let state: Result<(i64, i64, Option<f64>), sqlx::Error> = sqlx::query_as(
        "SELECT
             count(*) FILTER (WHERE attempts < $1),
             count(*) FILTER (WHERE attempts >= $1),
             EXTRACT(EPOCH FROM now() - min(created_at) FILTER (WHERE attempts < $1))::float8
         FROM outbox
         WHERE completed_at IS NULL",
    )
    .bind(max_attempts)
    .fetch_one(pool)
    .await;

    match state {
        Ok((pending, given_up, oldest)) => {
            text.single(
                DATABASE_UP,
                Kind::Gauge,
                "1 if the database answered the last scrape's query, 0 if not.",
                1.0,
            );
            text.family(
                OUTBOX_MESSAGES,
                Kind::Gauge,
                "Notifications not yet completed: pending (waiting, or between retries) and given_up (out of attempts, left for someone to look at).",
            );
            text.sample(OUTBOX_MESSAGES, &[("state", "pending")], pending as f64);
            text.sample(OUTBOX_MESSAGES, &[("state", "given_up")], given_up as f64);
            text.single(
                OUTBOX_OLDEST_PENDING_AGE,
                Kind::Gauge,
                "How long the oldest pending notification has waited since it was queued; 0 when none is pending.",
                oldest.unwrap_or(0.0).max(0.0),
            );
        }
        Err(error) => {
            tracing::warn!(%error, "metrics could not read the outbox");
            text.single(
                DATABASE_UP,
                Kind::Gauge,
                "1 if the database answered the last scrape's query, 0 if not.",
                0.0,
            );
        }
    }
}

// ---- Reports -----------------------------------------------------------------

/// The abuse reports waiting for review, and how long the oldest has waited
/// (DESIGN.md §9: every report is reviewed within 24 hours). Read at each
/// scrape, like the outbox, and reported by the API and the worker both.
/// Says nothing when the database cannot be read; `yuppers_database_up`
/// already does.
pub async fn render_reports(text: &mut Text, pool: &PgPool) {
    // Only open reports, which the partial index `report_open_idx` holds.
    let state: Result<(i64, Option<f64>), sqlx::Error> = sqlx::query_as(
        "SELECT count(*), EXTRACT(EPOCH FROM now() - min(created_at))::float8
         FROM report WHERE status = 'OPEN'",
    )
    .fetch_one(pool)
    .await;
    match state {
        Ok((open, oldest)) => {
            text.single(
                REPORTS_OPEN,
                Kind::Gauge,
                "Abuse reports waiting for review.",
                open as f64,
            );
            text.single(
                REPORTS_OLDEST_OPEN_AGE,
                Kind::Gauge,
                "How long the oldest open report has waited since it was made; 0 when none is open.",
                oldest.unwrap_or(0.0).max(0.0),
            );
        }
        Err(error) => tracing::warn!(%error, "metrics could not read the reports"),
    }
}

// ---- Text messages ---------------------------------------------------------

/// The one-time codes sent by text message in the current hour, by the whole
/// service, against the hourly cap (`SMS_MAX_PER_HOUR`): read from the counts
/// the API keeps in the database, so they are right however many copies of
/// the API there are. Served by the API when SMS delivery is on.
pub async fn render_sms(text: &mut Text, pool: &PgPool, cap: i64) {
    let counts: Result<Vec<(String, i64)>, sqlx::Error> = sqlx::query_as(
        "SELECT scope, sum(count)::bigint FROM sign_in_limit
         WHERE scope IN ($1, $2, $3, $4, $5)
           AND window_start = date_trunc('hour', now(), 'UTC')
         GROUP BY scope",
    )
    .bind(SMS_SENT)
    .bind(SMS_REFUSED)
    .bind(SMS_FAILED)
    .bind(SMS_REFUSED_PREFIX)
    .bind(SMS_REFUSED_COUNTRY)
    .fetch_all(pool)
    .await;
    text.single(
        SMS_CODES_HOURLY_CAP,
        Kind::Gauge,
        "Codes the service may send by text message per hour (SMS_MAX_PER_HOUR).",
        cap as f64,
    );
    let Ok(counts) = counts else {
        tracing::warn!("metrics could not read the text message counts");
        return;
    };
    let count = |scope: &str| {
        counts
            .iter()
            .find(|(found, _)| found == scope)
            .map_or(0, |(_, count)| *count)
    };
    let refusals = [
        ("hourly_cap", count(SMS_REFUSED)),
        ("prefix_cap", count(SMS_REFUSED_PREFIX)),
        ("country", count(SMS_REFUSED_COUNTRY)),
    ];
    let name = SMS_CODES_THIS_HOUR;
    text.family(
        name,
        Kind::Gauge,
        "One-time codes for phone numbers this hour (UTC), by the whole service: sent (taken by the SMS provider, each a message paid for), refused (not sent, for any reason in yuppers_sms_codes_refused_this_hour; the person was told), failed (the provider did not take it; not counted in sent).",
    );
    for (result, value) in [
        ("sent", count(SMS_SENT)),
        ("refused", refusals.iter().map(|(_, n)| n).sum()),
        ("failed", count(SMS_FAILED)),
    ] {
        text.sample(name, &[("result", result)], value as f64);
    }
    let name = SMS_CODES_REFUSED_THIS_HOUR;
    text.family(
        name,
        Kind::Gauge,
        "One-time codes for phone numbers refused this hour (UTC), by the whole service, by reason: hourly_cap (SMS_MAX_PER_HOUR), prefix_cap (SMS_MAX_PER_PREFIX_PER_HOUR, numbers beginning alike), country (not in SMS_ALLOWED_COUNTRY_CODES).",
    );
    for (reason, value) in refusals {
        text.sample(name, &[("reason", reason)], value as f64);
    }
}

// ---- The worker ------------------------------------------------------------

/// What this worker process has done since it started.
#[derive(Default)]
pub struct WorkerMetrics {
    sent: AtomicU64,
    failed: AtomicU64,
    given_up: AtomicU64,
    dropped: AtomicU64,
    push_sent: AtomicU64,
    push_failed: AtomicU64,
    push_given_up: AtomicU64,
    push_dropped: AtomicU64,
    push_devices_removed: AtomicU64,
    receipt_checks_ok: AtomicU64,
    receipt_checks_failed: AtomicU64,
    timer_runs_ok: AtomicU64,
    timer_runs_failed: AtomicU64,
    timer_changes: AtomicU64,
    reminder_runs_ok: AtomicU64,
    reminder_runs_failed: AtomicU64,
    reminders_queued: AtomicU64,
    last_pass: AtomicU64,
}

fn add(counter: &AtomicU64, amount: usize) {
    counter.fetch_add(amount as u64, Ordering::Relaxed);
}

fn read(counter: &AtomicU64) -> f64 {
    counter.load(Ordering::Relaxed) as f64
}

impl WorkerMetrics {
    pub fn timers(&self, result: &Result<usize, sqlx::Error>) {
        match result {
            Ok(changed) => {
                add(&self.timer_runs_ok, 1);
                add(&self.timer_changes, *changed);
            }
            Err(_) => add(&self.timer_runs_failed, 1),
        }
    }

    pub fn reminders(&self, result: &Result<usize, sqlx::Error>) {
        match result {
            Ok(queued) => {
                add(&self.reminder_runs_ok, 1);
                add(&self.reminders_queued, *queued);
            }
            Err(_) => add(&self.reminder_runs_failed, 1),
        }
    }

    pub fn delivered(&self, delivered: &Delivered) {
        add(&self.sent, delivered.sent);
        add(&self.failed, delivered.failed);
        add(&self.given_up, delivered.given_up);
        add(&self.dropped, delivered.dropped);
    }

    pub fn pushed(&self, pushed: &PushDelivered) {
        add(&self.push_sent, pushed.rows.sent);
        add(&self.push_failed, pushed.rows.failed);
        add(&self.push_given_up, pushed.rows.given_up);
        add(&self.push_dropped, pushed.rows.dropped);
        add(&self.push_devices_removed, pushed.devices_removed);
    }

    pub fn receipts(&self, result: &Result<ReceiptsRead, ReceiptsError>) {
        match result {
            Ok(read) => {
                // A look with nothing old enough to ask about asks nothing.
                if read.asked {
                    add(&self.receipt_checks_ok, 1);
                }
                add(&self.push_devices_removed, read.devices_removed);
            }
            Err(_) => add(&self.receipt_checks_failed, 1),
        }
    }

    /// A pass over every job has finished, at `unix_seconds`.
    pub fn pass_finished(&self, unix_seconds: u64) {
        self.last_pass.store(unix_seconds, Ordering::Relaxed);
    }

    pub fn render(&self, text: &mut Text) {
        let name = OUTBOX_DELIVERIES;
        text.family(
            name,
            Kind::Counter,
            "Notification sends by this worker since it started: sent, failed (every failed try, given up or not), given_up (the last try failed), dropped (closed unsent).",
        );
        for (result, counter) in [
            ("sent", &self.sent),
            ("failed", &self.failed),
            ("given_up", &self.given_up),
            ("dropped", &self.dropped),
        ] {
            text.sample(name, &[("result", result)], read(counter));
        }

        let name = PUSH_DELIVERIES;
        text.family(
            name,
            Kind::Counter,
            "Push notifications handled by this worker since it started, one per person and notice: sent (taken by the push service for at least one device), failed (every failed try), given_up (the last try failed), dropped (closed unsent: no device left, account closed, reminder no longer true, or push off).",
        );
        for (result, counter) in [
            ("sent", &self.push_sent),
            ("failed", &self.push_failed),
            ("given_up", &self.push_given_up),
            ("dropped", &self.push_dropped),
        ] {
            text.sample(name, &[("result", result)], read(counter));
        }
        text.single(
            PUSH_DEVICES_REMOVED,
            Kind::Counter,
            "Devices removed because the push service said their token is no longer registered, when sending or in a receipt.",
            read(&self.push_devices_removed),
        );
        let name = PUSH_RECEIPT_CHECKS;
        text.family(
            name,
            Kind::Counter,
            "Requests for push receipts, by whether the push service answered.",
        );
        for (result, counter) in [
            ("ok", &self.receipt_checks_ok),
            ("error", &self.receipt_checks_failed),
        ] {
            text.sample(name, &[("result", result)], read(counter));
        }

        let name = WORKER_RUNS;
        text.family(
            name,
            Kind::Counter,
            "Passes of the worker's timers (expiries and closures) and reminders, by whether they finished or failed.",
        );
        for (job, result, counter) in [
            ("timers", "ok", &self.timer_runs_ok),
            ("timers", "error", &self.timer_runs_failed),
            ("reminders", "ok", &self.reminder_runs_ok),
            ("reminders", "error", &self.reminder_runs_failed),
        ] {
            text.sample(name, &[("job", job), ("result", result)], read(counter));
        }

        text.single(
            WORKER_TIMER_CHANGES,
            Kind::Counter,
            "Exchanges the timers changed: revisions expired, close requests lapsed, inactivity prompts and closures.",
            read(&self.timer_changes),
        );
        text.single(
            WORKER_REMINDERS_QUEUED,
            Kind::Counter,
            "Reminders queued in the outbox.",
            read(&self.reminders_queued),
        );
        text.single(
            WORKER_LAST_PASS_TIMESTAMP,
            Kind::Gauge,
            "When the worker last finished a pass over its jobs, in seconds since the Unix epoch; 0 before the first.",
            read(&self.last_pass),
        );
    }
}

// ---- Serving ---------------------------------------------------------------

/// Serves `GET /metrics` on `addr`, each page written by `render`, until the
/// process ends. Binds before returning, so an address that cannot be used
/// stops the process at start rather than failing quietly later.
pub async fn serve<F, Fut>(addr: SocketAddr, render: F) -> anyhow::Result<()>
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = String> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| anyhow::anyhow!("METRICS_ADDR={addr} cannot be listened on: {error}"))?;
    let app = Router::new().route(
        "/metrics",
        get(move || {
            let render = render.clone();
            async move { ([(CONTENT_TYPE, TEXT_FORMAT)], render().await) }
        }),
    );
    tracing::info!(%addr, "metrics listening");
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!(%error, "metrics listener stopped");
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_written_in_the_text_format_with_labels_escaped() {
        const DEMO: Name = Name::new("demo", "{thing}");
        let mut text = Text::new();
        text.family(DEMO, Kind::Counter, "A demo.\nSecond line.");
        text.sample(DEMO, &[], 3.0);
        text.sample(
            DEMO,
            &[("a", "plain"), ("b", "quote \" back\\slash\nnewline")],
            0.25,
        );
        assert_eq!(
            text.finish(),
            "# HELP demo_total A demo.\\nSecond line.\n\
             # TYPE demo_total counter\n\
             demo_total 3\n\
             demo_total{a=\"plain\",b=\"quote \\\" back\\\\slash\\nnewline\"} 0.25\n"
        );
        assert_eq!(number(f64::INFINITY), "+Inf");
        assert_eq!(number(1e-3), "0.001");
    }

    /// The names docs/operations.md lists, derived from the OpenTelemetry
    /// names the way the collector derives them. A new family goes in both.
    #[test]
    fn every_prometheus_name_is_the_documented_one() {
        for (name, kind, expected) in [
            (BUILD_INFO, Kind::Gauge, "yuppers_build_info"),
            (HTTP_REQUESTS, Kind::Counter, "yuppers_http_requests_total"),
            (
                HTTP_REQUEST_DURATION,
                Kind::Histogram,
                "yuppers_http_request_duration_seconds",
            ),
            (DB_POOL_MAX, Kind::Gauge, "yuppers_db_pool_max"),
            (DB_POOL_SIZE, Kind::Gauge, "yuppers_db_pool_size"),
            (DB_POOL_IN_USE, Kind::Gauge, "yuppers_db_pool_in_use"),
            (DATABASE_UP, Kind::Gauge, "yuppers_database_up"),
            (OUTBOX_MESSAGES, Kind::Gauge, "yuppers_outbox_messages"),
            (
                OUTBOX_OLDEST_PENDING_AGE,
                Kind::Gauge,
                "yuppers_outbox_oldest_pending_age_seconds",
            ),
            (REPORTS_OPEN, Kind::Gauge, "yuppers_reports_open"),
            (
                REPORTS_OLDEST_OPEN_AGE,
                Kind::Gauge,
                "yuppers_reports_oldest_open_age_seconds",
            ),
            (
                SMS_CODES_HOURLY_CAP,
                Kind::Gauge,
                "yuppers_sms_codes_hourly_cap",
            ),
            (
                SMS_CODES_THIS_HOUR,
                Kind::Gauge,
                "yuppers_sms_codes_this_hour",
            ),
            (
                SMS_CODES_REFUSED_THIS_HOUR,
                Kind::Gauge,
                "yuppers_sms_codes_refused_this_hour",
            ),
            (
                OUTBOX_DELIVERIES,
                Kind::Counter,
                "yuppers_outbox_deliveries_total",
            ),
            (
                PUSH_DELIVERIES,
                Kind::Counter,
                "yuppers_push_deliveries_total",
            ),
            (
                PUSH_DEVICES_REMOVED,
                Kind::Counter,
                "yuppers_push_devices_removed_total",
            ),
            (
                PUSH_RECEIPT_CHECKS,
                Kind::Counter,
                "yuppers_push_receipt_checks_total",
            ),
            (WORKER_RUNS, Kind::Counter, "yuppers_worker_runs_total"),
            (
                WORKER_TIMER_CHANGES,
                Kind::Counter,
                "yuppers_worker_timer_changes_total",
            ),
            (
                WORKER_REMINDERS_QUEUED,
                Kind::Counter,
                "yuppers_worker_reminders_queued_total",
            ),
            (
                WORKER_LAST_PASS_TIMESTAMP,
                Kind::Gauge,
                "yuppers_worker_last_pass_timestamp_seconds",
            ),
            (
                WALLET_CERT_EXPIRY,
                Kind::Gauge,
                "yuppers_wallet_cert_expiry_seconds",
            ),
        ] {
            assert_eq!(name.prometheus(kind), expected, "{name:?}");
            assert!(
                name.otel.starts_with("yuppers."),
                "{name:?} is not namespaced"
            );
        }
    }

    #[test]
    fn the_page_is_also_written_as_otlp_metrics() {
        let mut text = Text::new();
        text.single(DB_POOL_MAX, Kind::Gauge, "Pool.", 10.0);
        text.family(HTTP_REQUESTS, Kind::Counter, "Requests.");
        text.sample(HTTP_REQUESTS, &[("route", "/v1/meta")], 3.0);
        text.family(HTTP_REQUEST_DURATION, Kind::Histogram, "Latency.");
        text.histogram(
            HTTP_REQUEST_DURATION,
            &[("route", "/v1/meta")],
            HistogramSample {
                bounds: &[0.1, 1.0],
                counts: vec![2, 0, 1],
                count: 3,
                sum: 12.5,
            },
        );
        let metrics = text.otlp(1_000, 2_000);
        assert_eq!(metrics.len(), 3);

        assert_eq!(metrics[0]["name"], "yuppers.db.pool.max");
        assert_eq!(metrics[0]["unit"], "{connection}");
        assert_eq!(metrics[0]["description"], "Pool.");
        assert_eq!(metrics[0]["gauge"]["dataPoints"][0]["asInt"], "10");
        assert_eq!(metrics[0]["gauge"]["dataPoints"][0]["timeUnixNano"], "2000");

        let sum = &metrics[1]["sum"];
        assert_eq!(sum["aggregationTemporality"], 2, "cumulative");
        assert_eq!(sum["isMonotonic"], true);
        let point = &sum["dataPoints"][0];
        assert_eq!(point["asInt"], "3");
        assert_eq!(point["startTimeUnixNano"], "1000");
        assert_eq!(point["attributes"][0]["key"], "route");
        assert_eq!(point["attributes"][0]["value"]["stringValue"], "/v1/meta");

        let point = &metrics[2]["histogram"]["dataPoints"][0];
        assert_eq!(metrics[2]["unit"], "s");
        assert_eq!(point["count"], "3");
        assert_eq!(point["sum"], 12.5);
        assert_eq!(point["bucketCounts"], json!(["2", "0", "1"]));
        assert_eq!(point["explicitBounds"], json!([0.1, 1.0]));

        // Fractions stay doubles.
        assert_eq!(otlp_number(0.25), ("asDouble", json!(0.25)));
        assert_eq!(otlp_number(7.0), ("asInt", json!("7")));
    }

    #[test]
    fn a_request_is_counted_under_its_route_method_and_status_class() {
        let metrics = HttpMetrics::default();
        let route = Some("/v1/exchanges/{id}");
        metrics.observe(
            &Method::GET,
            route,
            StatusCode::OK,
            Duration::from_millis(3),
        );
        metrics.observe(
            &Method::GET,
            route,
            StatusCode::NO_CONTENT,
            Duration::from_millis(30),
        );
        metrics.observe(
            &Method::GET,
            route,
            StatusCode::NOT_FOUND,
            Duration::from_millis(1),
        );
        metrics.observe(
            &Method::from_bytes(b"BREW").unwrap(),
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
            Duration::from_secs(20),
        );
        let mut text = Text::new();
        metrics.render(&mut text);
        let page = text.finish();

        let ok = r#"route="/v1/exchanges/{id}",method="GET",status="2xx""#;
        assert!(
            page.contains(&format!("yuppers_http_requests_total{{{ok}}} 2\n")),
            "{page}"
        );
        assert!(page.contains(
            r#"yuppers_http_requests_total{route="/v1/exchanges/{id}",method="GET",status="4xx"} 1"#
        ));
        assert!(page.contains(
            r#"yuppers_http_requests_total{route="unmatched",method="OTHER",status="5xx"} 1"#
        ));
        // Cumulative buckets: 3 ms is under 5 ms, 30 ms only from 50 ms.
        for (le, count) in [
            ("0.0025", 0),
            ("0.005", 1),
            ("0.025", 1),
            ("0.05", 2),
            ("+Inf", 2),
        ] {
            let line = format!(
                "yuppers_http_request_duration_seconds_bucket{{{ok},le=\"{le}\"}} {count}\n"
            );
            assert!(page.contains(&line), "{line} in {page}");
        }
        assert!(page.contains(&format!(
            "yuppers_http_request_duration_seconds_count{{{ok}}} 2\n"
        )));
        let sum_prefix = format!("yuppers_http_request_duration_seconds_sum{{{ok}}} ");
        let sum: f64 = page
            .lines()
            .find_map(|line| line.strip_prefix(&sum_prefix))
            .expect("a sum")
            .parse()
            .unwrap();
        assert!((sum - 0.033).abs() < 1e-9, "{sum}");
        // Slower than every bound: only in +Inf.
        assert!(page.contains(r#"yuppers_http_request_duration_seconds_bucket{route="unmatched",method="OTHER",status="5xx",le="10"} 0"#));
        assert!(page.contains(r#"yuppers_http_request_duration_seconds_bucket{route="unmatched",method="OTHER",status="5xx",le="+Inf"} 1"#));
        assert_eq!(page.matches("# TYPE").count(), 2);
    }

    #[test]
    fn the_worker_counts_its_push_notifications_apart_from_its_emails() {
        let metrics = WorkerMetrics::default();
        metrics.pushed(&PushDelivered {
            rows: Delivered {
                sent: 3,
                failed: 1,
                dropped: 2,
                ..Delivered::default()
            },
            devices_removed: 1,
        });
        metrics.receipts(&Ok(ReceiptsRead {
            asked: true,
            settled: 4,
            devices_removed: 2,
        }));
        metrics.receipts(&Ok(ReceiptsRead::default()));
        metrics.receipts(&Err(ReceiptsError::Service(anyhow::anyhow!("down"))));
        let mut text = Text::new();
        metrics.render(&mut text);
        let page = text.finish();
        for line in [
            r#"yuppers_push_deliveries_total{result="sent"} 3"#,
            r#"yuppers_push_deliveries_total{result="failed"} 1"#,
            r#"yuppers_push_deliveries_total{result="given_up"} 0"#,
            r#"yuppers_push_deliveries_total{result="dropped"} 2"#,
            "yuppers_push_devices_removed_total 3",
            r#"yuppers_push_receipt_checks_total{result="ok"} 1"#,
            r#"yuppers_push_receipt_checks_total{result="error"} 1"#,
            r#"yuppers_outbox_deliveries_total{result="sent"} 0"#,
        ] {
            assert!(page.contains(line), "{line} in\n{page}");
        }
    }

    #[test]
    fn the_worker_counts_what_it_did() {
        let metrics = WorkerMetrics::default();
        metrics.timers(&Ok(2));
        metrics.timers(&Err(sqlx::Error::PoolTimedOut));
        metrics.reminders(&Ok(5));
        metrics.delivered(&Delivered {
            sent: 4,
            failed: 2,
            given_up: 1,
            dropped: 1,
            cut_short: false,
        });
        metrics.pass_finished(1_790_000_000);
        let mut text = Text::new();
        metrics.render(&mut text);
        let page = text.finish();
        for line in [
            r#"yuppers_outbox_deliveries_total{result="sent"} 4"#,
            r#"yuppers_outbox_deliveries_total{result="failed"} 2"#,
            r#"yuppers_outbox_deliveries_total{result="given_up"} 1"#,
            r#"yuppers_outbox_deliveries_total{result="dropped"} 1"#,
            r#"yuppers_worker_runs_total{job="timers",result="ok"} 1"#,
            r#"yuppers_worker_runs_total{job="timers",result="error"} 1"#,
            r#"yuppers_worker_runs_total{job="reminders",result="ok"} 1"#,
            r#"yuppers_worker_runs_total{job="reminders",result="error"} 0"#,
            "yuppers_worker_timer_changes_total 2",
            "yuppers_worker_reminders_queued_total 5",
            "yuppers_worker_last_pass_timestamp_seconds 1790000000",
        ] {
            assert!(page.contains(&format!("{line}\n")), "{line} in {page}");
        }
    }
}
