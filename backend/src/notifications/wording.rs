//! The text of what the service sends (DESIGN.md §4.2).
//!
//! None of it is written here. It comes from the wording files that the web
//! and mobile apps also read, one per language, all of them embedded at build
//! time. A language added to those files is spoken here without a change to
//! this code.

use std::collections::HashMap;

use serde::Deserialize;

use super::html;
use crate::auth::Purpose;
use crate::combine::NoticeKind;
use crate::domain::notification::Notice;
use crate::languages;

// `WORDING_FILES`, written by the build script: every file in the wording
// directory, so that the set of languages is not spelled out in Rust.
include!(concat!(env!("OUT_DIR"), "/wording_files.rs"));

/// The parts of a wording file the service uses.
#[derive(Deserialize)]
struct File {
    #[serde(rename = "productName")]
    product_name: String,
    notifications: Notifications,
    /// The push notification's text (DESIGN.md §12): generic, the same for
    /// every notice, because a lock screen is read by whoever holds the
    /// phone.
    push: PushWording,
    /// The service's text messages: the agreement updates'.
    sms: SmsWording,
}

#[derive(Deserialize)]
struct PushWording {
    body: String,
}

/// The service's own texts, all of them "Yuppers.app agreement updates".
/// One-time codes are texted by Twilio Verify in its own template
/// (`super::verify`), so there is no wording for them here.
#[derive(Deserialize)]
struct SmsWording {
    /// An agreement update ("Yuppers.app agreement updates"): only that
    /// something changed, and the link. No terms, names, amounts or code.
    update: String,
    /// The confirmation texted when someone turns updates on.
    #[serde(rename = "optInConfirmation")]
    opt_in_confirmation: String,
}

#[derive(Deserialize)]
struct Notifications {
    email: EmailWording,
    /// The email that carries a one-time code, one message per purpose, so
    /// the message says what the code does.
    #[serde(rename = "oneTimeCode")]
    one_time_code: CodeWording,
    /// The email that tells a reviewer a report is waiting
    /// (`crate::review`). It says nothing about the report.
    #[serde(rename = "staffAlert")]
    staff_alert: Message,
    /// The email telling an address that its account was combined with
    /// another (`crate::combine`). It says nothing about any yup.
    #[serde(rename = "accountsCombined")]
    accounts_combined: Message,
    /// The email telling an address that it was replaced on its account.
    #[serde(rename = "emailChanged")]
    email_changed: Message,
    /// The email telling an address that it was removed from its account.
    #[serde(rename = "emailRemoved")]
    email_removed: Message,
}

#[derive(Deserialize)]
struct CodeWording {
    #[serde(rename = "signIn")]
    sign_in: Message,
    #[serde(rename = "deleteAccount")]
    delete_account: Message,
}

impl CodeWording {
    fn for_purpose(&self, purpose: Purpose) -> &Message {
        match purpose {
            Purpose::SignIn => &self.sign_in,
            Purpose::DeleteAccount => &self.delete_account,
        }
    }
}

#[derive(Deserialize)]
struct EmailWording {
    /// Wraps every message: where the text goes, the link, and why the
    /// reader is getting it.
    layout: String,
    /// Keyed by `Notice::as_str`.
    messages: HashMap<String, Message>,
}

#[derive(Deserialize)]
struct Message {
    subject: String,
    body: String,
    /// For a message that tells of a burst (`Notice::coalesces`): the same
    /// message about two or more items, with `{count}` in it. Absent for the
    /// rest, and the plain message is used.
    #[serde(default, rename = "subjectMany")]
    subject_many: Option<String>,
    #[serde(default, rename = "bodyMany")]
    body_many: Option<String>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum WordingError {
    #[error("the default language `{0}` has no usable notification wording")]
    NoDefault(String),
    #[error("the default language `{language}` has no wording for {notice}")]
    Missing {
        language: String,
        notice: &'static str,
    },
    #[error("the email layout of the default language `{0}` must contain {{body}} and {{link}}")]
    Layout(String),
}

/// Where a message sends its reader. Both are pages of the web app that ask
/// the reader to sign in; neither lets anyone in by itself (invariant 1).
#[derive(Clone, Copy, Debug)]
pub struct Links<'a> {
    /// The exchange. Every message ends with it.
    pub exchange: &'a str,
    /// The exchange's record, laid out to read, print or download. Given in
    /// the messages that tell a signer their agreement is on record.
    pub record: &'a str,
}

/// A message in one language, with its variables filled in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub subject: String,
    /// Plain text.
    pub body: String,
    /// The same message as an HTML document ([`super::html`]), made from the
    /// same wording and saying nothing the text does not.
    pub html: String,
}

