//! Logging for every binary (docs/operations.md, "Logs"), and the
//! OpenTelemetry export beside it (`crate::otel`; "Telemetry").
//!
//! The level and filter come from `RUST_LOG` (default `info`), and the shape
//! from `LOG_FORMAT`: `text` (the default), one human-readable line per
//! event, or `json`, one JSON object per line for a log collector. With
//! `OTEL_EXPORTER_OTLP_ENDPOINT` set, the same events, and the spans they
//! happen in, also go to that collector; the output here is the same with
//! or without it.
//!
//! What may be logged is decided where each event is written, not here:
//! never a request or response body, a query string, a token, a cookie, a
//! one-time code, an email address or a phone number. The one exception is
//! the development deliveries (`CODE_DELIVERY=log`, `NOTIFICATION_DELIVERY=log`),
//! whose whole purpose is to write the message to the log, and which no
//! deployment configures. `backend/tests/telemetry.rs` checks a sign-in,
//! in the log and in what is exported.

use std::io::IsTerminal;

use anyhow::bail;
use tracing::Subscriber;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::otel;
pub use crate::otel::Telemetry;

/// How each log line is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// One readable line per event, colored on a terminal.
    #[default]
    Text,
    /// One JSON object per line. The event's fields are at the top level,
    /// beside `timestamp`, `level`, `target` and `message`; the fields of the
    /// span it happened in (for a request: `method`, `path`, `request_id`)
    /// are under `span`.
    Json,
}

impl LogFormat {
    /// Reads a `LOG_FORMAT` value. Unset or empty is the default.
    pub fn parse(value: Option<&str>) -> anyhow::Result<Self> {
        match value.map(str::trim) {
            None | Some("") | Some("text") => Ok(Self::Text),
            Some("json") => Ok(Self::Json),
            Some(other) => bail!("LOG_FORMAT={other} is not one of `text` or `json`"),
        }
    }
}

/// The subscriber the binaries install, writing to `writer`, and exporting
/// through `otel` when there is one. Tests build one over a buffer to read
/// back what was logged. `color` applies to text only. The filter applies
/// to both outputs: what `RUST_LOG` keeps out of the log is not exported
/// either.
pub fn subscriber<W>(
    format: LogFormat,
    filter: EnvFilter,
    color: bool,
    writer: W,
    otel: Option<otel::Layer>,
) -> Box<dyn Subscriber + Send + Sync>
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer);
    match format {
        LogFormat::Text => Box::new(builder.with_ansi(color).finish().with(otel)),
        LogFormat::Json => Box::new(
            builder
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(false)
                .finish()
                .with(otel),
        ),
    }
}

/// Installs the log subscriber for a binary (`api`, `worker`, ...), writing
/// to standard output, and starts the OpenTelemetry export if
/// `OTEL_EXPORTER_OTLP_ENDPOINT` is set. The handle returned stops the
/// export cleanly; call [`Telemetry::shutdown`] last.
///
/// Reads `.env` first, as the configuration does, so `RUST_LOG` and
/// `LOG_FORMAT` work from there in development too.
pub fn init(process: &str) -> anyhow::Result<Telemetry> {
    dotenvy::dotenv().ok();
    let format = LogFormat::parse(std::env::var("LOG_FORMAT").ok().as_deref())?;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // Color only for a person at a terminal, never into a file or a log
    // collector, and never when NO_COLOR asks for none.
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let config = otel::Config::from_env(process)?;
    let (layer, telemetry) = match config.clone() {
        Some(config) => {
            let (layer, telemetry) = otel::start(config);
            (Some(layer), telemetry)
        }
        None => (None, Telemetry::off()),
    };
    // Also routes the `log` crate's records here, as `fmt().init()` did.
    subscriber(format, filter, color, std::io::stdout, layer).try_init()?;
    if let Some(config) = config {
        tracing::info!(
            endpoint = config.endpoint,
            service = config.service_name(),
            "exporting traces, logs and metrics (OTEL_EXPORTER_OTLP_ENDPOINT)"
        );
    }
    Ok(telemetry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_format_is_text_unless_json_is_asked_for() {
        assert_eq!(LogFormat::parse(None).unwrap(), LogFormat::Text);
        assert_eq!(LogFormat::parse(Some(" ")).unwrap(), LogFormat::Text);
        assert_eq!(LogFormat::parse(Some("text")).unwrap(), LogFormat::Text);
        assert_eq!(LogFormat::parse(Some("json")).unwrap(), LogFormat::Json);
        for wrong in ["JSON", "logfmt", "pretty"] {
            assert!(LogFormat::parse(Some(wrong)).is_err(), "{wrong}");
        }
    }
}
