//! Expo's push service: the push adapter at the edge (DESIGN.md §13.1).
//!
//! The app registers an Expo push token (`expo-notifications`), and Expo
//! passes each message on to Apple's or Google's service with the
//! credentials the owner uploads to the Expo project. Two requests:
//!
//! * `POST /--/api/v2/push/send`, up to 100 messages, answered with one
//!   ticket per message, in order;
//! * `POST /--/api/v2/push/getReceipts`, up to 1,000 ticket IDs, answered
//!   with the receipts that are ready.
//!
//! With push security turned on for the Expo project, every request needs
//! the project's access token (`EXPO_ACCESS_TOKEN`); without it, none does.
//!
//! What this file never does: keep or log the service's message text, which
//! can quote the token. Only the service's error names are kept.

use std::collections::HashMap;
use std::time::Duration;

use hyper::http::{HeaderName, HeaderValue, StatusCode, header};
use serde::Deserialize;
use serde_json::{Value, json};

use super::push::{PushFuture, PushMessage, PushSender, Receipt, Refusal, Ticket};
use super::smtp::Secret;
use crate::outbound::{Answer, Client};

/// Where Expo's push service is.
pub const EXPO_ORIGIN: &str = "https://exp.host";
const SEND_PATH: &str = "/--/api/v2/push/send";
const RECEIPTS_PATH: &str = "/--/api/v2/push/getReceipts";

/// The Android notification channel the app creates, with its name in the
/// person's language, before it registers. Messages name it, so that they
/// appear under that name in the system's settings.
pub const ANDROID_CHANNEL: &str = "default";

/// Sends through Expo's push service.
pub struct ExpoPushSender {
    client: Client,
    origin: String,
    access_token: Option<Secret>,
}

impl ExpoPushSender {
    /// A sender to the service at `origin`: [`EXPO_ORIGIN`], or a stand-in
    /// in a test. Each request must be answered within `timeout`.
    pub fn new(origin: &str, access_token: Option<Secret>, timeout: Duration) -> Self {
        Self {
            client: Client::new(timeout),
            origin: origin.trim_end_matches('/').to_owned(),
            access_token,
        }
    }

    fn headers(&self) -> Vec<(HeaderName, HeaderValue)> {
        let mut headers = vec![
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::ACCEPT, HeaderValue::from_static("application/json")),
        ];
        if let Some(token) = &self.access_token
            && let Ok(mut value) = HeaderValue::from_str(&format!("Bearer {}", token.expose()))
        {
            value.set_sensitive(true);
            headers.push((header::AUTHORIZATION, value));
        }
        headers
    }

    async fn post(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let answer = self
            .client
            .post(
                &format!("{}{path}", self.origin),
                &self.headers(),
                serde_json::to_vec(body)?,
            )
            .await?;
        read(&answer)
    }
}

/// The answer's JSON, or why there is none to use: the status, and Expo's
/// own error code if it gave one, never its message.
fn read(answer: &Answer) -> anyhow::Result<Value> {
    let body: Option<Value> = serde_json::from_slice(&answer.body).ok();
    let code = body
        .as_ref()
        .and_then(|body| body["errors"][0]["code"].as_str())
        .map(|code| code.chars().take(64).collect::<String>());
    if answer.status != StatusCode::OK || body.as_ref().is_some_and(|body| body["data"].is_null()) {
        let kind = if answer.status == StatusCode::TOO_MANY_REQUESTS {
            "turned the request away for now"
        } else {
            "refused the request"
        };
        return Err(match code {
            Some(code) => anyhow::anyhow!(
                "the push service {kind} (HTTP {}, {code})",
                answer.status.as_u16()
            ),
            None => anyhow::anyhow!("the push service {kind} (HTTP {})", answer.status.as_u16()),
        });
    }
    body.ok_or_else(|| anyhow::anyhow!("the push service's answer could not be read"))
}

#[derive(Deserialize)]
struct Item {
    status: String,
    id: Option<String>,
    #[serde(default)]
    details: Option<Details>,
}

#[derive(Deserialize)]
struct Details {
    error: Option<String>,
}

