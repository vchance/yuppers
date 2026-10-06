-- Consent to a one-time code by text: the first SMS program, "Yuppers.app
-- sign-in codes". Every form that texts a code shows a box, unticked, beside
-- wording that says what will be sent and how often, HELP and STOP, the
-- rates and the two policies; a code is texted only once it is ticked. This
-- is the record of each such tick that led to a code being issued: proof
-- that the person asked for the text they were sent.
--
-- Not `sms_consent` (0020): that is the record of agreement updates, where
-- an opt-in is always a person and an agreement and the number is always
-- kept, and its purge keeps what a subscription still rests on. A code is
-- asked for without an agreement, often without an account, and for a
-- number nobody has yet shown to be theirs, so the checks of 0020 would
-- have to be loosened for every row to fit these.
--
--   purpose          SIGN_IN (signing in, which creates the account the
--                    first time), DELETE_ACCOUNT (the code that confirms a
--                    deletion), VERIFY_NUMBER (adding a number to an account
--                    before turning on agreement updates);
--   account_id       the account the code is for, where there is one: the
--                    signed-in account deleting itself or adding a number,
--                    or the account that already has the number signing in;
--   phone            the number in full, only where it is already that
--                    account's number. Anyone can type any number, so a
--                    number not yet on an account is kept only as
--                    `phone_hash`;
--   phone_hash       always: HMAC-SHA256 of the number under APP_SECRET, so
--                    a record can be found from the number (a complaint
--                    names one) without the table holding numbers nobody
--                    has shown are theirs. A plain hash would not do: the
--                    numbers are few enough to try them all;
--   consent_version  the wording's version (`CODE_CONSENT_VERSION`), and
--   consent_language the language it was shown in: with the purpose, they
--                    say which words were beside the box;
--   source           which client, as it named itself.
CREATE TABLE sms_code_consent (
    id               bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    purpose          text NOT NULL CHECK (purpose IN ('SIGN_IN', 'DELETE_ACCOUNT', 'VERIFY_NUMBER')),
    account_id       uuid REFERENCES account,
    phone            text CHECK (phone ~ '^\+[1-9][0-9]{6,14}$'),
    phone_hash       bytea NOT NULL CHECK (octet_length(phone_hash) = 32),
    source           text NOT NULL CHECK (source IN ('WEB', 'IOS', 'ANDROID', 'UNKNOWN')),
    consent_version  text NOT NULL CHECK (char_length(consent_version) BETWEEN 1 AND 32),
    consent_language text NOT NULL CHECK (consent_language ~ '^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$'),
    created_at       timestamptz NOT NULL DEFAULT now(),

    -- Only signing in may be asked for without an account.
    CHECK (purpose = 'SIGN_IN' OR account_id IS NOT NULL),
    -- The number in full only beside the account it belongs to.
    CHECK (phone IS NULL OR account_id IS NOT NULL)
);

CREATE INDEX sms_code_consent_phone_hash_idx ON sms_code_consent (phone_hash);
CREATE INDEX sms_code_consent_account_idx ON sms_code_consent (account_id);
CREATE INDEX sms_code_consent_created_idx ON sms_code_consent (created_at);

-- The request's network address and user agent, kept apart and for as long
-- as a signature's (acceptance_network_metadata), then removed by the worker.
CREATE TABLE sms_code_consent_network (
    consent_id  bigint PRIMARY KEY REFERENCES sms_code_consent ON DELETE CASCADE,
    ip_address  inet,
    user_agent  text CHECK (char_length(user_agent) <= 512),
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX sms_code_consent_network_recorded_idx ON sms_code_consent_network (recorded_at);

-- The record is added to, and removed only by the worker's retention purge.
GRANT SELECT, INSERT, DELETE ON sms_code_consent, sms_code_consent_network TO exchange_app;
