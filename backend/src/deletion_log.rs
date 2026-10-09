//! Replaying the deletion log onto a restored database (docs/operations.md,
//! "Restoring"; the `replay-deletions` binary and
//! `scripts/replay-deletions.sh`).
//!
//! A backup restored brings back every account deleted since it was taken.
//! The log of deletions (migration 0015), exported to a file beside each
//! backup by `scripts/backup.sh` or from the live database by
//! `scripts/export-deletions.sh`, says which. Each one is applied again
//! through `deletion::replay`, the code a person's own deletion runs, so
//! that every rule runs again rather than a copy of them in SQL.
//!
//! The file is text, one deletion per line: the account's ID and the time of
//! the deletion in RFC 3339, separated by white space. A line for an account
//! that was combined into another (`crate::combine`, migration 0028) goes
//! on with that account's ID and `EMAIL` or `PHONE`, the kind of identifier
//! the two were combined by; replaying it combines them again
//! (`combine::replay`). Blank lines and lines starting with `#` are ignored.
//! Nothing else is in it.

use std::fmt;

use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::combine::{self, IdentifierKind};
use crate::deletion::{self, Replayed};
use crate::domain::Rules;
use crate::error::Redacted;

/// One line of the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub account: Uuid,
    pub deleted_at: OffsetDateTime,
    /// For an account combined into another rather than deleted: which,
    /// and by which kind of identifier.
    pub merged: Option<(Uuid, IdentifierKind)>,
}

/// A line that is not a deletion. The whole file is refused for it, before
/// anything is changed: a damaged log is not half replayed.
#[derive(Debug, PartialEq, Eq)]
pub struct BadLine {
    pub number: usize,
    pub reason: &'static str,
}

impl fmt::Display for BadLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.number, self.reason)
    }
}

impl std::error::Error for BadLine {}

/// Reads a log file's text.
pub fn parse(text: &str) -> Result<Vec<Entry>, BadLine> {
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |reason| BadLine { number, reason };
        let words: Vec<&str> = line.split_whitespace().collect();
        let (account, deleted_at, merged) = match words.as_slice() {
            [account, deleted_at] => (*account, *deleted_at, None),
            [account, deleted_at, into, by] => (*account, *deleted_at, Some((*into, *by))),
            [_, _, _] => return Err(bad("expected the kind of identifier after the account")),
            _ => return Err(bad("expected an account ID and a time")),
        };
        let account = Uuid::parse_str(account).map_err(|_| bad("not an account ID"))?;
        let deleted_at =
            OffsetDateTime::parse(deleted_at, &Rfc3339).map_err(|_| bad("not an RFC 3339 time"))?;
        let merged = match merged {
            None => None,
            Some((into, by)) => {
                let into = Uuid::parse_str(into).map_err(|_| bad("not an account ID"))?;
                let by = match by {
                    "EMAIL" => IdentifierKind::Email,
                    "PHONE" => IdentifierKind::Phone,
                    _ => return Err(bad("not EMAIL or PHONE")),
                };
                Some((into, by))
            }
        };
        entries.push(Entry {
            account,
            deleted_at,
            merged,
        });
    }
    Ok(entries)
}

/// What a replay did, counted.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Live in the restored database, and deleted again; those that were
    /// suspended there included.
    pub deleted: usize,
    /// Of those deleted again, the ones suspended in the restored database,
    /// whose suspension was lifted first (`deletion::replay`). Listed, so
    /// that whoever restores can tell the reviewers.
    pub lifted: Vec<Uuid>,
    pub already_deleted: usize,
    /// Combined into another account again (`combine::replay`).
    pub combined: usize,
    /// Combined here already.
    pub already_combined: usize,
    pub not_here: usize,
    /// Could not be deleted this time (the database refused or was busy).
    /// Replaying again tries them again.
    pub failed: Vec<Uuid>,
    /// Left alone because the log's time is before the account was created,
    /// or before it was last suspended, here
    /// (`deletion::Replayed::Contradicted`). Replaying again changes nothing:
    /// whoever restores looks into where the log came from, and removes the
    /// line once satisfied that it is wrong.
    pub contradicted: Vec<Uuid>,
    /// Combined into an account this database does not hold, and that no
    /// later line of the log follows to a deletion or to an account it
    /// holds: left as they are, live. Replaying again changes nothing;
    /// whoever restores looks into them by hand.
    pub unresolved: Vec<Uuid>,
}

