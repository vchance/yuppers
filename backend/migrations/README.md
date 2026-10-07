# Migrations

SQL files named `<version>_<description>.sql`, embedded into the binaries at build time and applied by `cargo run --bin migrate`.

- Migrations run as the schema owner (`MIGRATION_DATABASE_URL`). The `api` and `worker` processes connect as the restricted application role `exchange_app` and never run them.
- The role `exchange_app` must exist before migrating; migrations grant to it and fail if it is missing.
- Every new table needs its own explicit `GRANT` to `exchange_app`. Nothing is granted by default, so a table is unreachable by the service until a migration says what the service may do with it.
- A migration that has been applied anywhere is never edited. Add a new one.
- So the role names stay as the migrations wrote them, from before the product was called Yuppers: `exchange_app` for the service, and by convention `exchange` for the owner. A role belongs to the whole PostgreSQL cluster, so a migration cannot rename it either: every other database on the cluster still has to apply `0001` and grant to `exchange_app`. Likewise the `exchange` table and the other names in the schema are the technical model, not the brand.

## 0001_record_model

The record model of `DESIGN.md` §10 with the rules of §13.2.

**Append-only tables:** `revision`, `revision_attachment`, `contribution_snapshot`, `acceptance`, `exchange_event`. Two independent protections:

- the application role is granted only `SELECT` and `INSERT` on them;
- a trigger refuses `UPDATE`, `DELETE` and `TRUNCATE` for every role, the schema owner included.

Erasure and retention deletes (`DESIGN.md` §14) are still open with counsel. Until that is decided nothing can remove history; the migration that implements it adds the one audited path allowed to.

**Other rules the database enforces:**

- An acceptance must carry the content hash of the revision it signs, and the signer must be the account holding that slot.
- Every reference stays inside one exchange (composite foreign keys on `exchange_id`).
- Event sequence numbers are unique per exchange.
- At most one claimable invitation per exchange.
- Amount, currency and settlement mode appear on money contributions only, in the exchange's currency.
- A contribution can wait only on another contribution in the same revision, never on itself. Longer cycles are the service's job to reject.
- An exchange's state, closed outcome and revision pointers must agree.

`backend/tests/schema.rs` checks each of these against a real database.

## 0002_auth

One-time codes and sessions (`DESIGN.md` §8). A code is stored as a keyed hash (HMAC under `APP_SECRET`), because six digits are too few to protect with a plain hash; a session token is 256 random bits and stored as a SHA-256 hash. `backend/tests/auth.rs` exercises both through the HTTP endpoints.

## 0003_exchange_projection

What the exchange API needed beyond the first model. Each revision now records both party names as written, since they are part of what is signed. The exchange row gains the state the rules track between commands: a pending end proposal or close request, the inactivity prompt, and last activity. Indexes support the worker's timers.

`backend/tests/exchanges.rs` and `backend/tests/timers.rs` drive the API against databases of their own, created fresh on each run, because agreement history cannot be deleted and so cannot be cleaned up.

## 0004_any_language

The first migration allowed only `en` and `es` as an account's language and as a signer's consent language. Which languages exist is decided by the wording files (`DESIGN.md` §4.2), and adding one must not need a migration, so the database now checks only that the value is shaped like a language tag.

## 0005_text_bounds

Upper bounds on free text, as backstops. Agreement history cannot be deleted, so nothing written into it may be unbounded. The service enforces the real, tighter limits (`domain::Limits`), which can change without a migration.

## 0006_slot_holdings

Someone who opened an invitation that named nobody can be removed, or can leave, before the initiator confirms them (`DESIGN.md` §8). The invited party's slot is then free for someone else, so `participant.account_id` is no longer set once and for all, and a signature can no longer be tied to it: the signature is permanent and the row is not.

**`slot_holding`** has one row for each time an account held a slot: who, from when, and until when if it ended. A signature (`acceptance.holding`) now belongs to a holding.

- The database writes the table itself, from changes to `participant.account_id`. The application role can read it and nothing else; no role can change a holding except to end it once, by emptying the slot.
- Only the invited party's slot can be emptied, and only while the initiator has not confirmed them. A slot is emptied before someone else takes it. A confirmation is never taken back.
- A signature is stamped by the database with the holding open in its slot when it is inserted, and must come from that holding's account. That is the old rule, "the signer is the account holding that slot", made to hold for the past as well.
- One signature per revision, slot and holding: whoever takes a slot after someone was removed from it signs the same revision in their own right.
- A revision comes into force only when both slots' current holders have signed it and the invited party is confirmed. A signature made under a holding that has ended counts for nothing, whoever tries to rely on it.

