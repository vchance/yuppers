//! The network address a request came from, as far as it can be known.
//!
//! Behind a reverse proxy the peer of every connection is the proxy, and the
//! real address is in a header the proxy adds. Any client can send such a
//! header too, so it is believed only when the deployment names it
//! (`TRUSTED_PROXY_HEADER`) and says how many proxies in a row add to it
//! (`TRUSTED_PROXIES`). By default no header is trusted and the peer address
//! is the answer.
//!
//! The address is recorded with a signature (DESIGN.md §8, §13.4) and is what
//! the sign-in limits count requesters by (`crate::auth::Requester`).

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName};

use super::AppState;

/// Which header, if any, carries the client's address, and how many proxies
/// the request passes through on its way here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedProxies {
    header: Option<HeaderName>,
    count: usize,
}

impl TrustedProxies {
    /// Whether a proxy header is trusted at all.
    pub fn trusts_a_header(&self) -> bool {
        self.header.is_some()
    }

    /// No proxy: the peer of the connection is the client.
    pub fn none() -> Self {
        Self {
            header: None,
            count: 0,
        }
    }

    /// `count` proxies in a row, each appending the address it received the
    /// request from to `header`, as `X-Forwarded-For` works. For a header
    /// that holds one address only, such as a CDN's own, `count` is 1.
    pub fn behind(header: HeaderName, count: usize) -> Self {
        Self {
            header: Some(header),
            count: count.max(1),
        }
    }

    /// The client's address, given the connection's peer and the request
    /// headers.
    ///
    /// With `count` trusted proxies, the last `count - 1` entries of the
    /// header were added by proxies about the proxies before them, so the
    /// client is the entry `count` from the end. Anything a client put in the
    /// header itself is further left and never reached. A header shorter
    /// than that was written entirely by trusted proxies, so its first entry
    /// is taken. A header that is missing or unreadable, which a trusted
    /// proxy never sends, falls back to the peer, which is the proxy: every
    /// such request is then counted as one requester, so it is logged as a
    /// warning, at most about once a minute.
    pub fn client_address(&self, peer: Option<IpAddr>, headers: &HeaderMap) -> Option<IpAddr> {
        let Some(header) = &self.header else {
            return peer;
        };
        let entries: Vec<&str> = headers
            .get_all(header)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect();
        let index = entries.len().saturating_sub(self.count);
        let found = entries.get(index).and_then(|entry| parse_address(entry));
        if found.is_none() && time_to_warn(&LAST_WARNING, now_seconds()) {
            tracing::warn!(
                header = header.as_str(),
                present = !entries.is_empty(),
                "the trusted proxy header is missing or unreadable, so the proxy's own address \
                 is taken as the requester's; requests like this share one sign-in limit \
                 (check TRUSTED_PROXY_HEADER and TRUSTED_PROXIES)"
            );
        }
        found.or(peer)
    }
}

/// When the last warning about an unusable proxy header was logged, in
/// seconds since the epoch.
static LAST_WARNING: AtomicU64 = AtomicU64::new(0);

/// How often that warning may be logged: once is enough to be noticed, and
/// one per request would bury everything else.
const WARNING_INTERVAL_SECONDS: u64 = 60;