impl Summary {
    /// Every account in the log is deleted or combined, or was never here,
    /// the database contradicted no line, and every account combined into
    /// one it does not hold was followed to where it went.
    pub fn complete(&self) -> bool {
        self.failed.is_empty() && self.contradicted.is_empty() && self.unresolved.is_empty()
    }
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} deleted again ({} of them suspended here, their suspension lifted), {} already \
             deleted, {} not in this database, {} failed",
            self.deleted,
            self.lifted.len(),
            self.already_deleted,
            self.not_here,
            self.failed.len()
        )?;
        if self.combined + self.already_combined > 0 {
            write!(
                f,
                ", {} combined again, {} already combined",
                self.combined, self.already_combined
            )?;
        }
        if !self.unresolved.is_empty() {
            write!(
                f,
                ", {} UNRESOLVED: combined into an account neither this database nor the log \
                 accounts for",
                self.unresolved.len()
            )?;
        }
        if !self.contradicted.is_empty() {
            write!(
                f,
                ", {} left alone because this database contradicts the log",
                self.contradicted.len()
            )?;
        }
        Ok(())
    }
}

/// Applies each deletion in turn, in the order of the log, and reports each
/// one to `report` as it goes. A failure is reported and counted, and the
/// rest still run: every step can be repeated, so the remedy for a failure
/// is to replay the same file again.
///
/// An account combined into one this database does not hold (made after
/// the backup) is followed through the later lines of the log: where that
/// account was deleted, it is deleted at that time; where it was combined
/// into one this database holds, it is combined into that one. Where the
/// log says neither, it is left as it is, reported, and the replay is not
/// complete.
pub async fn replay(
    db: &PgPool,
    rules: &Rules,
    entries: &[Entry],
    mut report: impl FnMut(&str),
) -> Summary {
    let mut summary = Summary::default();
    for (position, entry) in entries.iter().enumerate() {
        let Entry {
            account,
            deleted_at,
            merged,
        } = *entry;
        let when = deleted_at.format(&Rfc3339).unwrap_or_default();
        let Some((into, by)) = merged else {
            delete_again(
                db,
                rules,
                account,
                deleted_at,
                "",
                &mut summary,
                &mut report,
            )
            .await;
            continue;
        };
        let mut target = into;
        let mut outcome = combine::replay(db, rules, account, target, by, deleted_at).await;
        if let Ok(combine::Replayed::TargetNotHere) = outcome {
            match follow(db, entries, position, into).await {
                Ok(Followed::Deleted(at)) => {
                    let because =
                        format!(" as {into}, which it was combined into {when}, was deleted then");
                    delete_again(db, rules, account, at, &because, &mut summary, &mut report).await;
                    continue;
                }
                Ok(Followed::Into(found)) => {
                    target = found;
                    outcome = combine::replay(db, rules, account, target, by, deleted_at).await;
                }
                Ok(Followed::Unresolved) => {
                    summary.unresolved.push(account);
                    report(&format!(
                        "{account}: UNRESOLVED: it was combined into {into} {when}, which is not \
                         in this database, and no later line of the log says where that account \
                         went; it is left as it is, a live account. Find the log that does, or \
                         look into it by hand (docs/operations.md, \"Restoring\")"
                    ));
                    continue;
                }
                Err(error) => {
                    summary.failed.push(account);
                    report(&format!(
                        "{account}: FAILED ({}); replay again to retry",
                        Redacted(&error)
                    ));
                    continue;
                }
            }
        }
        match outcome {
            Ok(combine::Replayed::Combined) => {
                summary.combined += 1;
                let through = if target == into {
                    String::new()
                } else {
                    format!(", through {into}, which is not in this database")
                };
                report(&format!(
                    "{account}: combined into {target} again (combined {when}{through})"
                ));
            }
            Ok(combine::Replayed::AlreadyCombined) => {
                summary.already_combined += 1;
                report(&format!("{account}: already combined"));
            }
            Ok(combine::Replayed::NotHere) => {
                summary.not_here += 1;
                report(&format!("{account}: not in this database"));
            }
            Ok(combine::Replayed::TargetNotHere) => {
                // Followed to an account found a moment ago, gone since.
                summary.failed.push(account);
                report(&format!(
                    "{account}: FAILED ({target} went while replaying); replay again to retry"
                ));
            }
            Ok(combine::Replayed::Contradicted) => {
                summary.contradicted.push(account);
                report(&format!(
                    "{account}: LEFT ALONE: the log says it was combined into {target} {when}, \
                     and this database does not allow it (one of them is deleted, suspended \
                     or a reviewer, or they share a yup); check where this log came from \
                     (nothing was done)"
                ));
            }
            Err(error) => {
                summary.failed.push(account);
                report(&format!(
                    "{account}: FAILED ({:?}); replay again to retry",
                    error.code
                ));
            }
        }
    }
    summary
}