Nothing is deleted: a removed person's claim event, their signature and their holding all stay, under their account.

**Rejected.** Dropping the foreign key and checking the signer in a trigger alone would have left no record of who held the slot when. Letting several participant rows share a slot would have changed what every other table's reference to a slot means. A "void" flag on the signature would have meant updating an append-only table. Starting a fresh exchange for the next claimant would have changed the exchange ID, which is part of what the initiator signed.

`backend/tests/schema.rs` checks these against the database alone, and `backend/tests/claimant.rs` through the API.

## 0007_reminders

`contribution_reminder`: what each contribution has already been reminded about (`DESIGN.md` §12). A "due soon" or "overdue" reminder is not caused by an event, so the outbox's key of one message per event per person cannot stop it repeating; a row here, written in the same transaction that queues the emails, is what does. The key is the contribution, the kind of reminder and the due date it was about, so an amendment that moves the date makes the new date something not yet reminded about, and one that leaves it alone brings no second reminder.

- The application role may read and add rows and nothing else: a row that could be changed or removed is a reminder that could go out twice.
- It is not agreement history, so no trigger protects it from the schema owner. It does hold a date taken from the agreement, so the erasure and retention path, when it exists, has to remove these rows with the exchange they belong to.
- A reminder writes nothing to `exchange` or `exchange_event`: the version, the history and the last activity stay as they were.

The index on `exchange (timezone)` for active exchanges is for the worker, which reads the date in each timezone that has an active exchange and then looks through that timezone's exchanges.

`backend/tests/schema.rs` checks the key and the grants; `backend/tests/reminders.rs` drives the worker's pass against a database of its own.

## 0008_sign_in_limits

Sign-in hardening (`DESIGN.md` §8, §18 item 4). A new code no longer ends the live ones, so `one_time_code` gains `purpose`: the codes kept live, checked and spent together are one identifier's for one purpose. Codes stored before it are taken as sign-in codes, so a deletion code in flight during the upgrade stops working, which is the safe way round.

**`sign_in_limit`** counts code requests per requester's address or per account, and failed guesses per identifier or per account, one row per thing counted and fixed window (an hour, or a UTC day). The service locks the row while it decides, so concurrent requests cannot both slip under a limit. Whom a row counts is a keyed hash under `APP_SECRET`, never an address or identifier in the clear, and the worker removes windows more than two days old; so deleting an account need not touch the table. The application role may read, add, change and remove rows. `backend/tests/schema.rs` checks the constraints and grants; `backend/tests/auth.rs` and `backend/tests/deletion.rs` the limits through the API. The scope `failed-guesses-by-address` is allowed by the constraint but no longer written: wrong guesses stopped being counted by address (README, "Signing in"), and its old rows go with the worker's purge.

## 0009_load_indexes

Three indexes on `exchange`, for queries the load check (README, "Load check") found reading the whole table, each run often enough that its cost would have grown with every exchange ever made:

- `(created_by, created_at)`: counting an account's exchanges in the last day, on every creation, under the per-account limit.
- `(open_revision_id)` where there is one: the worker's search for expired revisions, on every pass. Without it the search read every revision whose expiry had passed, which in time is nearly every revision ever sent, accepted or not.
- `(inactivity_prompted_at)` where it is set: the worker's search for idle exchanges whose prompt has gone unanswered, on every pass.

The last two are partial, so they hold only the exchanges the worker is looking for. Like every migration this one runs in a transaction, so the indexes are built without `CONCURRENTLY` and writes to `exchange` wait while they build. Against the load check's 4,500 exchanges the whole migration took a tenth of a second.

## 0010_wallet

What Wallet passes need beyond the `wallet_pass` table of 0001 (`DESIGN.md` §11, [docs/wallet.md](../../docs/wallet.md)).

