# Operations

How to run the service: the first deployment, what to check and watch, backups and the restore drill, rotating the secret, and what to do when the worker stops. Nothing here assumes a particular host. A deployment is a managed PostgreSQL database, a container platform that can run one image three ways, a reverse proxy or load balancer that terminates TLS, and an email provider's account (Resend's API, or any SMTP server); for codes to phone numbers, an SMS provider's account, and for push notifications, an Expo project ("Text messages and push notifications", below). `.env.example` documents every setting with its default; README, "Deploying", says what the image is.

The first deployment is on Render: [docs/deploy-render.md](deploy-render.md) is that checklist, with `render.yaml` the Blueprint. What follows holds there as anywhere, with the differences that page names (one-off commands, backups, reaching the database).

## The processes

| Process | Runs | Listens on |
|---|---|---|
| `migrate` | once per release, before the others start, then exits | nothing |
| `api` | always; as many copies as the load needs | `BIND_ADDR` (the image sets `0.0.0.0:8080`), and `METRICS_ADDR` if set |
| `worker` | always; one copy | `METRICS_ADDR` if set, nothing else |

All three come from the same image: the `api` is its default command, the other two are `/usr/local/bin/worker` and `/usr/local/bin/migrate`. The image has no shell, so health checks are HTTP requests made by the platform, and backups run from somewhere else (below).

## First deployment

