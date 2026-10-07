# Wallet passes

A party to a yup can keep it in their phone's wallet: Apple Wallet on an iPhone, Google Wallet on Android (`DESIGN.md` §11). The pass is their view of that one exchange, kept up to date as it changes, with a link back to it. It is optional, it never lets anyone in by itself, and it holds nothing from the agreement.

Everything is built and tested with throwaway credentials. Nothing has been issued to a real device: that needs the Apple and Google accounts, and then the steps under "Once the accounts exist". Until a deployment sets a platform's settings, that platform is off and nobody sees anything of it.

The code is `backend/src/wallet/` (with the routes in `backend/src/http/wallet.rs`), `packages/shared/src/wallet.ts`, and the button in each app (`apps/web/src/components/WalletButton.tsx`, `apps/mobile/src/components/WalletButton.tsx`).

## What a pass shows

A wallet shows a pass without the phone being unlocked, and a pass that has reached a device cannot be taken back, so the face is held to the lock-screen rule of `DESIGN.md` §11 and §12: generic words only, nothing from the terms.

| On the pass | What it is |
|---|---|
| Product | Yuppers, with the mark |
| Status (the large field) | By default *In force* while the agreement is in force; once closed, *Completed*, *Ended by agreement* or *Closed*, with the day it closed. With the detailed status line (below), how the agreement stands for the person holding the pass: *Waiting for you* (the other party marked something delivered to them, proposed a change, proposed ending, or asked to close), *Disputed*, *Overdue*, *Due soon* (within the reminders' lead, two days), *In force*; once closed, *Completed*, *Ended by agreement* or *Closed*, with the day it closed |
| Reference | The exchange's display code |
| Next due | Only with the detailed status line: the earliest date of a contribution still open, while it is in force |
| Still to do | How many contributions are still open |
| With | The other party's alias, only when one is set. Their name, as the agreement writes it, is never used in its place, so today, with no way to set an alias, this line does not appear |
| Back | A link to the exchange, which asks its reader to sign in, and one sentence saying the terms stay in Yuppers |

Never on it: what anyone gives or pays, an amount, a description, the terms, a note, a name. The labels and words come from the wording files, `wallet` in `packages/shared/wording/{language}.json`, in the account's language (English and Spanish today); a date is spelled from the same file, so both wallets show the same. The face is one pure function, `wallet::pass::render`, from the exchange as its party sees it, and both platforms draw that one model (`backend/src/wallet/pass/tests.rs` checks it in both languages, and that nothing from the agreement gets onto it).

**Silent updates.** A change to a pass never raises an alert of its own: Apple's fields carry no `changeMessage`, and nothing asks Google to notify. The person is told about the event by its notification, once (§12, no duplicates). §11 makes status on the lock screen something a person opts into; nothing offers that choice yet, so only the default is built, and a `changeMessage` on the status field is where the option would go.

**How much the status line says.** A wallet shows the face on the lock screen, so the face is neutral by default (`DESIGN.md` §11, the owner's decision of 3 October 2026): an agreement in force reads *In force* whatever presses, and once closed *Completed*, *Ended by agreement* or *Closed*; there is no *Waiting for you*, *Disputed*, *Overdue* or *Due soon*, and no next due date. How many contributions are still open stays on it. `WALLET_STATUS_ON_FACE=detailed` puts the detail back for every pass the deployment issues; it stays available as a deployment setting for now.

*Planned, a later change:* a per-person opt-in to the detailed face, in place of the deployment-wide setting. It would be a preference on the account (a column beside its language, set from the account screen), read when a face is drawn: `wallet::store` already loads the holder's language for each pass it renders and would load the preference with it, and pass it to `wallet::pass::render` as `PassContext::status_on_face` in place of the deployment's value. Changing the preference would mark that account's passes for an update, as a change to an exchange does, so the faces already on devices follow it. The rendering itself needs no change: it already draws both faces from the one model.

**Revocation.** When an account is deleted its passes are voided as part of the deletion (`backend/src/deletion.rs`): each is updated once to a void face that says only that it is no longer in use, without the reference, the link or anything else, then never again. Apple's pass is marked `voided`; Google's object goes `INACTIVE`. Apple's protocol takes two steps, a push that wakes the device and then the device asking which of its passes changed before it fetches one, so a voided pass's device registrations are kept until the device has been told, in that list, that the pass changed, and removed a day later (time enough to fetch the void face), or 30 days after the voiding if the device never asks; a device unregistering removes its own at once. A device that was offline keeps what it last showed, which is why the face is minimal to begin with.

**Suspension.** A suspended account's passes are frozen: no change marks them, the worker sends nothing for one already marked, devices are not told they changed and are answered 304 (or 401 to a device with no copy of its own), and no new device registers. They are not voided, and they resume with the next change if the account is reinstated. Whether suspending an account should void its passes, as deleting it does, is a product decision left to the owner.

## How it works

**Endpoints**, for a party to the exchange once something has been agreed (anyone else is told the exchange does not exist; before agreement, `ACTION_NOT_ALLOWED`; a platform that is not configured, `WALLET_UNAVAILABLE`, 404):

| | |
|---|---|
| `POST /v1/exchanges/{id}/wallet/apple` | The signed `.pkpass` itself (`application/vnd.apple.pkpass`) |
| `POST /v1/exchanges/{id}/wallet/apple/link` | A link that downloads that pass once, within ten minutes, without a session: `{web origin}/v1/wallet/apple/pass?token=…`. The token is random and only its SHA-256 is stored (`wallet_download_link`); using it uses it up, and a second use is answered as an expired or unknown link is, `NOT_FOUND`. It travels in the query string, which the service never logs and the reverse proxy must not log either (docs/operations.md, "Logs"). This is what the apps use: Safari opening the link is what adds the pass to Wallet |
| `POST /v1/exchanges/{id}/wallet/google` | Creates the pass's object at Google (or brings it up to date), then answers the "Save to Google Wallet" link, `https://pay.google.com/gp/v/save/<JWT>`, which names only that object |
| `GET /v1/meta` | `wallet_platforms`: the platforms configured, `APPLE`, `GOOGLE`, or none |

Each party gets one pass per exchange per platform, the same one every time they ask, and may ask ten times an hour per pass (`WalletRules`), which bounds how often a script can make the service sign.

**Apple's pass web service**, under the `webServiceURL` a pass names, `{web origin}/v1/wallet/apple`: a device registers for a pass's updates and unregisters, asks which of its passes changed since a tag, fetches the latest pass with `If-Modified-Since` (304 when unchanged), and sends its logs. A device proves it holds a pass with one of the pass's authentication tokens (`Authorization: ApplePass <token>`). A token is random and new each time the pass is handed out (by the endpoint or a download link); only its SHA-256 is stored (`wallet_auth_token`), and the latest three stay valid, so copies of the pass on a person's other devices keep updating while a token handed out long ago stops working. A device fetching the latest pass gets it with the token it asked with. A pass is registered on at most 20 devices: a new device past that takes the place of the one heard from longest ago, and a pass takes at most 10 new devices an hour (429 past that). The device log endpoint, which anyone may call, writes at debug level only, at most five lines of 300 characters from a body of at most 8 KiB (413 past that), and takes ten requests a minute per address (429 past that). The `.pkpass` is sent with `Cache-Control: no-store`. These routes are Apple's protocol, not part of the API description the clients are generated from.

**The pass.** `generic` style: a store card is for balances and points, and an event ticket, a boarding pass or a coupon each imply something this is not; generic shows the status large and the reference and counts beneath it, and matches Google's generic pass. Its files are `pass.json`, the icon and logo at three scales (drawn from the earlier Yuppers mark into `backend/assets/wallet/` by an earlier version of `apps/mobile/scripts/make-icons.mjs`, and kept until the pass's colours follow the new design), `manifest.json` with each file's SHA-1, and `signature`: a detached CMS `SignedData` over the manifest, SHA-256 with RSA, carrying the pass type certificate and the WWDR intermediate, with the content type, signing time and digest as signed attributes. The archive's entries are stored, not compressed.

**Google.** A generic class, `{issuer}.yuppers_agreement`, and one generic object per pass, `{issuer}.{serial}`. When the save link is asked for, the api creates the class (once per process) and the object through the Google Wallet API, or patches the object if it exists, with the service account it already holds; the link's JWT (RS256, signed with the service account's key, `aud` `google`, `typ` `savetowallet`, `origins` the web origin) then names only the object's ID and class. So a link used late saves the object as it is then, and one used after the account was deleted saves the void, `INACTIVE` object: no link can create a face of its own, let alone a live one after the void. Google documents no expiry (`exp`) for a save link's JWT, which is why the link carries no face rather than relying on one. If Google cannot be reached the api answers 500 and logs the HTTP status and Google's error code. The class limits a pass to one person's devices. Whether to ask Google for its private pass type instead is open (§11); the object would change in little but its type name.