- **`wallet_pass`** gains the SHA-256 of an Apple pass's authentication token (Apple's only: a check refuses one on a Google pass, and anything but 32 bytes), the face's Last-Modified time and the hashes of the face it belongs to and of the face last delivered, and the update queue's columns: a mark counter, when to try next, the worker's lease, the attempts and the last error. The pass row is its own queue entry, marked in the transaction of every change to its exchange. Its serial (`external_id`) must fit both Apple's and Google's rules. The application role keeps the grants of 0001: it reads, adds and updates passes, and never deletes one; a revoked pass is voided, not removed.
- **`wallet_device_registration`**: the devices that asked Apple's pass web service for a pass's updates, with their push tokens, which the service adds, updates and removes as devices and Apple say. Working data, granted in full.

`backend/tests/schema.rs` checks the constraints and the grants; `backend/tests/wallet.rs` everything through the API.

## 0011_devices

Push notifications and the cost of text messages (`DESIGN.md` §12, §13).

- **`device`**: a device the app registered for push, with its Expo push token, platform, app version and language, under the account and the session it was signed in with. One token is one device (`UNIQUE (service, token)`): registered again by another account, it moves to that account (since a later change only once the session it was registered under has ended; `backend/src/http/devices.rs`). Removing the session removes the device (`ON DELETE CASCADE`), and the service also removes it when that session signs out, when the account is deleted, when the push service says the token is no longer registered, and, in the worker, once its session has expired or been revoked. `service` names the push service the token belongs to, only `EXPO` so far, so that tokens for Apple's and Google's own services could sit beside them.
- **`push_ticket`**: a message Expo accepted, waiting for its receipt, by Expo's ticket ID. A receipt saying the token is no longer registered removes the device, and its tickets with it; a ticket is forgotten once its receipt is read, or after a day, when Expo keeps it no longer (the worker removes those on its own, whether or not receipts can be read).
- **`sign_in_limit`** may also hold `sms-sent`, `sms-refused` and `sms-failed`: the codes sent by text message, those refused under the service's hourly cap (`SMS_MAX_PER_HOUR`), and those the provider did not take, each counted for the whole service per hour under one keyed-hash subject. The cap is decided on the `sms-sent` row, locked like every other count, so it holds across copies of the API.

The application role may read, add, change and remove rows in both new tables: they are working data, like sessions. Neither holds anything from an agreement. A push token identifies an installed app, so the backups that hold it are held as narrowly as the rest of the database.

`backend/tests/schema.rs` checks the keys, the checks and the cascade as the application role; `backend/tests/push.rs` and `backend/tests/sms.rs` drive them through the API and the worker.

## 0012_wallet_tokens

Wallet passes after review (`DESIGN.md` §11, [docs/wallet.md](../../docs/wallet.md)).

- **`wallet_auth_token`**: an Apple pass's authentication tokens, by their SHA-256 only (32 bytes, each once). A token is random and new each time the pass is handed out, and the service keeps the latest three; the one hash each pass had in `wallet_pass.auth_token_hash` moves here, so passes already on devices keep working, and that column is dropped.
- **`wallet_download_link`**: a link that downloads an Apple pass without a session, by its token's SHA-256, until it expires and only once (`used_at`). The worker removes those that expired a day ago.
- **`wallet_pass`** gains `face_date`, the day in the exchange's timezone that the face last delivered was drawn for, from which the worker marks passes whose "Due soon" or "Overdue" may have changed, and the hourly count of new device registrations.
- **`wallet_device_registration`** gains `void_listed_at`: when the device was told that its voided pass changed. The registration is removed a day after, or 30 days after the voiding.

The application role may read, add, change and remove rows in both new tables: they are working data. Neither holds a token, only hashes.

`backend/tests/schema.rs` checks the checks and the grants; `backend/tests/wallet.rs` everything through the API and the worker.

## 0013_sms_limits

Harder-to-starve limits on text messages (README, "Signing in"). `sign_in_limit` may also hold `sms-sent-by-prefix`, the codes sent by text message per number prefix (the country code and the three digits after it, the area code for `+1`) under `SMS_MAX_PER_PREFIX_PER_HOUR`, its subject a keyed hash of the prefix; and, for the metrics, `sms-refused-prefix` and `sms-refused-country`, the codes refused under that cap and those refused because the number's country is not in `SMS_ALLOWED_COUNTRY_CODES`, each for the whole service. Since this change `sms-sent` counts only messages the provider took: a place is taken before a message is handed over and given back if the provider refuses it. No table or grant changes.

## 0014_staff_review

