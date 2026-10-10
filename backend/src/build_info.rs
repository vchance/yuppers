//! Which build is running: the package version, the git commit it was built
//! from, and when it was built (docs/operations.md, "What is deployed").
//!
//! The commit and the time are put in when the image is built: the
//! `Dockerfile` passes `GIT_SHA` and `BUILD_TIME` to `cargo build`, which
//! reads them here with `option_env!`. Nothing runs git during a build. A
//! build without them (any `cargo build` on a development machine) still
//! builds; the process then looks at the same names in its environment at
//! run time, and at `RENDER_GIT_COMMIT`, which Render sets on every service
//! it builds from a repository. With none of them the commit is `unknown`.
//!
//! What a value must look like is checked, so whatever is printed in a
//! header, a log line or a metric label is a hexadecimal commit and an
//! RFC 3339 time, or `unknown`.

use std::sync::OnceLock;

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::metrics::{BUILD_INFO, Kind, Text};

/// What a commit or time is shown as when the build does not say.
pub const UNKNOWN: &str = "unknown";

/// How many characters of the commit the short form keeps: git's default.
const SHORT: usize = 7;

/// The header every response names the build in: the short commit.
pub const HEADER: &str = "x-yuppers-version";

/// This build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    /// The package version, `CARGO_PKG_VERSION`.
    pub version: &'static str,
    /// The full commit, lowercase hexadecimal, if known.
    commit: Option<String>,
    /// When it was built, as `2026-10-03T12:00:00Z`, if known.
    built_at: Option<String>,
}

impl Default for BuildInfo {
    /// A build that says nothing about where it came from.
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            commit: None,
            built_at: None,
        }
    }
}

impl BuildInfo {
    /// The running build: what was compiled in, else what the environment
    /// says. Read once.
    pub fn current() -> &'static BuildInfo {
        static CURRENT: OnceLock<BuildInfo> = OnceLock::new();
        CURRENT.get_or_init(|| {
            Self::resolve(option_env!("GIT_SHA"), option_env!("BUILD_TIME"), &|name| {
                std::env::var(name).ok()
            })
        })
    }

    /// From the values compiled in, falling back to the environment
    /// (`GIT_SHA` then `RENDER_GIT_COMMIT`, and `BUILD_TIME`). A value that
    /// does not read as a commit or a time is passed over like a missing one.
    pub fn resolve(
        built_commit: Option<&str>,
        built_at: Option<&str>,
        runtime: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        let commit = built_commit
            .and_then(commit)
            .or_else(|| runtime("GIT_SHA").as_deref().and_then(commit))
            .or_else(|| runtime("RENDER_GIT_COMMIT").as_deref().and_then(commit));
        let built_at = built_at
            .and_then(timestamp)
            .or_else(|| runtime("BUILD_TIME").as_deref().and_then(timestamp));
        Self {
            commit,
            built_at,
            ..Self::default()
        }
    }

    /// The full commit, or `unknown`.
    pub fn commit(&self) -> &str {
        self.commit.as_deref().unwrap_or(UNKNOWN)
    }

    /// The commit's first seven characters, or `unknown`.
    pub fn short_commit(&self) -> &str {
        self.commit
            .as_deref()
            .map(|commit| &commit[..SHORT.min(commit.len())])
            .unwrap_or(UNKNOWN)
    }

    /// When it was built, if known.
    pub fn built_at(&self) -> Option<&str> {
        self.built_at.as_deref()
    }

    /// One line at a process's start, so every log says what was running.
    pub fn log_start(&self, process: &str) {
        tracing::info!(
            process,
            version = self.version,
            commit = self.commit(),
            built_at = self.built_at().unwrap_or(UNKNOWN),
            "build"
        );
    }

    /// `yuppers_build_info{version,commit} 1`, the usual way to put a build
    /// beside the numbers it produced.
    pub fn render_metrics(&self, text: &mut Text) {
        text.family(
            BUILD_INFO,
            Kind::Gauge,
            "The running build: package version and git commit. Always 1.",
        );
        text.sample(
            BUILD_INFO,
            &[("version", self.version), ("commit", self.commit())],
            1.0,
        );
    }
}

