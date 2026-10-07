//! No email address or phone number anywhere in the database (migration
//! 0025; README, "Contact details at rest"), in a database of its own: the
//! whole life of two people through the API, then every column of every
//! table searched for every address and number they used, as text and as
//! raw bytes.

mod common;

use common::texting::{found_anywhere, number, texting};

const DATABASE: &str = "yuppers_test_contact_scan";

#[tokio::test]
async fn no_email_address_or_phone_number_is_stored_anywhere() {
    let test = texting(DATABASE).await;
    let app = &test.app;

    // The search finds a number wherever it is, as raw bytes too: here, put
    // where nothing of the service's would put it.
    let control = number();
    let mut subject = control.as_bytes().to_vec();
    subject.resize(32, 0);
    sqlx::query(
        "INSERT INTO sign_in_limit (scope, subject, window_start, count)
         VALUES ('sms-sent', $1, now(), 1)",
    )
    .bind(&subject)
    .execute(&app.owner)
    .await
    .unwrap();
    let found = found_anywhere(&app.owner, std::slice::from_ref(&control)).await;
    assert!(
        found
            .iter()
            .any(|place| place.starts_with("sign_in_limit.subject")),
        "{found:?}"
    );
    sqlx::query("DELETE FROM sign_in_limit WHERE subject = $1")
        .bind(&subject)
        .execute(&app.owner)
        .await
        .unwrap();

    // A whole life through the API, twice over: signing in by address and
    // by number, adding one, invitations bound to each, codes, text updates
    // on and off, STOP and START, deletion.
    let mut needles = test.whole_life().await;
    needles.extend(test.whole_life().await);
    assert_eq!(
        found_anywhere(&app.owner, &needles).await,
        Vec::<String>::new()
    );
}