Staff review of abuse reports (`DESIGN.md` §9, §18 item 5; [docs/operations.md](../../docs/operations.md), "Reviewing reports").

- **`staff_member`**: the accounts that review reports. The application role may only read it; the `staff` command writes it with the owner's connection, so no API request can make anyone a reviewer.
- **`report`** gains its resolution: `resolved_by` (the reviewer), `outcome` (`DISMISSED`, `CONTENT_HIDDEN`, `ACCOUNT_SUSPENDED`, `CONTENT_HIDDEN_AND_ACCOUNT_SUSPENDED`) and `resolution_note`, set together with `resolved_at`, and agreeing with `status`. A trigger refuses any change to a report once it is resolved, and any change to what was reported; another refuses removing reports, for every role.
- **`review_event`**: the audit history. Every report opened, every decision and every undoing by a reviewer, and every grant and revoke from the command line, with who, when, the report, exchange and account concerned, and the note. Append-only like the agreement history: the application role may read and add, and a trigger refuses `UPDATE`, `DELETE` and `TRUNCATE` for every role.
- **`hidden_content`**: an exchange's content hidden from one account by a reviewer, until it is shown again. Working state: the application role may read, add and remove rows.

**Why resolution on the report and history apart.** The report row answers "how did this end" in one place, set once. Everything else, including looking again later, is a new event, so nothing is ever edited. A separate table of reviews per report would have held the same thing as the events, less completely: views, suspensions lifted and content shown again are not reviews of one report, but belong in the same history.

The link an unsigned report came through (§18 item 5) is not added: the owner decided on 3 October 2026 that reading a proposal needs signing in, which leaves reports made without an account to the past; a reviewer sees such a report as made "through the invitation link", and the exchange it names is the link's.

`backend/tests/schema.rs` checks the grants, checks and triggers; `backend/tests/review.rs` everything through the API and the command.

## 0015_deletion_log

The deletion log ([docs/operations.md](../../docs/operations.md), "Replaying deletions"). **`deletion_log`** holds one row per deleted account, its ID and when it was deleted, and nothing else; the row is written in the transaction that deletes the account. `scripts/backup.sh` exports it beside every backup, and `scripts/replay-deletions.sh` applies it to a restored copy, so that a restore does not bring back accounts deleted since its backup. Accounts deleted before this migration are added with the time it ran.

The application role may read the log and add to it, never change or remove a row. `backend/tests/schema.rs` checks the grants and that an account appears once; `backend/tests/deletion.rs` the writing and the replay.

## 0016_replay_lifts_suspension

Replaying the deletion log deletes an account that the restored copy holds suspended: a suspended account cannot delete itself, so its suspension was lifted after the backup, and the replay lifts it again and deletes the account in one transaction ([docs/operations.md](../../docs/operations.md), "Replaying deletions"). That lifting goes into `review_event` like any other, but no reviewer did it; the owner did, by running `replay-deletions`.

0014 already records the owner's command line with no reviewer (`staff_account_id` empty), for naming and removing reviewers, and the staff screen shows such an entry as the owner's. This migration replaces 0014's check that only those two actions may lack a reviewer with `review_event_actor`, which also lets `SUSPENSION_LIFTED` lack one, provided it has a note saying why. Every other action of a reviewer still needs the reviewer, and naming or removing one still needs none.

**Why not an account for the system.** A made-up account to stand for the replay would be a row in `account` that can never sign in, that every query about people must leave out, and that the deletion log, the reviewer list and the staff screen would each have to know about. An empty reviewer already means "the owner, from the command line", which is exactly who runs a replay. **Why not a new action.** The event is a suspension lifted; a new name for it would make the history of a suspension read differently depending on who lifted it, and change the API and the staff screen's wording for no gain.

`backend/tests/schema.rs` checks the new rule; `backend/tests/deletion.rs` the replay that uses it.

## 0020_sms_updates

Text updates for an agreement, "Yuppers.app agreement updates" (README, "Text updates"; `DESIGN.md` §12).