**Storage.** `wallet_pass` (from migration 0001) is one row per pass; migration `0010_wallet.sql` adds the face's time and hashes, the update queue's columns and the hourly count, and `wallet_device_registration` for Apple's devices; `0012_wallet_tokens.sql` moves the authentication tokens' hashes to `wallet_auth_token` (the token each pass had stays valid), and adds `wallet_download_link`, the day the delivered face was drawn for, the hourly count of new devices, and when a device was told that its voided pass changed. The serial number is random and says nothing about the exchange or the person.

**Updates.** Every change to an exchange marks its passes `PENDING` in the transaction that records it (`exchanges::repo::persist` calls `wallet::store::mark_exchange_changed`), the way the outbox queues a message with its event. The pass row is its own queue entry: many changes in a row make one update, and an update always carries the latest face, so nothing that reaches a device can go backwards. The worker (`wallet::delivery`, on every 5-second pass) takes marked passes on a two-minute lease rather than a lock held while it sends, so marking a pass never waits on a push. It draws the face as it now is; if that is what was last delivered, nothing is sent (a statement added, say, changes nothing on the pass). Otherwise, for Apple, the pass's Last-Modified moves on and each registered device gets an empty push through APNs, after which the device fetches the pass; a token Apple says is dead is forgotten. For Google, the object is patched through the Google Wallet API with an access token for the service account; should Google not have the object, the face is not counted as delivered, so the next update, or the next link, which makes the object again, carries it. A detailed face also changes with the date, *Due soon* and *Overdue*, with nothing happening to the exchange (a neutral one does not, and the daily look below then finds nothing to send): on each pass the worker also marks the passes of exchanges in force that have a pending contribution due on a date and whose last face was drawn for a day before today in the exchange's timezone (where the reminders read due dates too), so each such pass is looked at once a day and sent only if its face changed. A refusal is logged and kept in `last_error` as the HTTP status and the provider's error code only, never Google's message. A failure is retried with backoff, 8 times, and then the pass is left `FAILED` with the error in `last_error` until the exchange changes again.