/// Notification wording for every supported language that has it.
pub struct Wording {
    supported: &'static [String],
    default: &'static str,
    languages: HashMap<&'static str, File>,
}

impl Wording {
    /// The wording built into this binary. Fails if the default language
    /// cannot say everything, since it is what every other language falls
    /// back to.
    pub fn embedded() -> Result<Self, WordingError> {
        Self::from_files(languages::supported(), languages::default(), WORDING_FILES)
    }

    fn from_files(
        supported: &'static [String],
        default: &'static str,
        files: &[(&str, &str)],
    ) -> Result<Self, WordingError> {
        let mut languages: HashMap<&'static str, File> = supported
            .iter()
            .filter_map(|code| {
                let (_, text) = files.iter().find(|(name, _)| name == code)?;
                Some((code.as_str(), serde_json::from_str(text).ok()?))
            })
            .collect();
        // Without these a message would arrive empty, or with no way in.
        let whole = |file: &File| {
            let layout = &file.notifications.email.layout;
            layout.contains("{body}") && layout.contains("{link}")
        };

        let file = languages
            .get(default)
            .ok_or_else(|| WordingError::NoDefault(default.to_owned()))?;
        if !whole(file) {
            return Err(WordingError::Layout(default.to_owned()));
        }
        for notice in Notice::ALL {
            let messages = &file.notifications.email.messages;
            if !messages.contains_key(notice.as_str()) {
                return Err(WordingError::Missing {
                    language: default.to_owned(),
                    notice: notice.as_str(),
                });
            }
        }
        // Any other language whose file is missing or broken is left out, and
        // its readers get the default language. The wording check in the
        // TypeScript build is what refuses to ship one like that.
        languages.retain(|_, file| whole(file));

        Ok(Self {
            supported,
            default,
            languages,
        })
    }

    /// The email for a notice, in `language` (an account's preference) where
    /// that language has it and in the default language otherwise.
    ///
    /// `code` is the exchange's display code and `links` lead into it.
    /// Nothing else about the exchange can be put in: a message never carries
    /// what the parties agreed or wrote (DESIGN.md §12).
    pub fn email(&self, language: &str, notice: Notice, code: &str, links: Links<'_>) -> Rendered {
        self.email_about(language, notice, 1, code, links)
    }

    /// [`Wording::email`] for a message about `count` items: a burst of
    /// claims or confirmations is told in one message that says how many
    /// (DESIGN.md §12). A count of one, or a message with no wording for
    /// several, is the plain message.
    pub fn email_about(
        &self,
        language: &str,
        notice: Notice,
        count: u32,
        code: &str,
        links: Links<'_>,
    ) -> Rendered {
        let message_in = |language: &str| {
            let (language, file) = self.languages.get_key_value(language)?;
            let message = file.notifications.email.messages.get(notice.as_str())?;
            Some((*language, file, message))
        };
        let (language, file, message) = languages::resolve_among(self.supported, language)
            .and_then(message_in)
            .or_else(|| message_in(self.default))
            .expect("the default language has every notice; checked when loading");

        // The message about several, where it has one.
        let (subject_template, body_template) = match (
            count > 1,
            message.subject_many.as_deref(),
            message.body_many.as_deref(),
        ) {
            (true, Some(subject), Some(body)) => (subject, body),
            _ => (message.subject.as_str(), message.body.as_str()),
        };
        let count_text = count.to_string();
        let values = [
            ("productName", file.product_name.as_str()),
            ("code", code),
            ("link", links.exchange),
            ("recordLink", links.record),
            ("count", count_text.as_str()),
        ];
        let layout = &file.notifications.email.layout;
        let text = fill(body_template, &values);
        let mut with_text = values.to_vec();
        with_text.push(("body", &text));
        let subject = fill(subject_template, &values);
        let body = fill(layout, &with_text);

        // The HTML follows the layout paragraph by paragraph: the message
        // where `{body}` stands, a button where a paragraph ends in `{link}`
        // (its label is the text before the link), and whatever comes after
        // that as small print.
        let markup = [
            ("productName", Value::Text(&file.product_name)),
            ("code", Value::Text(code)),
            ("link", Value::Link(links.exchange)),
            ("recordLink", Value::Link(links.record)),
            ("count", Value::Text(&count_text)),
            ("body", Value::Text(&text)),
        ];
        let (mut main, mut small_print, mut past_link) = (String::new(), String::new(), false);
        for part in layout.split("\n\n") {
            if part.trim() == "{body}" {
                for paragraph in body_template.split("\n\n") {
                    main.push_str(&html::paragraph(&fill_html(paragraph, &markup)));
                }
                continue;
            }
            let label = part
                .trim_end()
                .strip_suffix("{link}")
                .map(|before| {
                    let label = fill(before, &values);
                    label
                        .trim_end_matches(|c: char| {
                            c.is_whitespace() || matches!(c, ':' | '：' | '-' | '–' | '—')
                        })
                        .to_owned()
                })
                .filter(|label| !label.is_empty());
            if let Some(label) = label {
                main.push_str(&html::button(&label, links.exchange));
                past_link = true;
            } else if past_link {
                small_print.push_str(&html::small_print(&fill_html(part, &markup)));
            } else {
                main.push_str(&html::paragraph(&fill_html(part, &markup)));
            }
        }
        let html = html::page(&html::Page {
            language,
            direction: languages::direction(language),
            product: &file.product_name,
            heading: &subject,
            main: &main,
            small_print: &small_print,
        });
        Rendered {
            subject,
            body,
            html,
        }
    }