/// What an `error` item says went wrong, by name.
fn refusal(details: Option<Details>) -> Refusal {
    match details.and_then(|details| details.error) {
        Some(name) if name == "DeviceNotRegistered" => Refusal::DeviceNotRegistered,
        Some(name) => Refusal::Other(name.chars().take(64).collect()),
        None => Refusal::Other("error".to_owned()),
    }
}

/// The request body for a batch of messages.
pub fn send_body(messages: &[PushMessage]) -> Value {
    Value::Array(
        messages
            .iter()
            .map(|message| {
                let mut item = json!({
                    "to": message.to,
                    "body": message.body,
                    "data": message.data,
                    "sound": "default",
                    // Shown at once, not held until the device wakes: these
                    // are messages a person sees, few and far between.
                    "priority": "high",
                    "channelId": ANDROID_CHANNEL,
                });
                if let Some(title) = &message.title {
                    item["title"] = json!(title);
                }
                item
            })
            .collect(),
    )
}

impl PushSender for ExpoPushSender {
    fn send<'a>(&'a self, messages: &'a [PushMessage]) -> PushFuture<'a, Vec<Ticket>> {
        Box::pin(async move {
            let answer = self.post(SEND_PATH, &send_body(messages)).await?;
            let items: Vec<Item> = serde_json::from_value(answer["data"].clone())
                .map_err(|_| anyhow::anyhow!("the push service's tickets could not be read"))?;
            Ok(items
                .into_iter()
                .map(|item| match item.status.as_str() {
                    "ok" => Ticket::Accepted { receipt: item.id },
                    _ => Ticket::Refused(refusal(item.details)),
                })
                .collect())
        })
    }

    fn receipts<'a>(&'a self, ids: &'a [String]) -> PushFuture<'a, HashMap<String, Receipt>> {
        Box::pin(async move {
            let answer = self.post(RECEIPTS_PATH, &json!({ "ids": ids })).await?;
            let items: HashMap<String, Item> = serde_json::from_value(answer["data"].clone())
                .map_err(|_| anyhow::anyhow!("the push service's receipts could not be read"))?;
            Ok(items
                .into_iter()
                .map(|(id, item)| {
                    let receipt = match item.status.as_str() {
                        "ok" => Receipt::Delivered,
                        _ => Receipt::Refused(refusal(item.details)),
                    };
                    (id, receipt)
                })
                .collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Bytes;

    use super::*;

    fn answer(status: u16, body: &str) -> Answer {
        Answer {
            status: StatusCode::from_u16(status).unwrap(),
            body: Bytes::from(body.to_owned()),
        }
    }

    #[test]
    fn a_refusal_is_described_by_its_status_and_code_and_never_its_message() {
        let token = "ExponentPushToken[secret-token]";
        let refused = answer(
            400,
            &format!(
                r#"{{"errors":[{{"code":"VALIDATION_ERROR","message":"\"{token}\" is wrong"}}]}}"#
            ),
        );
        let error = format!("{:#}", read(&refused).unwrap_err());
        assert_eq!(
            error,
            "the push service refused the request (HTTP 400, VALIDATION_ERROR)"
        );
        assert!(!error.contains(token));

        let busy = format!("{:#}", read(&answer(429, "")).unwrap_err());
        assert_eq!(
            busy,
            "the push service turned the request away for now (HTTP 429)"
        );
        assert!(read(&answer(200, r#"{"errors":[{"code":"X"}]}"#)).is_err());
        assert!(read(&answer(200, r#"{"data":[]}"#)).is_ok());
    }

    #[test]
    fn the_access_token_is_sent_only_when_there_is_one_and_never_shown() {
        let without = ExpoPushSender::new(EXPO_ORIGIN, None, Duration::from_secs(1));
        assert!(
            !without
                .headers()
                .iter()
                .any(|(name, _)| name == header::AUTHORIZATION)
        );
        let with = ExpoPushSender::new(
            EXPO_ORIGIN,
            Some(Secret::new("expo-access".to_owned())),
            Duration::from_secs(1),
        );
        let headers = with.headers();
        let (_, value) = headers
            .iter()
            .find(|(name, _)| name == header::AUTHORIZATION)
            .unwrap();
        assert_eq!(value.to_str().unwrap(), "Bearer expo-access");
        assert!(value.is_sensitive());
        assert!(!format!("{value:?}").contains("expo-access"));
    }
}