pub(super) fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Whether to warn now, given when the last warning was. Claims the moment,
/// so of requests arriving together only one warns.
pub(super) fn time_to_warn(last: &AtomicU64, now: u64) -> bool {
    let before = last.load(Ordering::Relaxed);
    if before != 0 && now.saturating_sub(before) < WARNING_INTERVAL_SECONDS {
        return false;
    }
    last.compare_exchange(before, now.max(1), Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
}

/// Reads an address as proxies write them: `203.0.113.7`, `2001:db8::7`,
/// `203.0.113.7:4242`, `[2001:db8::7]:4242` or `[2001:db8::7]`.
fn parse_address(entry: &str) -> Option<IpAddr> {
    if let Ok(address) = entry.parse::<IpAddr>() {
        return Some(address);
    }
    if let Ok(address) = entry.parse::<SocketAddr>() {
        return Some(address.ip());
    }
    entry
        .strip_prefix('[')?
        .strip_suffix(']')?
        .parse::<IpAddr>()
        .ok()
}

/// The requester's address, or nothing when it cannot be known (a request
/// made in-process, as the tests do).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientAddress(pub Option<IpAddr>);

impl FromRequestParts<AppState> for ClientAddress {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Infallible> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(address)| address.ip());
        Ok(Self(
            state.settings.proxies.client_address(peer, &parts.headers),
        ))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    const PEER: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, 0, 1));

    #[test]
    fn the_warning_about_an_unusable_header_comes_at_most_once_a_minute() {
        let last = AtomicU64::new(0);
        assert!(time_to_warn(&last, 1_000));
        assert!(!time_to_warn(&last, 1_000));
        assert!(!time_to_warn(&last, 1_059));
        assert!(time_to_warn(&last, 1_060));
        assert!(!time_to_warn(&last, 1_061));
    }
    const CLIENT: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 7));

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    fn forwarded() -> HeaderName {
        HeaderName::from_static("x-forwarded-for")
    }

    #[test]
    fn without_a_trusted_proxy_the_peer_is_the_client_whatever_the_headers_say() {
        let proxies = TrustedProxies::none();
        let spoofed = headers(&[("x-forwarded-for", "203.0.113.7")]);
        assert_eq!(proxies.client_address(Some(PEER), &spoofed), Some(PEER));
        assert_eq!(proxies.client_address(None, &spoofed), None);
    }

    #[test]
    fn a_header_other_than_the_trusted_one_is_ignored() {
        let proxies = TrustedProxies::behind(HeaderName::from_static("cf-connecting-ip"), 1);
        let other = headers(&[
            ("x-forwarded-for", "203.0.113.7"),
            ("x-real-ip", "203.0.113.8"),
        ]);
        assert_eq!(proxies.client_address(Some(PEER), &other), Some(PEER));
        let right = headers(&[("cf-connecting-ip", "203.0.113.7")]);
        assert_eq!(proxies.client_address(Some(PEER), &right), Some(CLIENT));
    }

    #[test]
    fn the_client_is_the_entry_as_many_from_the_end_as_there_are_proxies() {
        let chain = "198.51.100.1, 203.0.113.7, 10.0.0.2, 10.0.0.3";
        // Three proxies: the two rightmost entries name proxies, and the
        // third from the right is who reached the first proxy. The leftmost
        // entry is what the client sent itself.
        let three = TrustedProxies::behind(forwarded(), 3);
        assert_eq!(
            three.client_address(Some(PEER), &headers(&[("x-forwarded-for", chain)])),
            Some(CLIENT)
        );
        // One proxy: the last entry, whatever a client prepended.
        let one = TrustedProxies::behind(forwarded(), 1);
        assert_eq!(
            one.client_address(Some(PEER), &headers(&[("x-forwarded-for", chain)])),
            Some("10.0.0.3".parse().unwrap())
        );
        // Several header lines are one list.
        let lines = headers(&[
            ("x-forwarded-for", "198.51.100.1"),
            ("x-forwarded-for", "203.0.113.7, 10.0.0.2"),
            ("x-forwarded-for", "10.0.0.3"),
        ]);
        assert_eq!(three.client_address(Some(PEER), &lines), Some(CLIENT));
    }

    #[test]
    fn a_header_shorter_than_the_chain_was_written_by_proxies_alone() {
        let three = TrustedProxies::behind(forwarded(), 3);
        let short = headers(&[("x-forwarded-for", "203.0.113.7, 10.0.0.2")]);
        assert_eq!(three.client_address(Some(PEER), &short), Some(CLIENT));
    }

    #[test]
    fn a_missing_or_unreadable_header_falls_back_to_the_peer() {
        let one = TrustedProxies::behind(forwarded(), 1);
        assert_eq!(
            one.client_address(Some(PEER), &HeaderMap::new()),
            Some(PEER)
        );
        let junk = headers(&[("x-forwarded-for", "not-an-address")]);
        assert_eq!(one.client_address(Some(PEER), &junk), Some(PEER));
        assert_eq!(one.client_address(None, &junk), None);
    }

    #[test]
    fn addresses_are_read_as_proxies_write_them() {
        let v6: IpAddr = "2001:db8::7".parse().unwrap();
        assert_eq!(parse_address("203.0.113.7"), Some(CLIENT));
        assert_eq!(parse_address("203.0.113.7:4242"), Some(CLIENT));
        assert_eq!(parse_address("2001:db8::7"), Some(v6));
        assert_eq!(parse_address("[2001:db8::7]:4242"), Some(v6));
        assert_eq!(parse_address("[2001:db8::7]"), Some(v6));
        assert_eq!(parse_address("unknown"), None);
        assert_eq!(parse_address(""), None);
    }

    #[test]
    fn at_least_one_proxy_is_assumed_when_a_header_is_trusted() {
        assert_eq!(
            TrustedProxies::behind(forwarded(), 0),
            TrustedProxies::behind(forwarded(), 1)
        );
    }
}