    /// The file for `language` where it has wording, and the default
    /// language's otherwise.
    fn file_for(&self, language: &str) -> &File {
        languages::resolve_among(self.supported, language)
            .and_then(|language| self.languages.get(language))
            .or_else(|| self.languages.get(self.default))
            .expect("the default language has wording; checked when loading")
    }

    /// The text of a push notification, in `language` where that language
    /// has wording and in the default language otherwise. The same for
    /// every notice and every exchange: it names no exchange, no party and
    /// nothing agreed, since anyone holding the phone can read a lock screen
    /// (DESIGN.md §12). Opening it shows the rest, after signing in.
    pub fn push(&self, language: &str) -> String {
        let file = self.file_for(language);
        fill(
            &file.push.body,
            &[("productName", file.product_name.as_str())],
        )
    }

    /// The text message telling someone who turned on text updates for an
    /// agreement that its status changed, with `link` to the exchange
    /// (`crate::notifications::sms_updates`). The same for every change and
    /// every agreement: it names no term, amount, person or code, as the
    /// terms promise.
    pub fn update_sms(&self, language: &str, link: &str) -> String {
        let file = self.file_for(language);
        fill(&file.sms.update, &[("link", link)])
    }

    /// The text message confirming that text updates were turned on for an
    /// agreement, as the carriers ask: the program, how often, that rates
    /// may apply, and HELP and STOP.
    pub fn opt_in_sms(&self, language: &str) -> String {
        self.file_for(language).sms.opt_in_confirmation.clone()
    }

    /// The email that carries a one-time code, in `language` where that
    /// language has wording and in the default language otherwise. The
    /// message says what the code is for (`crate::auth::Purpose`). It is not
    /// wrapped in the notification layout: there is no exchange to link to.
    pub fn code_email(&self, language: &str, purpose: Purpose, code: &str) -> Rendered {
        let (language, file) = languages::resolve_among(self.supported, language)
            .and_then(|language| self.languages.get_key_value(language))
            .or_else(|| self.languages.get_key_value(self.default))
            .expect("the default language has wording; checked when loading");
        let message = file.notifications.one_time_code.for_purpose(purpose);
        let values = [("productName", file.product_name.as_str()), ("code", code)];
        let subject = fill(&message.subject, &values);

        // The text's paragraphs, with the code shown large under the first
        // one that gives it: the instruction to enter it. The warnings after
        // it stay part of the message, not small print.
        let markup = [
            ("productName", Value::Text(&file.product_name)),
            ("code", Value::Strong(code)),
        ];
        let mut main = String::new();
        let mut shown = false;
        for paragraph in message.body.split("\n\n") {
            main.push_str(&html::paragraph(&fill_html(paragraph, &markup)));
            if !shown && paragraph.contains("{code}") {
                main.push_str(&html::code(code));
                shown = true;
            }
        }
        let html = html::page(&html::Page {
            language,
            direction: languages::direction(language),
            product: &file.product_name,
            heading: &subject,
            main: &main,
            small_print: "",
        });
        Rendered {
            subject,
            body: fill(&message.body, &values),
            html,
        }
    }
}

impl Wording {
    /// The email that tells a reviewer a report is waiting, in `language`
    /// where that language has wording and in the default language
    /// otherwise. It names no report, exchange or person, and links to the
    /// review screen, which asks the reader to sign in.
    pub fn staff_alert(&self, language: &str, link: &str) -> Rendered {
        self.linked(language, link, |file| &file.notifications.staff_alert)
    }

