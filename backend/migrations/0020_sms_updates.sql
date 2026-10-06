-- Agreement updates by text message (DESIGN.md §12): the second SMS
-- program, "Yuppers.app agreement updates". A party turns texts on for one
-- agreement by ticking a box that shows the consent wording the terms quote;
-- each status change of that agreement then queues one text to them through
-- the outbox. Replying STOP to any of our texts ends every text to the
-- number, sign-in codes included.

-- Which agreements a person has turned text updates on for, and the number
-- they were turned on with. A row is there while updates are on and is
-- removed when they are turned off, by the person, by STOP, by a change of
-- the account's phone number, or by deleting the account. Texts go only
-- while the account still has this number.
CREATE TABLE sms_update (
    account_id   uuid NOT NULL REFERENCES account,
    exchange_id  uuid NOT NULL REFERENCES exchange,
    phone        text NOT NULL CHECK (phone ~ '^\+[1-9][0-9]{6,14}$'),
    turned_on_at timestamptz NOT NULL DEFAULT now(),

    PRIMARY KEY (account_id, exchange_id)
);

CREATE INDEX sms_update_phone_idx ON sms_update (phone);

-- The record of consent, kept as proof of opt-in: every time updates were
-- turned on or off for an agreement, and every STOP and START received.
-- Rows are only ever added; the worker removes them once they are past the
-- retention the privacy policy states, unless they are what keeps updates on.
--
--   OPT_IN   ticking the box: who, which agreement, the number, the version
--            and language of the consent wording shown, and from which client;
--   OPT_OUT  updates turned off for one agreement, and why (`source`);
--   STOP     a stop keyword received from the number, which ends all texts;
--   START    a start keyword received, which allows texts again (but turns
--            no agreement's updates back on).
CREATE TABLE sms_consent (
    id               bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    action           text NOT NULL CHECK (action IN ('OPT_IN', 'OPT_OUT', 'STOP', 'START')),
    account_id       uuid REFERENCES account,
    exchange_id      uuid REFERENCES exchange,
    phone            text NOT NULL CHECK (phone ~ '^\+[1-9][0-9]{6,14}$'),
    -- Where it came from: the web app or one of the apps (as the client
    -- named itself), a reply by text, or the service on the person's behalf
    -- when the account was deleted or its phone number replaced.
    source           text NOT NULL CHECK (source IN (
                         'WEB', 'IOS', 'ANDROID', 'UNKNOWN',
                         'SMS_REPLY', 'ACCOUNT_DELETED', 'PHONE_CHANGED')),
    -- The word received, for STOP and START.
    keyword          text CHECK (char_length(keyword) BETWEEN 1 AND 32),
    consent_version  text CHECK (char_length(consent_version) BETWEEN 1 AND 32),
    consent_language text CHECK (consent_language ~ '^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$'),
    created_at       timestamptz NOT NULL DEFAULT now(),

    CHECK (action NOT IN ('OPT_IN', 'OPT_OUT')
           OR (account_id IS NOT NULL AND exchange_id IS NOT NULL)),
    CHECK ((action = 'OPT_IN') = (consent_version IS NOT NULL AND consent_language IS NOT NULL)),
    CHECK ((action IN ('STOP', 'START')) = (keyword IS NOT NULL))
);

CREATE INDEX sms_consent_subscription_idx ON sms_consent (account_id, exchange_id);
CREATE INDEX sms_consent_created_idx ON sms_consent (created_at);

-- The request's network address and user agent for an opt-in, kept apart
-- and for as long as a signature's (acceptance_network_metadata), then
-- removed by the worker.
CREATE TABLE sms_consent_network (
    consent_id  bigint PRIMARY KEY REFERENCES sms_consent ON DELETE CASCADE,
    ip_address  inet,
    user_agent  text CHECK (char_length(user_agent) <= 512),
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX sms_consent_network_recorded_idx ON sms_consent_network (recorded_at);

-- Numbers that replied STOP and have not replied START since. Nothing is
-- texted to them: no update, and no one-time code.
CREATE TABLE sms_opt_out (
    phone        text PRIMARY KEY CHECK (phone ~ '^\+[1-9][0-9]{6,14}$'),
    opted_out_at timestamptz NOT NULL DEFAULT now()
);

-- Update texts go through the outbox, as their own channel.
ALTER TABLE outbox DROP CONSTRAINT outbox_kind_check;
ALTER TABLE outbox ADD CONSTRAINT outbox_kind_check
    CHECK (kind IN ('EMAIL', 'PUSH', 'PASS_UPDATE', 'SMS'));

-- The texts a person was queued today, for the daily cap on update texts.
CREATE INDEX outbox_sms_recipient_idx ON outbox (recipient_account_id, created_at)
    WHERE kind = 'SMS';

GRANT SELECT, INSERT, UPDATE, DELETE ON sms_update, sms_opt_out TO exchange_app;
-- The record is added to, and removed only by the worker's retention purge.
GRANT SELECT, INSERT, DELETE ON sms_consent, sms_consent_network TO exchange_app;