1. **The database.** Create a PostgreSQL 17 database and two roles: the schema owner (here `exchange`), which owns the database and runs migrations, and the application role `exchange_app`, which may log in and nothing more. Each gets a long random password of its own.

   ```sql
   CREATE ROLE exchange LOGIN PASSWORD '...';
   CREATE ROLE exchange_app LOGIN PASSWORD '...';
   CREATE DATABASE yuppers OWNER exchange;
   ```

   On a managed service the owner may be the role the service gives you; what matters is that the API and worker never connect as it. Where nobody can run `psql` before the first deploy, `migrate` can create `exchange_app` itself: `MIGRATE_CREATE_APP_ROLE=true` with the password in `APP_DB_PASSWORD` creates it if it does not exist, and refuses a role that can do more than log in (README, "Deploying"; [docs/deploy-render.md](deploy-render.md)). `exchange_app` must exist before the first migration, which grants to it, and must have exactly that name: the migrations name it, and they are never edited once applied. (The roles predate the name Yuppers; the database's name is free.) Require TLS to the database if the service offers it (`?sslmode=require` on both connection strings).

2. **Secrets.** In the platform's secret store, never in the image or the repository:
   - `DATABASE_URL`: `exchange_app`'s connection string, for the api and the worker.
   - `MIGRATION_DATABASE_URL`: the owner's, for `migrate` only.
   - `APP_SECRET`: `openssl rand -hex 32`.
   - `CONTACT_DATA_KEY`: `openssl rand -base64 32`, for `migrate`, the api and the worker. **Keep a copy outside the platform** before it is used: without it no address or number stored can be read again ("Contact data key" below).
   - `RESEND_API_KEY` for email by Resend's API, or for SMTP `SMTP_PASSWORD`, and `SMTP_USERNAME` if the provider treats it as secret.
   - When they are switched on: for the api, `SMS_API_KEY_SECRET` with its `SMS_API_KEY_SID` (recommended) or else `SMS_AUTH_TOKEN`, and `EXPO_ACCESS_TOKEN` for the worker if the Expo project has push security on.
   - Once Wallet passes are wanted: `APPLE_PASS_KEY` and `GOOGLE_WALLET_SERVICE_ACCOUNT` ([docs/wallet.md](wallet.md)).

3. **Settings** (plain environment):
   - `WEB_ORIGIN`: the public HTTPS origin, such as `https://app.example.com`, without a trailing slash. Cookie sessions are honored only from it, emails link into it, and because it is HTTPS every response carries HSTS, for its host and every name under it (`includeSubDomains`): serve every subdomain of that host over HTTPS.
   - `EMAIL_FROM`, the sender (`SMTP_FROM`, its older name, still works), and `CODE_DELIVERY` and `NOTIFICATION_DELIVERY`: `resend` (Resend's API, with `RESEND_API_KEY`) or `smtp` with `SMTP_HOST`, `SMTP_PORT`, `SMTP_TLS` (`tls` for port 465, `starttls` for 587). The sending domain needs the provider's SPF and DKIM records, or codes land in spam.
   - `TRUSTED_PROXY_HEADER`: the header the proxy in front of the API sets to the client's address, and `TRUSTED_PROXIES` if more than one proxy appends to `X-Forwarded-For`. Without it every signature records the proxy's address (`DESIGN.md` §8) and the per-address sign-in limits count every person as one. Name a header only if the proxy always sets it and clients cannot reach the API around the proxy; otherwise a client can choose its own address.
   - The `SIGN_IN_*` limits only if the placeholders in `.env.example` do not suit (README, "Deploying").
   - `SMS_CODE_DELIVERY`, `SMS_DELIVERY` and `PUSH_DELIVERY` stay unset (off) until their accounts exist; "Text messages and push notifications" says how to switch each on.
   - `LOG_FORMAT=json` if a log collector reads the output; `RUST_LOG` stays `info`.
   - `METRICS_ADDR=0.0.0.0:9100` on the api and the worker, if something will scrape them (below).
   - The Wallet settings, on the api and the worker, once the accounts exist, with `WALLET_DELIVERY=live` ([docs/wallet.md](wallet.md)). Until then, none of them.

4. **Migrate.** Run the image with `/usr/local/bin/migrate`, `MIGRATION_DATABASE_URL` and `CONTACT_DATA_KEY`. It exits 0 with `migrations applied`. Safe to run again; every release runs it before the new api and worker start.

5. **The worker.** One copy, `/usr/local/bin/worker`, with `DATABASE_URL`, `CONTACT_DATA_KEY`, `WEB_ORIGIN`, the delivery and email settings, and `PUSH_DELIVERY` (with `EXPO_ACCESS_TOKEN`) once push is on. It logs `worker started`. Give it a stop grace period of at least 35 seconds: on `SIGTERM` it finishes the message it is sending (at most 30 seconds) and exits 0.

6. **The api.** The image's default command, with `DATABASE_URL`, `APP_SECRET`, `CONTACT_DATA_KEY`, `WEB_ORIGIN`, `CODE_DELIVERY`, the email settings and `TRUSTED_PROXY_HEADER`, and once they are on, the `SMS_*` and `TWILIO_VERIFY_*` settings and the same `PUSH_DELIVERY` as the worker. It serves the web app itself from `/srv/web`. Point the platform's health check at `/readyz` (below). Any number of copies; each holds up to 10 database connections, so copies × 10 plus the worker's 10 must stay under the database's connection limit.

7. **TLS at the proxy.** The proxy or load balancer terminates HTTPS for `WEB_ORIGIN`'s host and forwards plain HTTP to port 8080, adding the header named in `TRUSTED_PROXY_HEADER`. Redirect HTTP to HTTPS there. Do not route the metrics port through it.

8. **Check it.** `https://<origin>/healthz` and `/readyz` answer 204; `/v1/meta` and the `X-Yuppers-Version` header name the commit just deployed ("What is deployed"); the home page loads; sign in with a real address and the code arrives; an exchange between two test accounts sends both their notification emails within a few seconds. Each response carries an `X-Request-Id`.

## Text messages and push notifications

Both are built and off: with `SMS_CODE_DELIVERY`, `SMS_DELIVERY` and `PUSH_DELIVERY` unset nothing changes from an email-only deployment. Each is switched on by settings alone, once its account exists. README, "Notifications" and "Signing in", says what each does.

**Codes by text message, and agreement updates.**

Two settings, for two kinds of text. `SMS_CODE_DELIVERY` says how one-time codes reach phone numbers: through **Twilio Verify**, which makes the code, texts it from Twilio's own registered senders in its own template and checks it, and needs no messaging campaign. `SMS_DELIVERY` says how the one text-message program, "Yuppers.app agreement updates", is sent: through Twilio's **Messages API** from a number registered for it. Either can be on without the other. docs/deploy-render.md, "Text messages", has the console steps, the campaign's text and the costs.

1. **The account.** Open an account with Twilio and restrict its geographic permissions to the countries served: for a US launch, the United States only (add Canada if it is served). `SMS_ALLOWED_COUNTRY_CODES` (default `+1`) is the service's own check, by country calling code, and `+1` also covers Canada and some twenty Caribbean and Pacific countries, whose messages cost more and are where SMS pumping sends its traffic; `SMS_ALLOWED_REGIONS` (default `US`) narrows `+1` by area code to the 50 states and DC (`US,CA` adds Canada; `backend/src/nanp.rs`), so the service refuses the rest itself, and the provider's permissions are the second check. A message the provider refuses uses up no place under the caps.
2. **A credential**, for both. In Twilio's console, create a **standard** API key for the account (the **API keys & tokens** page) and copy its SID (`SK...`) and secret; the secret is shown once. A standard key reaches every API but the Accounts and Keys resources, so it can send messages and verifications but cannot manage the account or other keys, and it can be revoked on its own if it leaks, so it is the recommended credential. To rotate it, create a new key, set both settings and restart the api and the worker, then revoke the old key. The account's auth token also works (`SMS_AUTH_TOKEN`), but rotating it touches everything that uses the account. Set one or the other: the api refuses to start with both, with neither, with half a key, or with a SID that is not `AC` (or `SK`) and 32 hexadecimal digits. Either way each request carries the credential as HTTP Basic, as Twilio documents ([requests to Twilio](https://www.twilio.com/docs/usage/requests-to-twilio), [API keys](https://www.twilio.com/docs/iam/api-keys)).
3. **Codes: Twilio Verify.** Create two Verify services, SMS only, code length 6: "Yuppers", for signing in and confirming a number added to an account, and "Yuppers account deletion", for deleting an account (Twilio refuses the dot in "Yuppers.app" as a friendly name, error 60200). Two, because a Verify service resends one number the same code for every request within its ten minutes, so one service for both would let a code sent for one purpose work for the other; the api refuses to start with the same SID twice. On the api: `SMS_CODE_DELIVERY=verify`, `SMS_ACCOUNT_SID`, the credential, `TWILIO_VERIFY_SERVICE_SID` and `TWILIO_VERIFY_DELETION_SERVICE_SID` (`VA` and 32 hexadecimal digits). The service then starts a verification (`POST https://verify.twilio.com/v2/Services/{sid}/Verifications`, `To`, `Channel=sms`, `Locale` `en` or `es`) for each code asked for, and checks a code offered back (`.../VerificationCheck`, `To`, `Code`) ([Verifications](https://www.twilio.com/docs/verify/api/verification), [VerificationCheck](https://www.twilio.com/docs/verify/api/verification-check)). It keeps no code and no hash of one it never saw: only that a code was asked for, for what, and until when, so the same limits count it as any code (README, "Signing in"). A wrong code is counted against those limits whatever Twilio says; a check Twilio does not answer is refused as unavailable and counted against nothing.
4. **Updates: the Messages API.** Buy a number able to send SMS, or create a Messaging Service, and register the brand and campaign (A2P 10DLC), without which carriers block or filter the messages. On the api and the worker: `SMS_DELIVERY=twilio`, `SMS_ACCOUNT_SID`, the credential and `SMS_FROM` (`+1...` or `MG...`). The worker sends the update texts people turn on, and needs `APP_SECRET`, with which it counts them under the same hourly caps as the api's codes; it refuses to start with SMS on and no `APP_SECRET`. Point the number's (or its Messaging Service's) incoming-message webhook at `https://{your origin}/v1/sms/inbound`, HTTP POST, and give the api the account's auth token as `SMS_WEBHOOK_AUTH_TOKEN` (not needed when `SMS_AUTH_TOKEN` is the credential): Twilio signs those requests with it, and without it every STOP and START is refused, which the api warns about at start. A number that replied STOP there gets no more texts from the service, codes included: the service does not ask Verify for one. Turn on Twilio's Advanced Opt-Out with the HELP and STOP replies in docs/deploy-render.md; the service never replies to a text itself. `SMS_UPDATES_PER_PERSON_PER_DAY` (default 20) bounds the update texts one person is queued a day.
5. **The caps.** `SMS_MAX_PER_HOUR` if 50 texts an hour, codes and updates together, is not right, `SMS_MAX_PER_PREFIX_PER_HOUR` if 10 an hour to one area code is not, and `SMS_ALLOWED_COUNTRY_CODES` if countries other than `+1` are served. The api logs at start how many it may send an hour.
6. Check: ask for a code for a phone you hold; it arrives in one message from Twilio Verify, in the account's or the browser's language, and `yuppers_sms_codes_this_hour{result="sent"}` counts it. With updates on, turn them on for an agreement: the confirmation arrives from the campaign's number.

With `SMS_CODE_DELIVERY` off (and `CODE_DELIVERY=resend` or `smtp`), a code for a phone number is refused as unavailable, as before, and the apps know it: `GET /v1/meta` says `"sign_in_channels": ["email"]`, so the sign-in screens ask for an email address only and stop a phone number typed anyway, and deleting an account sends its code by email. Turning it on adds `phone`, with the countries in `sms_country_codes`; an open page or app sees the change the next time it shows a sign-in form. `SMS_DELIVERY` has no part in it. Costs to watch are in "What to watch".

**Push notifications.**

1. The Expo project, the APNs key and the Firebase credentials, as docs/mobile-release.md, "Once the accounts exist", step 6, says; and a build of the app made after them.
2. On the api and the worker: `PUSH_DELIVERY=expo`. On the worker, `EXPO_ACCESS_TOKEN` if push security is on for the project. The api then tells the apps to offer notifications; the worker sends them, reads Expo's receipts about fifteen minutes later, and removes devices Expo says are gone.
3. Check: on a device, turn notifications on, have the other party act, and see `yuppers_push_deliveries_total{result="sent"}` go up and the notification arrive.

`PUSH_DELIVERY=log` on both is for development: the worker writes each notification to its log. Turning push off again closes whatever is queued for push unsent, and the apps stop offering it; the devices stay registered, harmlessly, until their sessions end.

## The terms version

Signing in is the assent to the Terms and the Privacy policy: the sentence above the sign-in button names them, and the request that completes the sign-in carries the version shown (`terms_version`). The service refuses a version it does not know (`TERMS_VERSION_UNKNOWN`) and stores the one accepted, with the time, in `terms_acceptance` and on the account (`GET /v1/me`). The version is the documents' effective date. To publish new terms or a new policy, change `TERMS_VERSION` in `backend/src/terms.rs` and in `packages/shared/src/terms.ts` together with `LEGAL_EFFECTIVE_DATES` in `packages/shared/src/legal-text.ts`; a test fails if they differ. Pages loaded before the deploy are asked to reload on their next sign-in. The field is optional on the request, because builds already in users' hands (the TestFlight app) do not send it: without it the sign-in completes and nothing is recorded. It becomes required once every shipped client sends it, by raising the minimum client version (`backend/src/client_version.rs`) past the first build that does, and then making the field required in `CreateSession` and the API document. Nothing is backfilled: an account gets its first row at its next sign-in, and nothing yet asks an account that stays signed in to accept a newer version.

## Health checks

| Path | Answers | Use it for |
|---|---|---|
| `GET /healthz` | 204 while the process runs | liveness: restart the copy if it fails |
| `GET /readyz` | 204 when the database answers, 503 when not | readiness: send traffic only when it passes |

The api starts without a database and answers `/readyz` with 503 until it can reach one, so a database outage takes copies out of rotation rather than restarting them in a loop. The worker has no health path; with `METRICS_ADDR` set, its `/metrics` answers while it runs, and `yuppers_worker_last_pass_timestamp_seconds` says when it last went round its jobs (every 5 seconds).

## What is deployed

Every build carries the git commit it was made from and when it was made (the image's `GIT_SHA` and `BUILD_TIME` build arguments; README, "Deploying"). To tell what is running:

- **The API**: `curl https://<origin>/v1/meta` gives `version` (the package's), `commit` (in full) and `built_at`.
- **Any response** carries `X-Yuppers-Version: <the commit's first seven characters>`, a page or an asset as much as an API call: `curl -sI https://<origin>/healthz | grep -i x-yuppers-version`. During a rolling deploy two values can answer for a minute.
- **The logs**: each process's first line is `build`, with `process` (`api`, `worker`, `migrate`), `version`, `commit` and `built_at`. So the worker, which has no address, says what it runs too.
- **The metrics**: `yuppers_build_info{version="0.1.0",commit="..."} 1` on each process's metrics listener.
- **The web app**: the account screen and the staff screen end with "Version 0.1.0 (abc1234)", the commit the web build was made from, and each request names it (`X-Client-Version: web/0.1.0+abc1234`). The apps show "Version 0.1.0 (build 12, abc1234)", the store build number and, for an EAS build, its commit.

**Matching it to a commit and a CI run.** The commit is the repository's: `https://github.com/vchance/yuppers/commit/<commit>`, whose checks list the CI run that tested it (or `gh run list --commit <commit>`). The `Container image` job of that run built an image from the same commit and checked that `/v1/meta` reports it. `unknown` means the build was given no commit: a local `cargo build`, or a platform that passed none, which is worth fixing before relying on it. On Render, the service's Events page names the commit each deploy built, and the process also reads `RENDER_GIT_COMMIT` when the build had none ([docs/deploy-render.md](deploy-render.md)).

## Logs

Every process writes to standard output, one line per event. `LOG_FORMAT=text` (the default) is for reading; `LOG_FORMAT=json` writes one JSON object per line: `timestamp`, `level`, `target`, `message`, the event's own fields, and for anything logged while handling a request, `span` with that request's `method`, `path`, `request_id`, `trace_id` and `span_id` (the trace's identifiers, the same ones the exported trace has; "Telemetry" below). In the worker, a line logged by a job carries `span` with the `job` (`timers`, `reminders`, `notifications`, ...) and the pass's `trace_id`.

Each request writes one line when it is answered, `request completed`, with:

| Field | What it is |
|---|---|
| `method`, `path` | The method and the path, never the query string |
| `status` | The status code returned |
| `latency_ms` | Time to answer, in milliseconds to the microsecond |
| `request_id` | `X-Request-Id` from the request if it is 1 to 64 letters, digits, `-`, `_` or `.`; otherwise a new UUID. Returned in the response's `X-Request-Id`, and on every other line logged for that request |
| `trace_id`, `span_id` | The request's trace (32 hexadecimal characters) and its span (16), made by the service for each request; no client's or proxy's trace header is believed. In Grafana, the trace by that ID is the same request |

A proxy that sets its own request ID ties its logs to the service's that way.

**Never logged:** request or response bodies, query strings, headers other than the request ID (so no `Authorization`, no cookie), session or invitation tokens, one-time codes, email addresses, phone numbers. A database error is logged by its SQLSTATE, constraint and table, never the server's message, which can quote a value. An email that could not be sent is logged by its outbox ID with the SMTP reply code only, or Resend's HTTP status and error name (`HTTP 422, validation_error`); the same goes into `outbox.last_error`. A push notification likewise, with the HTTP status and Expo's error code, a Wallet pass update with the HTTP status and APNs's or Google's error code (also in `wallet_pass.last_error`), and a text message that could not be sent with the HTTP status and Twilio's error code: never the provider's message, which quotes the token or the number. `backend/tests/telemetry.rs` signs a person in at every log level and checks that the output holds none of their address, phone number, codes, token or cookie; `backend/tests/smtp.rs` and `backend/tests/resend.rs` do the same for a refused recipient, and `backend/tests/sms.rs` for a refused text message.

**The reverse proxy's logs.** The service never logs a query string, but a proxy or load balancer in front of it logs what it is configured to. Configure its access log not to record query strings, at least for `/v1/wallet/apple/pass`: that link's `token` downloads a person's Wallet pass (once, within ten minutes, but still theirs). Most proxies log the full request line by default; log the path alone (nginx `$uri` rather than `$request`, Caddy's log filters, a load balancer's field selection).

The one exception is the development deliveries, `CODE_DELIVERY=log`, `NOTIFICATION_DELIVERY=log`, `SMS_CODE_DELIVERY=log`, `SMS_DELIVERY=log` and `PUSH_DELIVERY=log`, which write each code, email, text message and push notification to the log because that is their job. Even they write a phone number masked (`+1••••••••67`) and a push token by its first characters only. A deployment has to choose `CODE_DELIVERY` and `NOTIFICATION_DELIVERY`, so it never gets the log by default; check that both say `resend` or `smtp`, and that `SMS_CODE_DELIVERY`, `SMS_DELIVERY` and `PUSH_DELIVERY` are unset, `verify`, `twilio` and `expo`.

Useful lines besides requests: `api listening`, `worker started`, `worker shutting down`, `notifications delivered` and `push notifications delivered` (counts per pass), `notification not sent; will retry`, `push notification not sent; will retry`, `notification given up on` and `push notification given up on` (with the outbox ID; `refused_for_good=true` when the provider refused it for good, such as Resend's `422` for an address it will not take, which is not retried), `push receipts read` and `push receipts could not be read`, `devices of ended sessions removed`, `push tickets past their receipts removed`, `push notifications are off (PUSH_DELIVERY)` at the worker's start, `email handed to the SMTP server` (with the server's reply code and its reply, such as the provider's own ID for the message, never an address), `email handed to Resend` (with Resend's `id` for the message), `Resend refused the API key or the sender` and `Resend's sending quota is spent` (errors: check `RESEND_API_KEY`, the domain and the plan; the message is retried for a day without counting the tries, logged as `notification not sent: the provider refuses everything for now`), `the SMTP server refused the credentials` (the same, for SMTP), `notification given up on` with `idempotency_conflict=true` (Resend held another message under the row's key; not sent again), `one-time code could not be delivered`, `a one-time code could not be checked` (Twilio Verify did not answer a check; nothing was counted), `a code was not sent by SMS: an hourly cap is reached` (with `whose`: the whole service, or numbers beginning alike), `update texts delivered` (counts per pass), `update text not sent; will retry`, `update text given up on` and `update text held back by the hourly cap` (with the outbox ID), `agreement updates by text are off (SMS_DELIVERY)` at the worker's start, `STOP received: the number gets no more texts` (with how many agreements' updates it turned off), `START received: the number may be texted again`, `a request to the SMS webhook was refused: its signature is not Twilio's`, `text update consent records past retention removed`, `code consent records past retention removed`, `old sessions, idempotency keys and notifications removed` (about once an hour: sessions 30 days after they ended, idempotency keys 30 days after their request, finished notifications 90 days after they were queued; `backend/src/sweep.rs`), `timers ran`, `reminders queued`, `issuing Wallet passes` (at start, naming the platforms), `APPLE_PASS_CERT has expired: Apple Wallet passes are off until it is renewed` and `APPLE_WWDR_CERT does not vouch for APPLE_PASS_CERT: Apple Wallet passes are off` (errors: Apple is off, everything else runs), `APPLE_PASS_CERT expires within 30 days`, `a Google Wallet object could not be created` (a save link was refused), `wallet passes updated`, `wallet pass not updated; will retry` and `wallet pass update given up on` (with the pass's ID), `database error`, `readiness check failed`.

## Metrics

Off unless `METRICS_ADDR` is set. Then the process serves `GET /metrics` in the Prometheus text format on that address, a listener of its own: it is never on the API's port, so publishing the API cannot publish the metrics by accident. Keep the metrics port on the private network, reachable only by the scraper. The api and the worker each have their own; a scraper collects both. The same page is pushed to an OpenTelemetry collector every fifteen seconds when one is configured ("Telemetry" below): the names below are what Prometheus shows either way (over OTLP a metric travels under its OpenTelemetry name, `yuppers.http.requests` with the unit `{request}`, and the collector derives the name below from it).

From both, the **funnel** (`backend/src/funnel.rs`): each step counted where it happens, since the process started. No label names a person or an exchange.

| Metric | Type | Labels |
|---|---|---|
| `yuppers_accounts_created_total` | counter | `channel`: `email`, `phone`. A first sign-in made an account (the api) |
| `yuppers_yups_created_total` | counter | A first proposal was sent, opening a negotiation |
| `yuppers_invitations_shared_total` | counter | The initiator passed the invitation on (`POST /v1/exchanges/{id}/invitation/shared`), or it was claimed with no share recorded |
| `yuppers_invitations_claimed_total` | counter | The other party opened and claimed an invitation |
| `yuppers_agreements_in_force_total` | counter | Both signed, the first time an exchange came into force (an amendment in force is not another) |
| `yuppers_contributions_confirmed_total`, `yuppers_contributions_disputed_total` | counter | A recipient confirmed a delivery; a party disputed a contribution |
| `yuppers_agreements_closed_total` | counter | `outcome`: `withdrawn`, `declined`, `expired`, `discarded` (nothing was agreed), `completed`, `ended_by_agreement`, `unresolved_close_request`, `unresolved_inactive`. The timers' closures count in the worker, the parties' in the api |
| `yuppers_codes_sent_total` | counter | `channel`: `email`, `phone`. One-time codes handed to a provider (the api) |
| `yuppers_texts_sent_total` | counter | Agreement updates sent by text (the worker) |
| `yuppers_emails_sent_total` | counter | `kind`: `notice` (about an exchange, reminders included), `account` (a report to review, accounts combined). Emails the worker sent from the outbox; codes count under `yuppers_codes_sent_total` |

From the worker, the funnel's **daily snapshot**: the previous UTC day's counts, read from the database once a day (on the worker's first pass, then after each midnight UTC; the `funnel snapshot` log line carries the same numbers), so that a deploy, which starts the counters above again, loses nothing. Aggregate counts only.

| Metric | Type | |
|---|---|---|
| `yuppers_daily_yups_created` | gauge | first proposals sent that day (an exchange's first `REVISION_SENT` event) |
| `yuppers_daily_invitations_shared`, `yuppers_daily_invitations_claimed` | gauge | invitations passed on (a claim counts as one if none was recorded), and claimed, that day |
| `yuppers_daily_agreements_in_force` | gauge | agreements that first came into force that day |
| `yuppers_daily_agreements_completed` | gauge | agreements closed as completed that day |

From the api:

| Metric | Type | Labels |
|---|---|---|
| `yuppers_http_requests_total` | counter | `route` (the route's template, such as `/v1/exchanges/{id}`, or `unmatched` for the web app's pages and unknown paths), `method`, `status` (`2xx`, `4xx`, ...) |
| `yuppers_http_request_duration_seconds` | histogram, buckets from 1 ms to 10 s | the same |
| `yuppers_sms_codes_this_hour` | gauge, only with `SMS_CODE_DELIVERY` or `SMS_DELIVERY` on | Codes by Twilio Verify and agreement update texts count here together: they share the caps. `result`: `sent` (taken by the provider, each a message paid for), `refused` (not sent, for any of the reasons below), `failed` (the provider did not take it; not in `sent`, and it uses up no place under the caps). For the whole service in the current UTC hour, read from the database at each scrape. |
| `yuppers_sms_codes_refused_this_hour` | gauge, the same | `reason`: `hourly_cap` (`SMS_MAX_PER_HOUR`), `prefix_cap` (`SMS_MAX_PER_PREFIX_PER_HOUR`, numbers beginning alike), `country` (the number's country is not in `SMS_ALLOWED_COUNTRY_CODES`, or a `+1` number's area code not in `SMS_ALLOWED_REGIONS`) |
| `yuppers_sms_codes_hourly_cap` | gauge, the same | `SMS_MAX_PER_HOUR` |

From the worker:

| Metric | Type | Labels |
|---|---|---|
| `yuppers_outbox_deliveries_total` | counter | `result`: `sent`, `failed` (every failed try), `given_up` (the last try failed), `dropped` (closed unsent: recipient gone, or a reminder no longer true). Emails. |
| `yuppers_push_deliveries_total` | counter | The same results for push notifications, one per person and notice; `sent` once the push service took it for at least one of their devices, `dropped` also when no device is left or push is off |
| `yuppers_push_devices_removed_total` | counter | devices removed because the push service said their token is no longer registered |
| `yuppers_push_receipt_checks_total` | counter | `result`: `ok`, `error`: requests for the receipts of earlier notifications |
| `yuppers_worker_runs_total` | counter | `job`: `timers`, `reminders`; `result`: `ok`, `error` |
| `yuppers_worker_timer_changes_total` | counter | expiries, lapsed close requests, inactivity prompts and closures |
| `yuppers_worker_reminders_queued_total` | counter | |
| `yuppers_worker_last_pass_timestamp_seconds` | gauge | when the last pass over all jobs ended |

From both, always:

| Metric | Type | |
|---|---|---|
| `yuppers_build_info` | gauge, always 1 | `version`, `commit`: the running build ("What is deployed") |

From both, once Apple Wallet passes are configured:

| Metric | Type | |
|---|---|---|
| `yuppers_wallet_cert_expiry_seconds` | gauge | seconds until the Apple pass type certificate expires; negative once it has, when Apple Wallet passes are off |

From both, read from the database at each scrape (so they are right however many processes send, and the api still shows them while the worker is down):

| Metric | Type | |
|---|---|---|
| `yuppers_outbox_messages` | gauge | `state`: `pending` (waiting, or between retries), `given_up`; emails and push notifications together |
| `yuppers_outbox_oldest_pending_age_seconds` | gauge | how long the oldest pending message has waited since it was queued; 0 when none |
| `yuppers_database_up` | gauge | 0 when that read failed |
| `yuppers_db_pool_max`, `yuppers_db_pool_size`, `yuppers_db_pool_in_use` | gauge | this process's connection pool: its limit, connections open, connections busy |
| `yuppers_reports_open` | gauge | abuse reports waiting for review ("Reviewing reports") |
| `yuppers_reports_oldest_open_age_seconds` | gauge | how long the oldest open report has waited since it was made; 0 when none |

## Telemetry

Off unless `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Then the api and the worker each push three signals to that OpenTelemetry collector over OTLP/HTTP with the JSON encoding (`backend/src/otel.rs`), with the same conventions as the owner's other services, so one Grafana shows them all alike:

- **Traces.** A span per request, named `GET /v1/exchanges/{id}` (the method and the route's template, `unmatched` for the web app's pages), with `http.request.method`, `http.route`, `http.response.status_code` and `request_id`; within it, a span per database operation (`exchange.load`, `exchange.view`, `exchange.persist`, `revision.insert`, with `db.system.name` and `db.operation.name`, never the SQL) and per provider request (`provider request`, with the provider's host, the method and the status; never the path or the bodies). The worker makes a trace per pass, `worker pass`, with a span per job (`worker timers`, `worker notifications`, ...). A span whose request answered 5xx, whose provider could not be reached, or in which an error was logged has the error status. Trace IDs are the service's own, never taken from a client or a proxy.
- **Logs.** Every line the process writes to standard output (at the level `RUST_LOG` allows) as a log record, with the same fields as attributes, `target`, the severity, and the trace and span it happened in, so a line and its trace find each other in Grafana (Loki's `trace_id` to Tempo).
- **Metrics.** The page `METRICS_ADDR` would serve ("Metrics" above), every fifteen seconds, cumulative since the process started, under the OpenTelemetry names (`yuppers.http.requests`, unit `{request}`; `yuppers.http.request.duration`, unit `s`); the collector gives them the Prometheus names listed above. Plus `yuppers_telemetry_dropped_total{signal}`: spans or log records not exported because the buffer was full or the collector refused them.

Every signal carries the resource `service.name` (`yuppers-api`, `yuppers-worker`, or `OTEL_SERVICE_NAME`), `service.namespace=yuppers`, `service.version` (the crate's), `service.instance.id` (Render's instance ID, else the host name, else random), `deployment.environment` (`DEPLOYMENT_ENVIRONMENT`: `production`, `development` by default) and `vcs.commit` (the build's commit, when known), merged with `OTEL_RESOURCE_ATTRIBUTES`. `OTEL_EXPORTER_OTLP_HEADERS` (`Name=value,Name=value`) goes with every request, which is how a collector behind a Cloudflare Tunnel is passed: `CF-Access-Client-Id=...,CF-Access-Client-Secret=...`. Only `http/json` is spoken: `OTEL_EXPORTER_OTLP_PROTOCOL` set to anything else stops the process at start.

**What is never exported** is what is never logged: the rule for a log line ("Logs" above) is the rule for a span attribute and a log record, decided where each is written. No request or response body, no query string, no header, no token, no one-time code, no email address, no phone number, no payment option. The one field a request's log line has that its span does not is `path`, which can name an exchange; the span has the route's template instead. `backend/tests/telemetry.rs` exports a sign-in, a yup from its draft to an agreement in force with payment options shown, and two people's whole life with text updates, STOP and START replies and a deletion by text, to a stand-in collector, and checks that none of their addresses, numbers, codes, tokens, cookies, invitation tokens or payment options is anywhere in what arrived, in any signal.

**Cost and failure.** Recording is a push onto a bounded buffer (4,096 spans, 4,096 log records); exporting happens on a task of its own, every two seconds, 256 records to a POST, metrics every fifteen seconds. A collector that is slow, down or refusing costs the records of that interval, counted in `yuppers_telemetry_dropped_total`, and one `telemetry export failed` warning (with the signal and the status, never the headers), then at most one every five minutes; `telemetry export recovered` when it answers again. Nothing waits for it: a request is not slower and readiness is not affected, and the test above covers both with a collector that cannot be reached. On SIGTERM each process sends what is buffered, within five seconds, before it exits; `migrate` and `replay-deletions` do the same at their end, so a one-off command's lines arrive too.

**Locally**, against the `grafana/otel-lgtm` image (OTLP on 4318, Grafana on 3000, `admin`/`admin`): `docker run --rm -p 3000:3000 -p 4318:4318 grafana/otel-lgtm`, then `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 cargo run --bin api` and the same for the worker. Grafana's Explore shows the traces (Tempo, service `yuppers-api`), the logs (Loki, `{service_name="yuppers-api"}`) and the metrics (Prometheus, `yuppers_http_requests_total`). **The dashboard**: import `docs/grafana/yuppers.json` (Dashboards → New → Import → upload) into that Grafana, or the owner's; it expects the stack's Prometheus data source (uid `prometheus`) and is built from the metric names above: the funnel for the time range chosen, the previous day's snapshot, request rates and latency, errors, the outbox, the worker's last pass, the messages sent, what the exporter dropped, and, from the stack's Loki data source (uid `loki`), the log lines by level, the warnings and errors, and the latest lines, each opening to its fields and its trace. It filters by `deployment_environment`, so `development` traffic from a laptop and `production` traffic stay apart.

**In production** the endpoint is `https://otel.yuppers.app`, a Cloudflare Tunnel to the owner's stack; [docs/deploy-render.md](deploy-render.md), "Telemetry", says what is set where. The copy of the logs that reaches it holds nothing personal, like the logs themselves; how long the stack keeps it is the stack's own setting.

## What to watch

Starting points; tune them once there is real traffic.

- **Outbox age.** `yuppers_outbox_oldest_pending_age_seconds` above 10 minutes. A message normally goes within one 5-second pass; a failure waits 1, 2, 4 ... minutes, up to an hour, so a single retrying message can legitimately be older, but a rising age with a growing `pending` count means mail is not going out. Check the worker is running, then its `notification not sent` lines for the SMTP reply code or Resend's status.
- **Given up.** `yuppers_outbox_messages{state="given_up"}` above 0. After 8 failed tries a message is left for someone to look at (below); while the provider refuses everything (a wrong key or password, an unverified domain, a spent quota) the tries are not counted, and a message is given up on once it is 24 hours old. `notification given up on` says why (`reason`).
- **Error rate.** `5xx` responses above 1% of `yuppers_http_requests_total` over 5 minutes, or any sustained run of them; then the api's `database error` lines.
- **Latency.** The 95th percentile of `yuppers_http_request_duration_seconds` above 500 ms for a route. The load check (README) measured under 20 ms on a laptop.
- **Pool.** `yuppers_db_pool_in_use` at `yuppers_db_pool_max` for minutes: requests are queuing for connections.
- **Worker alive.** `time() - yuppers_worker_last_pass_timestamp_seconds` above 60, or its scrape failing.
- **Readiness** failing on every copy: the database is unreachable.
- **Text messages, which cost money.** `yuppers_sms_codes_this_hour{result="sent"}` against the cap, and its daily sum against the budget: at the default cap of 50 an hour the service can send at most 1,200 a day, codes and updates together: a code by Twilio Verify costs $0.0083 a text to a US number plus $0.05 once it is approved, so at most about $70 a day if every one were a code used (Twilio's prices of 2026-10-06; docs/deploy-render.md, "Costs"), and an update text the Messages API's per-message price plus carrier fees. International numbers cost several times more. Any `refused` means people asking for a code by phone were turned away; `yuppers_sms_codes_refused_this_hour` says why. `hourly_cap`: either real demand above the cap, which is the cue to raise `SMS_MAX_PER_HOUR`, or someone sending codes to numbers that are not theirs (SMS pumping), which the provider's fraud tools and its geographic permissions (allow only the countries you serve) are for. `prefix_cap` from one or two area codes at a time is more likely the latter; spread over many, demand, and the cue to raise `SMS_MAX_PER_PREFIX_PER_HOUR`. `country`: people abroad trying to sign in by phone, or someone trying numbers the service will not text; neither costs anything. `failed` above a few in an hour: the provider is refusing; its error code is in the api's log.
- **Push.** `yuppers_push_deliveries_total{result="given_up"}` growing, or `yuppers_push_receipt_checks_total{result="error"}` most of the time: Expo is refusing or unreachable; the error code is in the worker's log. A jump in `yuppers_push_devices_removed_total` after a release can mean the app's project or credentials changed and every token stopped working.
- **Wallet passes**, once on: `wallet pass update given up on` in the worker's log, or rows in `wallet_pass` with `update_status = 'FAILED'` (`last_error` says why; APNs refusing the certificate means it expired or was revoked). And the pass type certificate: `yuppers_wallet_cert_expiry_seconds` below 2,592,000 (30 days) is the cue to renew, and below 604,800 (7 days) is urgent: at 0 every Apple pass stops updating and no new one can be added, though the api and the worker keep running everything else. Renewing takes a new certificate from Apple's developer account and a restart (docs/wallet.md).
- **Reports.** `yuppers_reports_oldest_open_age_seconds` above 72,000 (20 hours): a report is close to its 24 hours without review; tell whoever is on call. Any open report on a day nobody is named is the same alarm.
- **Refusals** are not errors: `429` is a limit working (too many codes asked for, too many wrong guesses), and `4xx` in general is a person or a client being told no. Watch them for sudden jumps, not as failures.
- **Telemetry.** `yuppers_telemetry_dropped_total` growing, or `telemetry export failed` in a log: the collector is down, the tunnel's token has changed, or the service is logging far more than usual. The service itself is unaffected; only the dashboards go stale.
- **The funnel.** `yuppers_daily_yups_created` at 0 on a day with sign-ins, or `yuppers_invitations_claimed_total` not moving while `yuppers_invitations_shared_total` does: people share links nobody opens, which is a product question, not an outage, but one the dashboard asks every morning.

## Reviewing reports

Every abuse report is read by a reviewer within 24 hours, every day of the week (`DESIGN.md` §9). Reviewers are named by the owner and use the review screen at `{WEB_ORIGIN}/staff`, which nothing in the app links to and which is "not found" to everyone else. The code is `backend/src/review.rs`; README, "Help", and the help page on blocking and reporting say what a person is told.

**Naming reviewers.** The person signs in to the app once, normally, so that the account exists and has a name. Then the owner runs the `staff` command with the schema owner's connection, the same one `migrate` uses; the service's own role can read the list of reviewers and cannot change it, so no request to the API can make anyone a reviewer.

```sh
MIGRATION_DATABASE_URL=postgres://exchange:...@db.internal:5432/yuppers \
  cargo run --bin staff -- grant rita@example.com     # an email address, a phone number or an account ID
cargo run --bin staff -- list                         # ID, address, name, status, since when
cargo run --bin staff -- revoke rita@example.com
```

In the image it is `/usr/local/bin/staff`. Each grant and revoke is written to the review history as the owner's. A suspended or deleted account cannot be made a reviewer. A reviewer's sign-in must be recent: the staff screen and every `/v1/staff/` path want a one-time code entered within the last 12 hours (`STAFF_SIGN_IN_MAX_AGE`, a placeholder), and say "sign in again" (`SESSION_TOO_OLD`) after that. Revoke a reviewer who leaves; deleting their account also ends it, since a deleted account has no session.

**The daily routine.** The reviewer on call that day:

1. Opens `/staff` and signs in if asked. The queue lists every open report, oldest first, with how long it has waited, its reason and its yup's code; one older than 24 hours is outlined and tagged "Overdue". What the reporter wrote, and who anyone is, shows only once a report is opened, which is recorded.
2. Opens each report. It shows the reason, what the reporter wrote, who reported it (always an account: reporting needs signing in, through an invitation link as well; only a report from before that rule shows "someone with the invitation link, not signed in"), the person reported and their account's status, the reported yup's whole record (every version sent, every signature, the history with every note), the other reports about the same yup, and what review has done so far. Opening a report is itself recorded. A yup can be read here only while a report about it is open: once it is resolved, the report shows nothing more.
3. Decides, with a note saying why (required for everything but dismissing):
   - **Dismiss.** Nothing changes for anyone.
   - **Hide content.** For the person reported, and only them, everything written in that yup (its terms, each item's description, completion criteria and unit of quantity, the messages sent with versions, and every note, reason and statement in its history) reads "Hidden by review", in their language, in the yup, its history and their copy of the record, and a notice says so. Their copy of the record leaves out the signed document of each version and gives a redacted copy, `redacted`, in its place, since a document with text replaced no longer matches what was signed. They can no longer sign or send terms there (`CONTENT_HIDDEN`), but can still decline, withdraw, mark items and close, so the yup can still end. Names and amounts stay. Nothing stored changes, and the reporter's view does not change. It protects what the reporter wrote there (an address, a phone number) from the person reported.
   - **Suspend account.** The person reported is signed out of every session, their devices stop getting notifications, and signing in is refused (`ACCOUNT_SUSPENDED`). Nothing is sent to them, and their Wallet passes stop updating. Nothing new can bind them: the invitation links they sent that nobody took stop working (and stay dead if the suspension is lifted), an offer or amendment of theirs still waiting to be signed is withdrawn in their name, and where they had opened someone's link and were not yet confirmed they leave. The other party sees an ordinary withdrawal, departure or dead link, nothing about a suspension. Agreements in force are left as they are; the other party can still end them. Offers sent to them are left to lapse. A reviewer cannot suspend another reviewer (`SUBJECT_IS_REVIEWER`): the owner first runs `staff revoke` for that reviewer, and the report can then be resolved by anyone else.
   - **Hide and suspend.** Both.
4. A report is resolved once: who, when, the outcome and the note are set together and the database refuses to change them. A second report about the same yup is reviewed on its own.

Below the queue are the suspended accounts and the hidden content, each with a way to undo it ("Lift suspension", "Show again"), again with a required note. Lifting a suspension lets the person sign in again; the sessions they had stay ended, and so does what the suspension ended. A reviewer's suspension is the owner's to lift. Neither the reporter nor the person reported is told the outcome, though the person reported will notice a suspension or hidden content.

**Reports a reviewer is part of.** A reviewer never sees or decides a report they made, one about them, or one about a yup in which they hold or once held a place (a claimant since removed included). It is not in their queue, and opening or resolving it answers exactly as for a report that does not exist, with nothing read or recorded. The same goes for the suspension or hidden content such a report led to, and for their own. Another reviewer handles it; with only one reviewer, the owner names a second for it.

Requests are limited per reviewer: 300 reports opened, 60 actions and 600 lists read (the queue, the suspensions and the hidden content together) an hour (`VIEWS_PER_HOUR`, `ACTIONS_PER_HOUR`, `LISTS_PER_HOUR`, placeholders), then `TOO_MANY_REQUESTS`. Requests made at the same moment take turns, so they cannot together pass a limit.

**The audit history.** Every report opened and every action, and every grant and revoke from the command line, is a row in `review_event`: the reviewer's account (none for the owner's command line: naming and removing reviewers, and a suspension lifted by replaying the deletion log, "Replaying deletions"), the time, the action, the report, yup and account it concerns, and the note. The service cannot change or remove a row, nor write one with no reviewer (migration 0019): only the schema owner can, which the replay does through a function of the owner's that lifts a suspension only for an account deleted in the same transaction. Triggers protect the rows from the application, not from the owner, who can disable a trigger; keep the owner's credentials as narrow as the backups. A later look at the same matter, such as lifting a suspension, is a new row tied to the same report, and the report's resolution stays as it was. A report's page shows its latest 200 entries; for anything else, as the owner:

```sql
SELECT occurred_at, action, staff_account_id, report_id, exchange_id, account_id, note
FROM review_event ORDER BY id DESC LIMIT 100;
```

**The alert.** When a report arrives, every reviewer with an email address gets "A report is waiting for review", through the outbox like every other email (so it needs the worker running), with a link to `/staff` and nothing about the report. A reviewer already waiting for one is not sent another. Watch the queue as well: `yuppers_reports_open` and `yuppers_reports_oldest_open_age_seconds` ("Metrics").

## When the worker is down

Requests keep working: people can sign in, sign and record deliveries. What stops:

- **Notification emails and push notifications** queue in the outbox; nothing is lost. So does the alert to reviewers that a report is waiting: check `/staff` by hand until the worker is back. One-time codes, by email or text message, are sent by the api itself, so sign-in is unaffected.
- **Timers**: unanswered revisions do not expire, close requests do not lapse into closing as unresolved, idle exchanges are not prompted or closed.
- **Reminders** of contributions due soon or overdue are not sent.
- **Wallet passes** are not updated; each catches up with the latest face when the worker is back.
- **Purges**: network addresses and user agents older than 90 days (`DESIGN.md` §14), old sign-in counts, ended sessions, old idempotency keys and finished notifications are not removed, so a long outage keeps personal data past its retention period.

To recover:

1. Look at its last log lines and exit code. A worker that will not start says why (a missing setting, wording that cannot be read, a metrics address in use); one that cannot reach the database keeps running and logs `timers failed` with `database error: pool timed out` or similar.
2. Restart it. On its first pass it applies every timer whose time has passed, queues the reminders still due, purges what is past retention, then drains the outbox at up to 100 messages per pass (each pass stops taking new messages after 20 seconds, so a slow email provider cannot hold up the timers) until `pending` is back near 0.
3. One worker is the normal setup. Delivering is safe from several at once (each message is locked while it is sent), so a second copy started during a handover does no harm.
4. If messages were given up on while email was failing, send them again once it works. They hold no personal data, only which notice and which exchange:

   ```sql
   -- As the owner. Shows what failed and why (the SMTP reply code or Resend's status only).
   SELECT id, exchange_id, attempts, last_error, created_at FROM outbox
   WHERE completed_at IS NULL AND attempts >= 8 ORDER BY id;

   -- Gives them another round of tries.
   UPDATE outbox SET attempts = 0, available_at = now()
   WHERE completed_at IS NULL AND attempts >= 8;
   ```

   A reminder that is no longer true is dropped when it is retried, not sent.

## Backups

`scripts/backup.sh` writes the whole database to one file in `pg_dump`'s custom format: schema, data, and the grants to `exchange_app`. It runs against the live database without stopping anything; the file is one consistent snapshot.

```sh
MIGRATION_DATABASE_URL=postgres://exchange:...@db.internal:5432/yuppers \
  scripts/backup.sh /backups/yuppers-$(date -u +%Y%m%d).dump
```

- With no file named it writes `yuppers-<UTC time>.dump` in the current directory. It refuses to replace a file that is already there, unless given `--force`, and refuses a directory even then. It writes the dump under a fresh name beside the target first, created with `mktemp` so that a link someone left in the directory is never followed, readable by the running user alone, and moves it into place only once `pg_restore` can read it.
- **The deletion log beside it.** Each backup gets a companion, `<backup>.deletions`: the deletion log (which accounts were deleted, and when; nothing else), exported by `scripts/export-deletions.sh` just after the dump and under the same rules (never replacing a file without `--force`, readable by the running user alone, put in place before the dump is). Because it is taken after the dump, it holds every deletion the dump holds and perhaps a few more, which is the safe way round. Keep each with its backup and copy them together; "Restoring" says which one a restore uses.
- It needs `pg_dump`, `pg_restore` and `psql` of the server's major version or newer (PostgreSQL 17); the image does not contain them. Run it from a small scheduled job in the same network, for example a `postgres:17` container with the repository's `scripts/` mounted, or set `PG_BIN` to where the tools are.
- A connection string with a password in it is visible to other users of the same machine while the command runs. On a shared machine leave the password out of the URL and put it in `PGPASSWORD` or a `.pgpass` file.
- Managed databases take their own snapshots and point-in-time recovery; keep those on. These files are the copy that does not depend on the provider, that can be restored anywhere, and that the drill below proves.
- **What the file holds**: every account's email address and phone number, encrypted under `CONTACT_DATA_KEY` (a backup made before that release holds them as they were), every agreement, signature and the network addresses recorded with signatures. Encrypt it at rest, keep it where access is as narrow as the database's, and never in the repository (`.gitignore` refuses `*.dump`). Keep the key apart from the backups, and keep it as long as they are kept ("Contact data key").
- **How long to keep them**: the service forgets network metadata after 90 days and deleted accounts' contact details at once (`DESIGN.md` §14); a backup keeps whatever it held when it was taken. Keep backups no longer than the retention the privacy policy states (`/privacy#how-long-we-keep-it`: up to 7 days for Render's own recovery copies, and that we keep no copies of our own: before running this on a schedule, change the policy to say where these files are kept and for how long, "Decisions still open"), and see "Restoring" for what a restore brings back.

## Restoring

`scripts/restore.sh` restores a backup into a database, in one transaction: it completes or changes nothing.

```sh
# As the owner, on the server to restore to. The application role must exist
# there first, with a password of its own; roles are not in a backup.
psql "$ADMIN_URL" -c "CREATE ROLE exchange_app LOGIN PASSWORD '...'"   # if it does not exist
psql "$ADMIN_URL" -c "CREATE DATABASE yuppers_restored OWNER exchange"

scripts/restore.sh -d postgres://exchange:...@db.internal:5432/yuppers_restored yuppers.dump

# Then, before the api and the worker start on it: migrate, and replay the
# newest deletion log ("Replaying deletions" below). Not optional. Both need
# the key the backup was made under ("Contact data key").
export CONTACT_DATA_KEY=...
MIGRATION_DATABASE_URL=postgres://exchange:...@db.internal:5432/yuppers_restored migrate
scripts/replay-deletions.sh -d postgres://exchange_app:...@db.internal:5432/yuppers_restored deletions.txt
```

- `APP_ROLE` names the application role if it is not `exchange_app`. It reaches the server only as a value psql quotes (`:'app_role'`), never pasted into a query.
- It refuses a database that already holds tables. `--overwrite` replaces every object the backup holds instead, but leaves alone anything it does not, so a new, empty database is the safe target; point `DATABASE_URL` and `MIGRATION_DATABASE_URL` at it when it is ready.
- **Grants**: the backup carries the grants the migrations gave `exchange_app`, and the restore applies them as they were. Afterwards `exchange_app` holds exactly those: `SELECT` and `INSERT` on the five append-only tables (`revision`, `revision_attachment`, `contribution_snapshot`, `acceptance`, `exchange_event`) and `contribution_reminder`; `SELECT`, `INSERT`, `UPDATE` on the current-state tables; `SELECT` only on `slot_holding`; `SELECT`, `INSERT`, `UPDATE`, `DELETE` on working data (drafts, blocks, idempotency keys, the outbox, network metadata, codes, sessions, sign-in counts, devices registered for push and their push tickets); and for review, `SELECT` only on `staff_member`, `SELECT` and `INSERT` on `review_event`, and `SELECT`, `INSERT`, `DELETE` on `hidden_content`. `backend/tests/schema.rs` asserts these. Do not restore with `--no-acl` or as another application role: the service would then have no rights, or the wrong ones.
- **Owner**: every object belongs to the role that ran the restore, whatever the owner was called where the backup was made, so a backup moves between servers whose owner roles have different names.
- **Triggers**: the append-only triggers come back with their tables and refuse `UPDATE`, `DELETE` and `TRUNCATE` for every role again. The restore loads rows before creating triggers, so loading history does not trip them and the stamps the database writes (slot holdings) are restored as stored, not recomputed.
- **Checked afterwards**: every grant `exchange_app` holds and every trigger (not only the append-only ones; the review, deletion log and restore mark tables of migrations 0014 to 0019 too) must be exactly what the migrations the backup holds give: nothing missing, nothing more. The list is `scripts/restore-inventory.txt`, each line with the migration it comes with; `backend/tests/inventory.rs` applies the migrations one at a time and checks the file after each. The script prints any difference and fails.
- **The mark**: the restored database is marked as waiting for its deletion log to be replayed (`restore_marker`, migration 0018). The api and the worker refuse to start on it while the mark stands, saying to run `replay-deletions`; `migrate` runs (the replay needs the current schema) and warns. `replay-deletions` clears the mark once every account in the log is deleted, and the history of the mark keeps when and by which role. In an emergency where it is certain that no account was deleted since the backup, `restore.sh --no-replay-needed` marks the copy as needing no replay instead; that is recorded in the same history.
- **Migrations**: the backup includes `_sqlx_migrations`, so `migrate` against the restored database applies only what is newer than the backup. Restore with the release that made the backup or a newer one, never an older one.
- **What a restore brings back**: everything as it was at the backup. Accounts deleted since then return with their contact details and their sessions, until the deletion log is replayed (below), which is a required step of every restore. Network metadata purged since then is back until the worker's next pass purges it again.

### Replaying deletions

Every deletion adds the account's ID and the time to the deletion log (`deletion_log`, migration 0015), in the transaction that deletes it (`backend/src/deletion.rs`). The log is in every backup, and also exported beside it, because the log a restore needs is the newest one there is, not the one inside the backup being restored.

**Which log to replay**, in this order:

1. **The live database's, if it can still be reached** (a bad migration, rows destroyed by mistake, a copy restored for a drill). Export it before restoring anything, and keep the file:

   ```sh
   scripts/export-deletions.sh -d postgres://exchange:...@db.internal:5432/yuppers deletions-$(date -u +%Y%m%dT%H%M%SZ).txt
   ```

2. **Otherwise, the `.deletions` file of the newest backup there is**, whichever backup is being restored. Restoring last week's backup because last night's is damaged still replays last night's log, which the backup next to it would be missing. The log is cumulative and never pruned, so the newest file holds every deletion of every older one.

What neither can hold, when the live database is lost, is a deletion made after the newest backup. That deletion is lost with everything else written since that backup, so the gap is the time between backups. Closing it takes a log kept outside the database as deletions happen, which this build does not have (see "Decisions still open"). Two things narrow the gap without one. Where the provider's point-in-time recovery reaches past the newest backup, restore it to a scratch database and export that log. And `export-deletions.sh` is cheap enough to run hourly to storage of its own, apart from the backups.

**Replaying.** Run it after `restore.sh` and `migrate`, and before the api and the worker start on the restored database, connected as the application role:

```sh
scripts/replay-deletions.sh -d postgres://exchange_app:...@db.internal:5432/yuppers_restored deletions.txt
# in the image: DATABASE_URL=... /usr/local/bin/replay-deletions deletions.txt
```

- It deletes each account through the service's own deletion (the `replay-deletions` binary, which calls `deletion::replay`), not through SQL, so every rule runs again. Sessions end, devices, codes and working data go, the email address and phone number are freed, Wallet passes are voided, and open exchanges are left through the rules in the account's name.
- **What it reports**: a line for each account, then a count. *LEFT ALONE* means the log's time for the account is before the copy says it was created, or last suspended: that line cannot be a deletion of the account as the copy holds it, so nothing is done; find out where the log came from, and remove the line once satisfied it is wrong (it leaves the replay incomplete, and the mark in place, until then). *deleted again* means it was live in the copy and is now deleted. *already deleted* means the backup already had it deleted. *not in this database* means it was made after the backup. *deleted again ... it was suspended here* means the copy had it suspended: a suspended account cannot delete itself, so a reviewer lifted that suspension after the backup and the person then deleted their account. The replay does the same again, in one transaction: it lifts the suspension and deletes the account. The lifting is written to the review history as the owner's (no reviewer), tied to the report that led to the suspension, with the note "Lifted to replay a deletion" and the time from the log; the reviewer's own lifting, with their note, was lost with the restore like everything else after the backup. The count says how many of those deleted again were suspended; tell the reviewers, whose list of suspended accounts no longer shows them. *FAILED* means the database refused or was busy, and changed nothing for that account, a suspension included; replay again. It exits non-zero if any account was left undeleted or left alone, and with status 2, changing nothing, if the file is damaged. When it exits zero, it has cleared the restore mark, and says so.
- **Combined accounts** (README, "Combining accounts"). Combining one account into another adds a line too, with the account it went into and `EMAIL` or `PHONE`, the kind of identifier they were combined by: `B<TAB>time<TAB>A<TAB>PHONE`. Replaying it combines the two again through the service's own code (`combine::replay`), in log order, so a later deletion of A finds B already in it. Nobody is told again. *combined into ... again* means both were live in the copy and apart; *already combined* that the copy had it so; where the account it went into is not in the copy (made after the backup), the replay follows that account through the later lines of the log: where it was deleted, the account is deleted at that time (*deleted again ... as ..., which it was combined into ..., was deleted then*); where it was combined into one the copy holds, the account is combined into that one (*combined into ... again (... through ...)*); *UNRESOLVED* that no later line says, so the account is left as it is, live, and the replay is incomplete until someone finds the log that does or decides by hand; *LEFT ALONE* that the copy does not allow it (one of them deleted or suspended there, a reviewer, or the two sharing a yup), which leaves the replay incomplete until looked into. A log exported before this release has no such lines and reads as before.
- **Running it again is harmless**: every account it already deleted is *already deleted* the second time. After a failure, run the same file again.
- **What it cannot reproduce exactly**: the events and notices of leaving an exchange carry the time of the replay, not of the deletion; the deletion log keeps the original time. The other party of an exchange the account was still in is told again that the person left, if they had been told before the database was lost.

## The restore drill

A backup is only known to work once it has been restored. Do this on a schedule, monthly at least, and after any change to the schema or the database's settings, on a scratch database that is not the one the service uses:

1. Take a backup with `scripts/backup.sh`, or pick last night's.
2. Create an empty database and restore into it with `scripts/restore.sh`. Note how long it took: that, plus replaying deletions and starting the processes, is the time to recover.
3. Replay the newest deletion log into it with `scripts/replay-deletions.sh` ("Replaying deletions" above), and read its report.
4. Before replaying, or on a second copy, `scripts/check-restore.sh SOURCE_URL RESTORED_URL` compares the two: migrations, every privilege of `exchange_app`, every trigger, each table's row count and a digest of its rows. Against the live database the counts and digests differ by what has been written since the backup; against a database restored from the same file they must match exactly.
5. `scripts/check-restored-record.sh RESTORED_URL RESTORED_APP_URL` starts an api on the copy, reads the oldest agreement in force through the API, and checks that its stored terms still reproduce the hash that was signed. It writes a session into the copy and deletes it again when it finishes, whether it passed or not; still, never point it at the database the service uses. The session's token is never on a command line: psql reads it on standard input, and curl from a header file only the running user can read.
6. Drop the scratch database and the backup copy you made for it.

CI runs the same steps on every change (the `Backup and restore` job). It fills a database through the API with the load check and backs it up. It checks that `backup.sh` will replace neither that file nor a deletion log without `--force`, and that `restore.sh` refuses the non-empty source. It restores into a new database and compares the two with `check-restore.sh`, then restores again with `--overwrite` and compares again. Then the source stands for the live database: one person deletes their account through the API, with codes read from the log, and a newer backup and the live log are exported. The earlier backup is restored into another database, where that person's account is back with its address and sessions. The newer backup's deletion log is replayed, and the account is deleted again: no address, no sessions, no devices, nobody holding the address, and the log's original time. Replaying a second time does nothing. Last, `backend/tests/schema.rs` runs against both copies, and a signed agreement is read back from each.

## Contact data key

`CONTACT_DATA_KEY` encrypts every email address and phone number the database holds, and the payment options people save (README, "Contact details at rest", "Payment options"): 32 random bytes in base64, `openssl rand -base64 32`, apart from `APP_SECRET`. `migrate`, the api, the worker, `staff`, `contact-data` and `replay-deletions` need it. The api and the worker refuse to start if it does not decrypt the database's blind-index key (`contact_key`, migration 0025), or if any value stored is under a key that is neither it nor `CONTACT_DATA_KEY_PREVIOUS`; while the database cannot be reached they wait for it. Every process refuses a key that decodes to printable text (typed, not generated), and, wherever `WEB_ORIGIN` is not this machine, the keys published in the repository for development (`.env.example`), the tests and CI. No process ever writes it to a log or an error.

**Keep a copy outside the platform**, in a password manager, from the moment it is made. Losing it loses every address and number stored, in the database and in every backup made since: nobody could be emailed or texted, and nobody could sign in by the address or number they used. Keep any key it replaced, too, for as long as backups made under that key are kept.

**Where things stand.** `contact-data status`, as the owner (`MIGRATION_DATABASE_URL`; in the image, `/usr/local/bin/contact-data status`, run as a one-off job on Render), prints for each column that holds encrypted values how many are under the current key and how many under another, and whether `CONTACT_DATA_KEY_PREVIOUS` is still needed.

**Rotating it**, if it may have leaked, or on a schedule:

1. Make a new key and save it.
2. On every process at once (on Render, in `yuppers-secrets`): `CONTACT_DATA_KEY_PREVIOUS` = the current key, `CONTACT_DATA_KEY` = the new one. Restart or redeploy. New values are now encrypted under the new key, and the old one only decrypts.
3. Run `contact-data rotate` as the owner. It re-encrypts every value not under the new key, and the blind-index key with them, in batches while the service runs, and prints the counts. Running it again changes nothing. It exits 1, leaving them as they were, if any values decrypt under neither key.
4. Once a run says that every value is under `CONTACT_DATA_KEY`, remove `CONTACT_DATA_KEY_PREVIOUS` and restart. A process started without the previous key while anything is still under it refuses to start, rather than fail later.

**The blind indexes do not rotate.** The index key is derived once, from the first key, and kept in the database encrypted under whichever key is current; rotating re-encrypts it, so no index changes, every lookup and join stays exact, and nothing waits on step 3. An index key that changed with the key would leave, until every row was recomputed, an opt-out under one key and the number it stops under another: a text to someone who replied STOP. The cost: a leaked key leaks the index key, and whoever also has a copy of the database could test guessed addresses and numbers against the indexes (all US numbers can be tried). Rotating protects the encrypted values in later copies, not the indexes. Recomputing the indexes under a new index key needs every value decrypted and nothing writing meanwhile, which this build does not offer; treat a leak of the key as a leak of the contact details.

**Backups and restores.** A backup holds the values and the index key encrypted, never the key. Restoring one needs the key it was made under, set as `CONTACT_DATA_KEY` (or as `CONTACT_DATA_KEY_PREVIOUS` beside a newer one) for `migrate`, `replay-deletions` and the api. The restore drill's `check-restored-record.sh` starts an api on the copy with it, which proves that the copy kept outside the platform still opens the backups.

**The release that added it** started from an empty database. Its migration (0025) drops the plaintext columns rather than converting them, and refuses to run, changing nothing, on a database where any of those tables has a row; docs/deploy-render.md, "Contact data key", says how the live database was emptied first. A database restored from a backup made before that release cannot be migrated past it. From now on, any change to how contact details are stored, on a database with people in it, needs a migration that converts what is there.

## Rotating `APP_SECRET`

`APP_SECRET` keys the hashes of one-time codes and the hashes that sign-in limits are counted under. It does not touch sessions, invitation links, signatures or content hashes, and the worker does not use it.

To rotate it, set the new value and restart every api copy together (a rolling restart works, but while old and new copies both run, a code sent by one is refused by the other). What it invalidates:

- **Codes in flight**: every sign-in, deletion and new-identifier code already sent stops working. People ask for a new one; nobody is signed out.
- **Sign-in limit counts**: the counts for the current hour and day start again from zero, since they are kept under the old secret's hashes. The old rows are removed by the worker within two days.
- **Wallet passes** are not affected: their tokens and download links are random, stored as hashes, and keyed by nothing.

Rotate it if it may have leaked: anyone with it and a copy of the database could test guesses at codes offline, and could tell which identifiers a count belongs to.

## Upgrading

1. Take a backup.
2. Run `migrate` from the new image.
3. Roll out the worker and the api from the new image.
4. Check that `/v1/meta` names the new commit ("What is deployed").

Between steps 2 and 3 the old processes run against the new schema for a few minutes. A release whose migration the old processes cannot live with says so in its notes, and then the old processes are stopped before step 2.

## Decisions still open

- How long backups are kept, and where, given the retention in `DESIGN.md` §14.
- **Deletions after the newest backup, when the live database is lost.** The deletion log covers every deletion up to the newest backup ("Replaying deletions"); a deletion made after it is lost with the database. Closing that gap needs a log kept outside the database as deletions happen, for example the worker appending each new line of the log to object storage. Until that exists, the gap is the time between backups, narrowed by point-in-time recovery or by exporting the log hourly.
- **How long the deletion log is kept.** It holds account IDs and times only, and is never pruned, because a restore of any backup still kept needs every deletion since. It could be pruned of deletions older than the oldest backup kept, once backup retention is decided (above).- The alert thresholds above are placeholders until there is real traffic.
- **Review's numbers**: the 12-hour sign-in for reviewers, 300 reports opened and 60 actions an hour, and the 1,000-character note, are placeholders.
- **Who is on call.** The design names a reviewer each day, weekends included, in English and Spanish (`DESIGN.md` §9). The rota is not in the product; the alert goes to every reviewer.
- **Push through Expo, or straight to Apple and Google.** `DESIGN.md` §13.1 decides that app push goes directly from the backend to Apple's and Google's services, with no third party. This build sends through Expo's push service instead, which holds the APNs key and FCM credentials, sees each token and the generic text, and needs nothing from Apple or Google on the service. Going direct means an APNs adapter (HTTP/2, a signed JWT per hour) and an FCM v1 adapter (OAuth with a service account), and the app registering device tokens (`getDevicePushTokenAsync`) instead of Expo tokens; the `PushSender` interface and the `device.service` column are where they would go.
- **Email and push both.** Someone with the app and an email address gets both for each notice, one per channel, as §12 reads. Sending the email only when the push was not delivered or not opened within some time would halve that, at the cost of the email's detail; the outbox could hold an email back for that time.
- **The SMS caps**, 50 an hour and 10 an hour per area code, are placeholders like the sign-in limits.
- **`+1` is more than the US.** `SMS_ALLOWED_REGIONS` refuses every `+1` area code listed as Canada's, another country's or territory's, or not a place (toll-free, personal, service codes), and takes the rest as the US (`backend/src/nanp.rs`). The lists come from NANPA's assignments and need updating when a code opens outside the US, which is rare; a new US overlay needs nothing. The provider's geographic permissions back it up.