    /// The email telling an address that the account it was on was combined
    /// with another (`crate::combine`), in `language` where that language
    /// has wording and in the default language otherwise. It names no yup,
    /// person or address, and links to the account page, which asks the
    /// reader to sign in.
    pub fn accounts_combined(&self, language: &str, link: &str) -> Rendered {
        self.linked(language, link, |file| &file.notifications.accounts_combined)
    }

    /// The email to an address about its account (`crate::combine`), as
    /// [`Self::accounts_combined`]: combined, replaced or removed.
    pub fn account_notice(&self, kind: NoticeKind, language: &str, link: &str) -> Rendered {
        match kind {
            NoticeKind::AccountsCombined => self.accounts_combined(language, link),
            NoticeKind::EmailChanged => {
                self.linked(language, link, |file| &file.notifications.email_changed)
            }
            NoticeKind::EmailRemoved => {
                self.linked(language, link, |file| &file.notifications.email_removed)
            }
        }
    }

    /// An email with no exchange behind it, whose paragraph ending in
    /// `{link}` becomes a button.
    fn linked(&self, language: &str, link: &str, message: fn(&File) -> &Message) -> Rendered {
        let (language, file) = languages::resolve_among(self.supported, language)
            .and_then(|language| self.languages.get_key_value(language))
            .or_else(|| self.languages.get_key_value(self.default))
            .expect("the default language has wording; checked when loading");
        let message = message(file);
        let values = [("productName", file.product_name.as_str()), ("link", link)];
        let subject = fill(&message.subject, &values);

        // A paragraph that ends in the link becomes a button, as in the
        // notification layout; what follows it is small print.
        let markup = [
            ("productName", Value::Text(&file.product_name)),
            ("link", Value::Link(link)),
        ];
        let (mut main, mut small_print, mut past_link) = (String::new(), String::new(), false);
        for part in message.body.split("\n\n") {
            let label = part
                .trim_end()
                .strip_suffix("{link}")
                .map(|before| {
                    fill(before, &values)
                        .trim_end_matches(|c: char| {
                            c.is_whitespace() || matches!(c, ':' | '：' | '-' | '–' | '—')
                        })
                        .to_owned()
                })
                .filter(|label| !label.is_empty());
            if let Some(label) = label {
                main.push_str(&html::button(&label, link));
                past_link = true;
            } else if past_link {
                small_print.push_str(&html::small_print(&fill_html(part, &markup)));
            } else {
                main.push_str(&html::paragraph(&fill_html(part, &markup)));
            }
        }
        let html = html::page(&html::Page {
            language,
            direction: languages::direction(language),
            product: &file.product_name,
            heading: &subject,
            main: &main,
            small_print: &small_print,
        });
        Rendered {
            subject,
            body: fill(&message.body, &values),
            html,
        }
    }
}

/// A piece of a template: text as written, or a variable that has a value.
enum Piece<'t, V> {
    Text(&'t str),
    Value(V),
}

/// Splits `template` at each `{name}` that has a value. What is put in is
/// not read again, so a value containing braces stays as it is.
fn pieces<'t, V: Copy>(template: &'t str, values: &[(&str, V)]) -> Vec<Piece<'t, V>> {
    let mut out = Vec::new();
    let mut rest = template;
    let mut text_from = 0;
    let mut at = 0;
    while let Some(start) = rest[at..].find('{').map(|start| at + start) {
        let after = &rest[start + 1..];
        let value = after.find('}').and_then(|end| {
            let (_, value) = values.iter().find(|(name, _)| *name == &after[..end])?;
            Some((end, *value))
        });
        match value {
            Some((end, value)) => {
                out.push(Piece::Text(&rest[text_from..start]));
                out.push(Piece::Value(value));
                rest = &after[end + 1..];
                text_from = 0;
                at = 0;
            }
            None => at = start + 1,
        }
    }
    out.push(Piece::Text(&rest[text_from..]));
    out
}

/// Replaces each `{name}` that has a value.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    pieces(template, values)
        .into_iter()
        .map(|piece| match piece {
            Piece::Text(text) | Piece::Value(text) => text,
        })
        .collect()
}

