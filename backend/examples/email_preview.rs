//! Writes every email the service sends, in every language, as files to open
//! in a browser: each one's HTML part, its plain text beside it, and an index
//! page linking them all. A development tool; nothing is sent.
//!
//! ```sh
//! cargo run --example email_preview -- <directory>
//! ```
//!
//! The values put in are made up: a display code, a one-time code, and links
//! under a web origin that is not served anywhere.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::{env, fs};

use yuppers_backend::auth::Purpose;
use yuppers_backend::domain::notification::Notice;
use yuppers_backend::languages;
use yuppers_backend::notifications::wording::{Links, Rendered, Wording};

const DISPLAY_CODE: &str = "K7QX-M2PD";
const ONE_TIME_CODE: &str = "482915";
const LINK: &str = "https://app.example.test/exchanges/0b2f6a3e-5d1c-4f8e-9a7b-3c2d1e0f9a8b";

fn main() -> anyhow::Result<()> {
    let Some(directory) = env::args_os().nth(1).map(PathBuf::from) else {
        anyhow::bail!("usage: cargo run --example email_preview -- <directory>");
    };
    let wording = Wording::embedded()?;
    let record = format!("{LINK}/record");
    let links = Links {
        exchange: LINK,
        record: &record,
    };

    let mut index = String::from(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>Email preview</title>\n</head>\n\
         <body style=\"font-family:system-ui,sans-serif;max-width:56rem;margin:2rem auto;padding:0 1rem;\">\n\
         <h1>Email preview</h1>\n",
    );
    for language in languages::supported() {
        let dir = directory.join(language);
        fs::create_dir_all(&dir)?;
        let _ = writeln!(index, "<h2>{language}</h2>\n<ul>");
        let mut write = |name: &str, email: Rendered| -> anyhow::Result<()> {
            fs::write(dir.join(format!("{name}.html")), &email.html)?;
            fs::write(
                dir.join(format!("{name}.txt")),
                format!("Subject: {}\n\n{}\n", email.subject, email.body),
            )?;
            let _ = writeln!(
                index,
                "<li><a href=\"{language}/{name}.html\">{name}</a> \
                 (<a href=\"{language}/{name}.txt\">text</a>)</li>"
            );
            Ok(())
        };
        for (name, purpose) in [
            ("code-sign-in", Purpose::SignIn),
            ("code-delete-account", Purpose::DeleteAccount),
        ] {
            write(name, wording.code_email(language, purpose, ONE_TIME_CODE))?;
        }
        for notice in Notice::ALL {
            write(
                notice.as_str(),
                wording.email(language, notice, DISPLAY_CODE, "Sam", links),
            )?;
        }
        index.push_str("</ul>\n");
    }
    index.push_str("</body>\n</html>\n");
    let path = directory.join("index.html");
    fs::write(&path, index)?;
    println!("{}", path.display());
    Ok(())
}