**What it is built on.** RSA signatures are ring's (constant-time, and already the TLS provider here); the CMS structures are the `cms` crate's from RustCrypto, used without its `builder` feature so that the `rsa` crate, whose timing advisory has no fix (RUSTSEC-2023-0071), is not in the service. HTTP to Apple and Google is hyper with rustls (`hyper-rustls`), HTTP/2 to APNs with the pass type certificate as the client certificate. The tests make their RSA keys with the `rsa` crate, as a development dependency only, which the advisory itself says is fine for local use.

## Settings

| Setting | |
|---|---|
| `APPLE_PASS_TYPE_ID` | The pass type identifier, such as `pass.app.yuppers` |
| `APPLE_TEAM_ID` | The Apple team ID, ten letters and digits |
| `APPLE_PASS_CERT` | The pass type certificate, PEM: the text itself, or the path of a file holding it |
| `APPLE_PASS_KEY` | Its private key, unencrypted PEM (PKCS #8 or PKCS #1), text or path. A secret |
| `APPLE_WWDR_CERT` | Apple's WWDR intermediate certificate that issued the pass type certificate, PEM, text or path |
| `GOOGLE_WALLET_ISSUER_ID` | The issuer ID, digits |
| `GOOGLE_WALLET_SERVICE_ACCOUNT` | The service account's JSON key file: its path, or the JSON itself. A secret |
| `WALLET_DELIVERY` | How updates go once a platform is on: `live` (APNs and the Google Wallet API; the api also creates Google objects through the API) or `log` (written to the log, for development). Required then, with no default, like the other deliveries |
| `WALLET_STATUS_ON_FACE` | `neutral` (the default) or `detailed`: how much a pass's status line says (above) |

A platform is on when all of its settings are set and off when none are; anything in between, a key that is not the certificate's, or a certificate issued for another pass type or team stops the process at start. Certificates that have run out take Apple off instead, with an error in the log, while Google and everything else go on: at start, a WWDR certificate that has expired or did not issue the pass type certificate (its name and its signature are checked); and a pass type certificate that has expired, at start or whenever it expires while the process runs (checked on every use). Apple then answers `WALLET_UNAVAILABLE`, and `GET /v1/meta` stops naming it. A certificate within 30 days of expiry is logged as a warning at start, and both processes report `yuppers_wallet_cert_expiry_seconds` (docs/operations.md, "Metrics"). Both the api and the worker read the same settings: the api signs passes and answers devices, the worker pushes and patches. The web origin (`WEB_ORIGIN`) is where passes link to and where Apple's devices reach the web service, so it must be the public HTTPS origin the API is served from.

## Once the accounts exist

In order. None of it can be done before the accounts exist (`DESIGN.md` §11).

### Apple

1. **Pass type identifier.** In the Apple Developer account, Certificates, Identifiers & Profiles, Identifiers, add a *Pass Type ID*: `pass.app.yuppers` (the description is for you).
2. **Certificate.** On a machine you trust, make a key and a signing request:

   ```sh
   openssl req -new -newkey rsa:2048 -nodes -keyout pass.key -out pass.csr -subj "/CN=Yuppers Pass Type"
   ```

   In Certificates, add a *Pass Type ID Certificate* for that identifier, upload `pass.csr`, download `pass.cer`, and convert it: `openssl x509 -inform der -in pass.cer -out pass.pem`. (Keychain Access can make the request instead; then export the certificate with its key as a `.p12` and take the two apart with `openssl pkcs12 -in pass.p12 -clcerts -nokeys -out pass.pem` and `openssl pkcs12 -in pass.p12 -nocerts -nodes -out pass.key`.) `pass.key` is the secret: put it in the platform's secret store and delete the local copy.
3. **WWDR intermediate.** From Apple's certificate authority page (apple.com/certificateauthority), download the *Worldwide Developer Relations* intermediate that issued `pass.pem` (`openssl x509 -in pass.pem -noout -issuer` names it; G4 at the time of writing) and convert it: `openssl x509 -inform der -in AppleWWDRCAG4.cer -out wwdr.pem`.
4. **Calendar.** Pass type certificates expire (a year, roughly; `openssl x509 -in pass.pem -noout -enddate`). An expired one stops every update and every new Apple pass: Apple is then off, and `yuppers_wallet_cert_expiry_seconds` counts down to it. Put the date on the shared calendar with its owner and backup, and renew a month ahead: a new certificate for the same identifier keeps every issued pass working.
5. **Settings.** `APPLE_PASS_TYPE_ID=pass.app.yuppers`, `APPLE_TEAM_ID`, `APPLE_PASS_CERT`, `APPLE_PASS_KEY`, `APPLE_WWDR_CERT`, and `WALLET_DELIVERY=live`, on the api and the worker; restart both.

### Google

1. **Issuer account.** In the Google Pay & Wallet Console, set up the Google Wallet API; note the issuer ID.
2. **Service account.** In a Google Cloud project, enable the Google Wallet API, create a service account, and create a JSON key for it. The file is the secret.
3. **Access.** In the Wallet Console, add the service account's email under Users, with developer access. While the issuer is in demo mode only the test accounts added there can save passes; request publishing access to issue to anyone, which Google reviews.
4. **Settings.** `GOOGLE_WALLET_ISSUER_ID`, `GOOGLE_WALLET_SERVICE_ACCOUNT`, and `WALLET_DELIVERY=live`; restart the api and the worker.

### Trying it on a device

1. With the settings in place and the API on the public HTTPS origin, `GET /v1/meta` names the platform.
2. **iPhone.** Make two test accounts and take an exchange to an agreement in force. Open it in Safari on the iPhone (or in a development build of the app) and press "Add to Apple Wallet": Wallet's sheet shows the pass. Add it. In the API's log, a `POST` to `/v1/wallet/apple/v1/devices/...` answered 201 is the registration.
3. As the other party, mark something delivered. Within a few seconds the worker logs `wallet passes updated`, and the pass on the iPhone says *Waiting for you* (pull down on the pass's back to make Wallet ask at once). On the device, Settings, Developer, PassKit Testing: *Additional Logging* writes Wallet's side to the console of a Mac attached to it, and *Allow HTTP Services* lets a development build reach a web service that is not HTTPS, which Wallet otherwise refuses.
4. **Android.** The same with "Add to Google Wallet" in Chrome or the app, signed in to Google as one of the issuer's test accounts while in demo mode.
5. **Revocation.** Delete the test account that holds the passes: Apple's pass turns void within a few seconds, and Google's moves to expired passes.
6. Check both languages, light and dark, and VoiceOver and TalkBack reading the pass.

## What remains

- **Not verified on a device**, because no account exists: that Wallet accepts the signature and the pass (the tests check them against the same libraries that made them, not against Apple), that APNs takes the push as sent (an empty JSON object, topic only, no `apns-push-type`, which is how Apple's pass web service documentation describes it), that Google accepts the class and the object as created through the API, and the JWT that names the object.
- **iOS app.** The app opens the download link in Safari, which shows Wallet's sheet, rather than presenting the sheet itself: that needs PassKit's `PKAddPassesViewController` in a small native module (§13.1 expects one), and handing the file to the share sheet does not reliably offer Wallet. On Android the app opens the save link, which Google Wallet takes.
- **Badges.** The buttons are plain buttons with the product's own words. Apple's "Add to Apple Wallet" badge and Google's "Add to Google Wallet" button may only be used under their brand guidelines, which the owner accepts with each account; once accepted, the artwork can replace the buttons (Apple asks for its badge wherever a pass is offered).
- **The web on an Apple device** opens the same download link, so Safari adds the pass. Chrome and the others on a Mac cannot add passes and are shown no button.
- **Alias.** The pass names the other party only by an alias, and nothing sets one yet.
- **Lock-screen status.** Opt-in in §11; not offered (above).
- **Private pass type** for Google, if trying both on devices settles on it (§11).
- **Metrics.** The worker logs its Wallet passes (`wallet passes updated`, `wallet pass not updated; will retry`, `wallet pass update given up on`); it does not yet count them in `/metrics`, which has only the certificate's expiry.
- **Detail on the face, per person.** Neutral by default, detailed only for a whole deployment; the per-person opt-in is planned (above).
- **The owner's decisions.** Whether suspending an account should void its passes (today it freezes them).