- **`sms_update`**: an agreement a person turned text updates on for, with the number they were turned on with, one row per person and agreement while they are on. Removed when they are turned off, by the person, by a STOP reply, by a change of the account's number or by deleting the account.
- **`sms_consent`**: the record of consent, kept as proof of opt-in. Every opt-in (who, which agreement, the number, the time, the version and language of the consent wording, and from which client), every opt-out and why, and every STOP and START received, with the word. Checks hold an opt-in to its wording and an opt-in or opt-out to a person and an agreement. The application role may read it and add to it, never change it; it may remove rows only because the worker's retention purge does, four years after the updates they cover ended.
- **`sms_consent_network`**: an opt-in's IP address and user agent, apart, removed with the signatures' after 90 days; and with its record, by cascade.
- **`sms_opt_out`**: the numbers that replied STOP and not START since. Nothing is texted to them.
- **`outbox`** takes a fourth kind, `SMS`, and an index on its recipient and time for the daily cap on update texts.

Working data and the record both name a phone number in full, as `account` does: the number is what consent was given for. `backend/tests/schema.rs` checks the checks and grants; `backend/tests/sms_updates.rs` everything through the API, the webhook and the worker.

## 0021_sms_code_consent

Consent to a one-time code by text, "Yuppers.app sign-in codes" (README, "Signing in"; `backend/src/code_consent.rs`): every form that texts a code shows a box beside the number, and a code is texted only once it is ticked.

- **`sms_code_consent`**: one row for each code issued after a tick. Why it was asked for (`SIGN_IN`, `DELETE_ACCOUNT`, `VERIFY_NUMBER`), the account where there is one (signing in has none until the number is an account's), the wording's version and language, the client and the time. The number is kept in full only beside the account it already belongs to; every row has `phone_hash`, an HMAC-SHA256 of the number under `APP_SECRET`, so a record can be found from a number without the table holding numbers nobody has shown are theirs. Checks hold every purpose but signing in to an account, and a number in full to one. The application role may read it and add to it, never change it; it may remove rows only because the worker's retention purge does, after `Rules::sms_consent_retention`.
- **`sms_code_consent_network`**: the request's IP address and user agent, apart, removed with the signatures' after 90 days; and with its record, by cascade.

Not `sms_consent` (0020): an opt-in there is always a person and an agreement, and its number always kept, and its purge keeps what a subscription rests on; a code is asked for without an agreement, often without an account, for a number nobody has yet shown to be theirs. Fitting these rows there would loosen every check of 0020. `backend/tests/schema.rs` checks the checks and grants; `backend/tests/auth.rs` and `backend/tests/deletion.rs` the refusals and the records through the API.

## 0022_verify_codes

One-time codes for phone numbers through Twilio Verify (README, "Signing in"; `backend/src/notifications/verify.rs`): Twilio makes, texts and checks the code, so the service never sees it and has no hash to keep. `one_time_code` gains `checked_by`, `SERVICE` for every code the service makes (email, and the development log) and `TWILIO_VERIFY` for the others, and `code_hash` may be empty, but only for `TWILIO_VERIFY`. Such a row records that a code was asked for, for which identifier and purpose, when and until when, so the hourly limits, the live codes, the wrong guesses per code and per day and the purpose binding hold as before. Once Twilio approves a code offered back, its keyed hash is written to the row, so that the same code is recognised again until it is used or expires: Twilio forgets an approved verification, and a deletion that found the account busy is retried with the same code. Rows already stored are `SERVICE`, as they were. The application role's grants on the table are unchanged. `backend/tests/verify.rs` checks it through the API, against a stand-in for Verify.

## 0023_sms_inbound_seen

`sms_inbound_seen`: the MessageSid of every STOP and START the webhook took (`backend/src/http/sms.rs`), and when. Twilio's signature has no time in it, so a request captured once would check out again, and a START replayed after a later STOP would lift the opt-out; a MessageSid already here is answered as taken and changes nothing. Only the SID and the time: no number, no text. The application role may read, add and remove rows; the worker removes them after 30 days (`sms_updates::INBOUND_SEEN_RETENTION`). `backend/tests/sms_updates.rs` posts the same signed START twice.

## 0024_sms_consent_no_longer_a_party

`sms_consent.source` takes `NO_LONGER_A_PARTY`: a party removed from an agreement before the other party confirmed them, or who left it then, loses its text updates with the rest of what was theirs alone in it (`exchanges::repo::vacate`), and the ending is recorded as every other is.

## Outside the database

**What the service still owns:** computing content hashes, validating timezones, generating display codes, rejecting dependency cycles, checking invitation expiry, and every state transition.