/// A value put into HTML.
#[derive(Clone, Copy)]
enum Value<'a> {
    /// Text, escaped.
    Text(&'a str),
    /// A web address, as a link.
    Link(&'a str),
    /// Text set apart, such as a one-time code.
    Strong(&'a str),
}

/// [`fill`] for HTML: the template's own text and every value escaped, and a
/// line break kept as one.
fn fill_html(template: &str, values: &[(&str, Value<'_>)]) -> String {
    let text = |text: &str| html::escape(text).replace('\n', "<br>\n");
    pieces(template, values)
        .into_iter()
        .map(|piece| match piece {
            Piece::Text(written) => text(written),
            Piece::Value(Value::Text(value)) => text(value),
            Piece::Value(Value::Link(url)) => html::link(url),
            Piece::Value(Value::Strong(value)) => html::strong(value),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;

    const CODE: &str = "AB12-CD34";
    const LINK: &str = "https://app.test/exchanges/7";
    const RECORD: &str = "https://app.test/exchanges/7/record";
    const LINKS: Links<'static> = Links {
        exchange: LINK,
        record: RECORD,
    };

    #[test]
    fn every_supported_language_has_every_message_with_nothing_left_unfilled() {
        let wording = Wording::embedded().unwrap();
        for language in languages::supported() {
            let file = wording
                .languages
                .get(language.as_str())
                .unwrap_or_else(|| panic!("{language} has no notification wording"));
            for notice in Notice::ALL {
                assert!(
                    file.notifications
                        .email
                        .messages
                        .contains_key(notice.as_str()),
                    "{language} has no wording for {}",
                    notice.as_str()
                );
                let email = wording.email(language, notice, CODE, LINKS);
                for text in [&email.subject, &email.body] {
                    assert!(
                        !text.contains('{') && !text.contains('}'),
                        "{language} {}: an unknown variable in {text:?}",
                        notice.as_str()
                    );
                }
                assert!(!email.subject.trim().is_empty());
                assert!(email.body.contains(LINK), "every message links back");
                // An agreement coming into force is when each signer is told
                // where their copy is (DESIGN.md §14.1).
                let copy = matches!(notice, Notice::AgreementInForce | Notice::AmendmentInForce);
                assert_eq!(
                    email.body.contains(RECORD),
                    copy,
                    "{language} {}: the link to the record",
                    notice.as_str()
                );
            }
            // A wording file with a message the service never sends is a
            // translation nobody will read.
            for name in file.notifications.email.messages.keys() {
                assert!(Notice::parse(name).is_some(), "{language}: stray {name}");
            }
        }
    }

    #[test]
    fn a_message_is_in_the_language_asked_for() {
        let wording = Wording::embedded().unwrap();
        let english = wording.email("en", Notice::DeliveryClaimed, CODE, LINKS);
        let spanish = wording.email("es", Notice::DeliveryClaimed, CODE, LINKS);
        assert_ne!(english, spanish);
        assert!(english.subject.contains(CODE));
        // A regional tag gets its base language.
        assert_eq!(
            wording.email("es-MX", Notice::DeliveryClaimed, CODE, LINKS),
            spanish
        );
    }

    #[test]
    fn a_language_without_wording_falls_back_to_the_default() {
        let wording = Wording::embedded().unwrap();
        let default = wording.email(languages::default(), Notice::DisputeOpened, CODE, LINKS);
        assert_eq!(
            wording.email("tlh", Notice::DisputeOpened, CODE, LINKS),
            default
        );
        assert_eq!(
            wording.email("", Notice::DisputeOpened, CODE, LINKS),
            default
        );
    }

    #[test]
    fn a_code_email_names_the_code_and_what_it_is_for_in_the_language_asked_for() {
        let wording = Wording::embedded().unwrap();
        for language in languages::supported() {
            let sign_in = wording.code_email(language, Purpose::SignIn, "123456");
            let delete = wording.code_email(language, Purpose::DeleteAccount, "123456");
            assert_ne!(
                sign_in, delete,
                "{language}: the two purposes read differently"
            );
            for email in [&sign_in, &delete] {
                assert!(email.subject.contains("123456"), "{language}: {email:?}");
                assert!(email.body.contains("123456"), "{language}: {email:?}");
                for text in [&email.subject, &email.body] {
                    assert!(
                        !text.contains('{') && !text.contains('}'),
                        "{language}: {text:?}"
                    );
                }
            }
        }
        assert_ne!(
            wording.code_email("en", Purpose::SignIn, "123456"),
            wording.code_email("es", Purpose::SignIn, "123456")
        );
        assert_eq!(
            wording.code_email("tlh", Purpose::SignIn, "123456"),
            wording.code_email(languages::default(), Purpose::SignIn, "123456")
        );
    }

    fn file(product: &str, messages: &[Notice]) -> String {
        let messages: serde_json::Map<String, serde_json::Value> = messages
            .iter()
            .map(|notice| {
                let message = serde_json::json!({
                    "subject": format!("{product} {} {{code}}", notice.as_str()),
                    "body": "Text.",
                });
                (notice.as_str().to_owned(), message)
            })
            .collect();
        let code = |what: &str| serde_json::json!({ "subject": format!("{product} {what} {{code}}"), "body": "{code}" });
        serde_json::json!({
            "productName": product,
            "notifications": {
                "email": { "layout": "{body} {link}", "messages": messages },
                "oneTimeCode": { "signIn": code("sign-in"), "deleteAccount": code("delete") },
                "staffAlert": { "subject": format!("{product} review"), "body": "Waiting.\n\nOpen: {link}" },
                "accountsCombined": { "subject": format!("{product} combined"), "body": "Combined.\n\nOpen: {link}" },
                "emailChanged": { "subject": format!("{product} changed"), "body": "Changed.\n\nOpen: {link}" },
                "emailRemoved": { "subject": format!("{product} removed"), "body": "Removed.\n\nOpen: {link}" },
            },
            "push": { "body": format!("{product} news") },
            "sms": {
                "signIn": "{code} in",
                "deleteAccount": "{code} out",
                "verifyNumber": "{code} added",
                "update": "changed: {link}",
                "optInConfirmation": "on",
            },
        })
        .to_string()
    }

    static THREE: LazyLock<Vec<String>> =
        LazyLock::new(|| vec!["en".to_owned(), "fr".to_owned(), "de".to_owned()]);

    #[test]
    fn a_language_is_picked_up_from_its_file_alone() {
        let english = file("Yuppers", &Notice::ALL);
        let french = file("Échange", &Notice::ALL);
        let wording =
            Wording::from_files(&THREE, "en", &[("en", &english), ("fr", &french)]).unwrap();

        let email = wording.email("fr", Notice::EndProposed, CODE, LINKS);
        assert_eq!(email.subject, format!("Échange END_PROPOSED {CODE}"));
        assert_eq!(email.body, format!("Text. {LINK}"));
        // Listed, but with no file yet.
        assert_eq!(
            wording
                .email("de", Notice::EndProposed, CODE, LINKS)
                .subject,
            format!("Yuppers END_PROPOSED {CODE}")
        );
    }

    #[test]
    fn a_message_one_language_lacks_is_sent_whole_in_the_default_language() {
        let english = file("Yuppers", &Notice::ALL);
        let french = file("Échange", &[Notice::EndProposed]);
        let wording =
            Wording::from_files(&THREE, "en", &[("en", &english), ("fr", &french)]).unwrap();

        assert_eq!(
            wording
                .email("fr", Notice::CloseRequested, CODE, LINKS)
                .subject,
            format!("Yuppers CLOSE_REQUESTED {CODE}"),
            "the product name is the default language's too, not a mixture"
        );
    }

    #[test]
    fn an_incomplete_default_language_is_refused() {
        let english = file("Yuppers", &[Notice::EndProposed]);
        assert_eq!(
            Wording::from_files(&THREE, "en", &[("en", &english)]).err(),
            Some(WordingError::Missing {
                language: "en".to_owned(),
                notice: Notice::ALL[0].as_str(),
            })
        );
        assert_eq!(
            Wording::from_files(&THREE, "en", &[]).err(),
            Some(WordingError::NoDefault("en".to_owned()))
        );

        let no_link = file("Yuppers", &Notice::ALL).replace("{link}", "");
        assert_eq!(
            Wording::from_files(&THREE, "en", &[("en", &no_link)]).err(),
            Some(WordingError::Layout("en".to_owned()))
        );
    }

    #[test]
    fn filling_replaces_known_variables_once() {
        let values = [("code", "{link}"), ("link", "L")];
        assert_eq!(fill("a {code} b {link}", &values), "a {link} b L");
        assert_eq!(fill("{unknown} {code", &values), "{unknown} {code");
        assert_eq!(fill("", &values), "");
        assert_eq!(fill("{{code}}", &values), "{{link}}");
    }

    #[test]
    fn filling_html_escapes_the_template_and_every_value() {
        let values = [
            ("code", Value::Text("<b>\"x\"</b>")),
            ("link", Value::Link("https://app.test/a?b=1&c='2'")),
            ("one", Value::Strong("1<2")),
        ];
        assert_eq!(
            fill_html("a & {code}\n{one} {missing}", &values),
            "a &amp; &lt;b&gt;&quot;x&quot;&lt;/b&gt;<br>\n<strong dir=\"ltr\">1&lt;2</strong> {missing}"
        );
        let link = fill_html("{link}", &values);
        assert!(
            link.contains("href=\"https://app.test/a?b=1&amp;c=&#39;2&#39;\""),
            "{link}"
        );
    }

    /// What a reader sees of an HTML document, as words: no style sheet, no
    /// conditional comments, no tags, entities read back.
    fn seen_words(html: &str) -> Vec<String> {
        fn without(text: &str, open: &str, close: &str) -> String {
            let mut kept = String::new();
            let mut rest = text;
            while let Some(start) = rest.find(open) {
                kept.push_str(&rest[..start]);
                let end = rest[start..].find(close).expect("closed") + start;
                rest = &rest[end + close.len()..];
            }
            kept.push_str(rest);
            kept
        }
        let text = without(&without(html, "<style>", "</style>"), "<!--", "-->");
        let mut seen = String::new();
        let mut in_tag = false;
        for c in text.chars() {
            match c {
                '<' => in_tag = true,
                '>' if in_tag => {
                    in_tag = false;
                    seen.push(' ');
                }
                c if !in_tag => seen.push(c),
                _ => {}
            }
        }
        let seen = seen
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&");
        words(&seen)
    }

    fn words(text: &str) -> Vec<String> {
        text.split_whitespace()
            .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric()).to_owned())
            .filter(|word| !word.is_empty())
            .collect()
    }

    /// Checks one email's HTML against its text.
    fn check_html(language: &str, what: &str, email: &Rendered) {
        let html = &email.html;
        assert!(
            html.starts_with("<!DOCTYPE html>")
                && html.contains(&format!("<html lang=\"{language}\" dir=\"ltr\">")),
            "{language} {what}: the document's language"
        );
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<meta name=\"color-scheme\" content=\"light dark\">"));
        for banned in [
            "<script", "<img", "<link", "@import", "url(", "<form", "<iframe",
        ] {
            assert!(!html.contains(banned), "{language} {what}: {banned}");
        }
        // Every link goes into the web app, and nothing else is addressed.
        for (at, _) in html.match_indices("href=\"") {
            let target = &html[at + 6..];
            assert!(
                target.starts_with("https://app.test/exchanges/7")
                    || target.starts_with("https://app.test/staff\"")
                    || target.starts_with("https://app.test/account\""),
                "{language} {what}: a link to {}",
                &target[..target.find('"').unwrap()]
            );
        }
        assert_eq!(
            html.matches("https://").count(),
            html.matches("https://app.test/").count(),
            "{language} {what}: an address outside the web origin"
        );

        let seen = seen_words(html);
        assert!(
            !seen
                .iter()
                .any(|word| word.contains('{') || word.contains('}')),
            "{language} {what}: an unfilled variable in {seen:?}"
        );
        // Nothing the text and the subject do not say, besides the mark's
        // letter.
        let said: std::collections::HashSet<String> = words(&email.subject)
            .into_iter()
            .chain(words(&email.body))
            .collect();
        for word in &seen {
            assert!(
                said.contains(word) || word == html::MARK_LETTER,
                "{language} {what}: the HTML says {word:?}, which the text does not"
            );
        }
        // And all of the text is there.
        let seen: std::collections::HashSet<&String> = seen.iter().collect();
        for word in words(&email.body) {
            assert!(
                seen.contains(&word),
                "{language} {what}: {word:?} is missing from the HTML"
            );
        }
    }

    #[test]
    fn every_email_has_an_html_part_in_every_language_saying_what_the_text_says() {
        let wording = Wording::embedded().unwrap();
        for language in languages::supported() {
            for notice in Notice::ALL {
                let email = wording.email(language, notice, CODE, LINKS);
                check_html(language, notice.as_str(), &email);
                // The way in is a button, with the address beneath it.
                assert!(
                    email.html.contains(&format!("<a href=\"{LINK}\"")),
                    "{language} {}: the button",
                    notice.as_str()
                );
                assert!(email.html.contains(&format!(">{LINK}</a>")));
                // Under the message, why the reader gets it.
                assert!(email.html.contains("class=\"y-muted\" dir=\"ltr\""));
            }
            for purpose in [Purpose::SignIn, Purpose::DeleteAccount] {
                let email = wording.code_email(language, purpose, "123456");
                check_html(language, purpose.as_str(), &email);
                assert!(
                    email.html.contains("user-select:all;\">123456</span>"),
                    "{language} {}: the code, large and easy to select",
                    purpose.as_str()
                );
                assert!(!email.html.contains("href"), "no link in a code email");
            }
        }
    }

    #[test]
    fn the_staff_alert_says_only_that_a_report_waits_in_every_language() {
        let wording = Wording::embedded().unwrap();
        let link = "https://app.test/staff";
        for language in languages::supported() {
            let email = wording.staff_alert(language, link);
            check_html(language, "staff alert", &email);
            assert!(email.body.contains(link), "{language}: the link");
            assert!(
                email.html.contains(&format!("<a href=\"{link}\"")),
                "{language}: the button"
            );
            assert!(
                !email.body.contains('{'),
                "{language}: every variable filled"
            );
        }
        assert_eq!(
            wording.staff_alert("en", link).subject,
            "A report is waiting for review"
        );
    }

    #[test]
    fn the_accounts_combined_notice_says_only_that_in_every_language() {
        let wording = Wording::embedded().unwrap();
        let link = "https://app.test/account";
        for language in languages::supported() {
            for kind in [
                NoticeKind::AccountsCombined,
                NoticeKind::EmailChanged,
                NoticeKind::EmailRemoved,
            ] {
                let email = wording.account_notice(kind, language, link);
                check_html(language, "account notice", &email);
                assert!(email.body.contains(link), "{language} {kind:?}: the link");
                assert!(
                    email.html.contains(&format!("<a href=\"{link}\"")),
                    "{language} {kind:?}: the button"
                );
                assert!(
                    !email.body.contains('{') && !email.subject.contains('{'),
                    "{language} {kind:?}: every variable"
                );
            }
        }
        assert_eq!(
            wording
                .account_notice(NoticeKind::EmailChanged, "en", link)
                .subject,
            "Your Yuppers email address was changed"
        );
        assert_eq!(
            wording.accounts_combined("en", link).subject,
            "Two of your Yuppers accounts were combined"
        );
    }

    #[test]
    fn the_button_is_labelled_by_the_layout_in_the_readers_language() {
        let wording = Wording::embedded().unwrap();
        let english = wording.email("en", Notice::RevisionSent, CODE, LINKS);
        let spanish = wording.email("es", Notice::RevisionSent, CODE, LINKS);
        assert!(
            english.html.contains(">Open the yup</a>"),
            "{}",
            english.html
        );
        assert!(
            spanish.html.contains(">Abre el yup</a>"),
            "{}",
            spanish.html
        );
    }

    #[test]
    fn the_text_part_is_what_it_always_was() {
        let wording = Wording::embedded().unwrap();
        assert_eq!(
            wording
                .email("en", Notice::DeliveryClaimed, CODE, LINKS)
                .body,
            format!(
                "The other party has marked one of their contributions as delivered. \
                 Review it, then confirm it or dispute it.\n\n\
                 Open the yup: {LINK}\n\n\
                 You’re getting this email because you’re part of yup {CODE} on Yuppers. \
                 These emails never include the terms; sign in to see them."
            )
        );
        assert_eq!(
            wording.code_email("en", Purpose::SignIn, "123456").body,
            "Enter 123456 to sign in to Yuppers. The code works once and expires in a few \
             minutes.\n\nIf you didn’t ask to sign in, ignore this email: nothing happens \
             without the code, and nobody from Yuppers will ever ask you for it."
        );
    }

    #[test]
    fn markup_in_a_value_is_shown_as_text_and_never_run() {
        let wording = Wording::embedded().unwrap();
        let hostile = r#"<script>alert("x")</script>'"#;
        let escaped = "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;&#39;";
        let link = "https://app.test/exchanges/7\"><script>alert('y')</script>";
        let record = format!("{link}/record");
        let links = Links {
            exchange: link,
            record: &record,
        };
        for language in languages::supported() {
            for notice in Notice::ALL {
                let html = wording.email(language, notice, hostile, links).html;
                assert!(!html.contains("<script"), "{html}");
                assert!(html.contains(escaped), "{html}");
                assert!(
                    html.contains(
                        "https://app.test/exchanges/7&quot;&gt;&lt;script&gt;alert(&#39;y&#39;)"
                    ),
                    "{html}"
                );
            }
            for purpose in [Purpose::SignIn, Purpose::DeleteAccount] {
                let html = wording.code_email(language, purpose, hostile).html;
                assert!(!html.contains("<script"), "{html}");
                assert!(html.contains(escaped), "{html}");
            }
        }
    }

    #[test]
    fn a_layout_of_another_shape_still_makes_a_whole_page() {
        // No paragraph ends in the link: it is shown in its sentence.
        let english = file("Yuppers", &Notice::ALL);
        let wording = Wording::from_files(
            &THREE,
            "en",
            &[(
                "en",
                &english.replace("{body} {link}", "{body}\\n\\nGo to {link} now."),
            )],
        )
        .unwrap();
        let email = wording.email("en", Notice::EndProposed, CODE, LINKS);
        assert_eq!(email.body, format!("Text.\n\nGo to {LINK} now."));
        assert!(
            email.html.contains("Go to <a class=\"y-link\""),
            "{}",
            email.html
        );
        assert!(
            !email
                .html
                .contains("display:inline-block;padding:12px 24px")
        );
    }
}
