//! Yuppers backend. One crate, several processes: `api`, `worker`, `migrate`,
//! `openapi` and `replay-deletions` under `src/bin` all build on this library
//! (DESIGN.md §13.3).

pub mod app_role;
pub mod auth;
pub mod build_info;
pub mod client_version;
pub mod code_consent;
pub mod config;
pub mod contact;
pub mod db;
pub mod deletion;
pub mod deletion_log;
pub mod domain;
pub mod error;
pub mod exchanges;
pub mod http;
pub mod languages;
pub mod metrics;
pub mod nanp;
pub mod notifications;
pub mod outbound;
pub mod review;
pub mod safety;
pub mod shutdown;
pub mod telemetry;
pub mod wallet;