/// Where the log says an account this database does not hold went.
#[derive(Debug, PartialEq, Eq)]
enum Followed {
    /// Deleted, at this time.
    Deleted(OffsetDateTime),
    /// Combined, perhaps through others this database does not hold, into
    /// this one, which it does.
    Into(Uuid),
    /// No later line says.
    Unresolved,
}

/// Follows `missing`, an account this database does not hold, through the
/// lines after `position`.
async fn follow(
    db: &PgPool,
    entries: &[Entry],
    position: usize,
    missing: Uuid,
) -> Result<Followed, sqlx::Error> {
    let (mut missing, mut from) = (missing, position);
    // Each step moves on in the log, so this ends.
    loop {
        let Some((found, next)) = entries
            .iter()
            .enumerate()
            .skip(from + 1)
            .find(|(_, entry)| entry.account == missing)
        else {
            return Ok(Followed::Unresolved);
        };
        let Some((into, _)) = next.merged else {
            return Ok(Followed::Deleted(next.deleted_at));
        };
        let here: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM account WHERE id = $1)")
            .bind(into)
            .fetch_one(db)
            .await?;
        if here {
            return Ok(Followed::Into(into));
        }
        (missing, from) = (into, found);
    }
}

/// Deletes `account` again, as the log says it was at `deleted_at`, and
/// reports it, with `because` after the time where it was not the account's
/// own line.
async fn delete_again(
    db: &PgPool,
    rules: &Rules,
    account: Uuid,
    deleted_at: OffsetDateTime,
    because: &str,
    summary: &mut Summary,
    report: &mut impl FnMut(&str),
) {
    let when = deleted_at.format(&Rfc3339).unwrap_or_default();
    match deletion::replay(db, rules, account, deleted_at).await {
        Ok(Replayed::Deleted) => {
            summary.deleted += 1;
            report(&format!(
                "{account}: deleted again (deleted {when}{because})"
            ));
        }
        Ok(Replayed::AlreadyDeleted) => {
            summary.already_deleted += 1;
            report(&format!("{account}: already deleted"));
        }
        Ok(Replayed::NotHere) => {
            summary.not_here += 1;
            report(&format!("{account}: not in this database"));
        }
        Ok(Replayed::SuspensionLiftedAndDeleted) => {
            summary.deleted += 1;
            summary.lifted.push(account);
            report(&format!(
                "{account}: deleted again (deleted {when}{because}); it was suspended here, and \
                 the suspension was lifted first, in the review history as the owner's"
            ));
        }
        Ok(Replayed::Contradicted) => {
            summary.contradicted.push(account);
            report(&format!(
                "{account}: LEFT ALONE: the log says it was deleted {when}{because}, before this \
                 database says it was created or last suspended; check where this log came \
                 from (nothing was done)"
            ));
        }
        Err(error) => {
            summary.failed.push(account);
            report(&format!(
                "{account}: FAILED ({:?}); replay again to retry",
                error.code
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    const ANA: &str = "0b6f6a43-5f4e-4c64-9d4c-0f3f0c1f7a01";
    const BEN: &str = "6a8c2a5e-2f9b-4a8e-b0a7-8f1e2d3c4b02";

    #[test]
    fn a_log_is_an_id_and_a_time_per_line_with_comments_and_blank_lines_ignored() {
        let text = format!(
            "# Yuppers deletion log\n\n{ANA}\t2026-10-03T09:15:00.123456Z\n  {BEN}  \
             2026-10-03T10:00:00+02:00  \n# the end\n"
        );
        assert_eq!(
            parse(&text),
            Ok(vec![
                Entry {
                    account: ANA.parse().unwrap(),
                    deleted_at: datetime!(2026-10-03 09:15:00.123456 UTC),
                    merged: None,
                },
                Entry {
                    account: BEN.parse().unwrap(),
                    deleted_at: datetime!(2026-10-03 08:00:00 UTC),
                    merged: None,
                },
            ])
        );
    }

    #[test]
    fn a_combined_account_names_the_account_it_went_into_and_by_which_kind() {
        // As `scripts/export-deletions.sh` writes it: tab-separated, and a
        // deletion's empty columns at the end.
        let text =
            format!("{ANA}\t2026-10-03T09:15:00Z\t{BEN}\tPHONE\n{BEN}\t2026-10-04T09:15:00Z\t\t\n");
        assert_eq!(
            parse(&text),
            Ok(vec![
                Entry {
                    account: ANA.parse().unwrap(),
                    deleted_at: datetime!(2026-10-03 09:15:00 UTC),
                    merged: Some((BEN.parse().unwrap(), IdentifierKind::Phone)),
                },
                Entry {
                    account: BEN.parse().unwrap(),
                    deleted_at: datetime!(2026-10-04 09:15:00 UTC),
                    merged: None,
                },
            ])
        );
        assert_eq!(
            parse(&format!("{ANA} 2026-10-03T09:15:00Z {BEN} FAX\n")),
            Err(BadLine {
                number: 1,
                reason: "not EMAIL or PHONE"
            })
        );
    }

    #[test]
    fn an_empty_log_replays_nothing() {
        assert_eq!(parse(""), Ok(vec![]));
        assert_eq!(parse("# Yuppers deletion log\n"), Ok(vec![]));
    }

    #[test]
    fn a_damaged_line_refuses_the_whole_file_and_says_where() {
        let good = format!("{ANA} 2026-10-03T09:15:00Z\n");
        for (bad, reason) in [
            (format!("{ANA}\n"), "expected an account ID and a time"),
            (
                format!("{ANA} 2026-10-03T09:15:00Z extra\n"),
                "expected the kind of identifier after the account",
            ),
            (
                format!("{ANA} 2026-10-03T09:15:00Z {BEN} EMAIL more\n"),
                "expected an account ID and a time",
            ),
            (
                format!("{ANA} 2026-10-03T09:15:00Z not-an-id EMAIL\n"),
                "not an account ID",
            ),
            (
                "not-an-id 2026-10-03T09:15:00Z\n".to_owned(),
                "not an account ID",
            ),
            (format!("{ANA} 2026-10-03\n"), "not an RFC 3339 time"),
            (format!("{ANA} yesterday\n"), "not an RFC 3339 time"),
            // Half a line, as a file cut short would end.
            (
                format!("{}\n", &ANA[..20]),
                "expected an account ID and a time",
            ),
        ] {
            assert_eq!(
                parse(&format!("{good}{bad}")),
                Err(BadLine { number: 2, reason }),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_summary_says_what_was_done_and_whether_anything_is_left() {
        let mut summary = Summary {
            deleted: 2,
            already_deleted: 1,
            not_here: 3,
            ..Summary::default()
        };
        assert!(summary.complete());
        assert_eq!(
            summary.to_string(),
            "2 deleted again (0 of them suspended here, their suspension lifted), 1 already \
             deleted, 3 not in this database, 0 failed"
        );
        // A suspension lifted to delete the account leaves nothing undone.
        summary.lifted.push(ANA.parse().unwrap());
        assert!(summary.complete());
        assert!(
            summary
                .to_string()
                .starts_with("2 deleted again (1 of them suspended here")
        );
        summary.failed.push(BEN.parse().unwrap());
        assert!(!summary.complete());

        // A line the database contradicts leaves the replay incomplete too,
        // and says so.
        let contradicted = Summary {
            contradicted: vec![ANA.parse().unwrap()],
            ..Summary::default()
        };
        assert!(!contradicted.complete());
        assert!(
            contradicted
                .to_string()
                .ends_with(", 1 left alone because this database contradicts the log")
        );
    }
}