/// A git commit: 7 to 64 hexadecimal characters (SHA-1 or SHA-256, or a
/// short form), as lowercase.
fn commit(value: &str) -> Option<String> {
    let value = value.trim();
    ((SHORT..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

/// An RFC 3339 time, written back in UTC to the second.
fn timestamp(value: &str) -> Option<String> {
    let time = OffsetDateTime::parse(value.trim(), &Rfc3339).ok()?;
    let utc = time.to_offset(time::UtcOffset::UTC);
    let format =
        time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    utc.format(&format).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn none(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn the_values_compiled_in_come_first() {
        let runtime = |name: &str| match name {
            "GIT_SHA" => Some("fedcba9".to_owned()),
            "BUILD_TIME" => Some("2020-01-01T00:00:00Z".to_owned()),
            _ => None,
        };
        let build = BuildInfo::resolve(Some(SHA), Some("2026-10-03T12:34:56Z"), &runtime);
        assert_eq!(build.commit(), SHA);
        assert_eq!(build.short_commit(), "0123456");
        assert_eq!(build.built_at(), Some("2026-10-03T12:34:56Z"));
        assert_eq!(build.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_environment_stands_in_for_a_build_that_did_not_say() {
        let runtime = |name: &str| match name {
            "RENDER_GIT_COMMIT" => Some(SHA.to_uppercase()),
            "BUILD_TIME" => Some("2026-10-03T14:34:56+02:00".to_owned()),
            _ => None,
        };
        let build = BuildInfo::resolve(None, None, &runtime);
        assert_eq!(build.commit(), SHA, "lowercased");
        assert_eq!(build.built_at(), Some("2026-10-03T12:34:56Z"), "in UTC");

        let both = |name: &str| match name {
            "GIT_SHA" => Some("abcdef1".to_owned()),
            "RENDER_GIT_COMMIT" => Some(SHA.to_owned()),
            _ => None,
        };
        assert_eq!(BuildInfo::resolve(None, None, &both).commit(), "abcdef1");
    }

    #[test]
    fn without_any_it_is_unknown() {
        let build = BuildInfo::resolve(None, None, &none);
        assert_eq!(build, BuildInfo::default());
        assert_eq!(build.commit(), "unknown");
        assert_eq!(build.short_commit(), "unknown");
        assert_eq!(build.built_at(), None);
    }

    #[test]
    fn what_does_not_read_as_a_commit_or_a_time_is_passed_over() {
        for bad in [
            "",
            "abc",
            "not-a-commit",
            "0123456 789",
            "g123456",
            &"a".repeat(65),
        ] {
            let build = BuildInfo::resolve(Some(bad), Some(bad), &none);
            assert_eq!(build.commit(), "unknown", "{bad:?}");
            assert_eq!(build.built_at(), None, "{bad:?}");
        }
        // An empty build argument, as a Docker build without one passes,
        // still falls back to the environment.
        let runtime = |name: &str| (name == "GIT_SHA").then(|| SHA.to_owned());
        assert_eq!(
            BuildInfo::resolve(Some(""), Some(""), &runtime).commit(),
            SHA
        );
    }

    #[test]
    fn the_metric_names_the_version_and_commit() {
        let mut text = Text::new();
        BuildInfo::resolve(Some(SHA), None, &none).render_metrics(&mut text);
        let page = text.finish();
        assert!(page.contains("# TYPE yuppers_build_info gauge"), "{page}");
        assert!(
            page.contains(&format!(
                "yuppers_build_info{{version=\"{}\",commit=\"{SHA}\"}} 1",
                env!("CARGO_PKG_VERSION")
            )),
            "{page}"
        );
    }
}
