-- Payment options (backend/src/payments.rs; README, "Payment options"): the
-- names a person may save for being paid in another app, and, per
-- agreement, whether they show them to the other party. Yuppers never
-- moves money: a payment option is a link to someone else's app, shown to
-- the person who owes money, and the payment is recorded only when that
-- person says so ("I've paid") and the other confirms it.
--
-- `payment_handle`: one row per account that has saved any, or changed them.
--
--   venmo_encrypted     a Venmo username, without the @;
--   cash_app_encrypted  a Cash App $Cashtag, without the $;
--   paypal_encrypted    a PayPal.Me name;
--   zelle_encrypted     the email address or US phone number (E.164) a
--                       person is paid at by Zelle, which has no links: it
--                       is only shown.
--
-- Each encrypted as an email address is (0025): XChaCha20-Poly1305 under a
-- key derived from CONTACT_DATA_KEY, with the table, the column and the
-- row's account as associated data, so that a value copied into another
-- account's row does not decrypt there. One byte naming the key, the
-- 24-byte nonce, the text (a username of at most 30 bytes, a $Cashtag or a
-- PayPal.Me name of at most 20, an email address of at most 254) and the
-- 16-byte tag. Nothing looks them up, so there is no blind index.
--
-- `*_changed_at`: when each was last set to a value it did not have
-- before, empty while there is none. A payer is warned beside a payment
-- option that changed recently, while they owe money (`payments::
-- CHANGE_WARNING_DAYS`); the old value is never kept.
--
-- `writes_in_window` and `write_window_started_at` count the changes the
-- account made to its payment options, saving them or showing them on an
-- agreement, in the last hour (`payments::WRITES_PER_HOUR`). Removing them
-- empties the columns and keeps the row, so the count stays; deleting the
-- account deletes the row.
CREATE TABLE payment_handle (
    account_id              uuid PRIMARY KEY REFERENCES account,
    venmo_encrypted         bytea CHECK (octet_length(venmo_encrypted) BETWEEN 46 AND 71),
    cash_app_encrypted      bytea CHECK (octet_length(cash_app_encrypted) BETWEEN 42 AND 61),
    paypal_encrypted        bytea CHECK (octet_length(paypal_encrypted) BETWEEN 42 AND 61),
    zelle_encrypted         bytea CHECK (octet_length(zelle_encrypted) BETWEEN 44 AND 295),
    venmo_changed_at        timestamptz,
    cash_app_changed_at     timestamptz,
    paypal_changed_at       timestamptz,
    zelle_changed_at        timestamptz,
    updated_at              timestamptz NOT NULL DEFAULT now(),
    writes_in_window        integer NOT NULL DEFAULT 0 CHECK (writes_in_window >= 0),
    write_window_started_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT payment_handle_changed_with_value CHECK (
        (venmo_changed_at IS NULL) = (venmo_encrypted IS NULL)
        AND (cash_app_changed_at IS NULL) = (cash_app_encrypted IS NULL)
        AND (paypal_changed_at IS NULL) = (paypal_encrypted IS NULL)
        AND (zelle_changed_at IS NULL) = (zelle_encrypted IS NULL))
);

GRANT SELECT, INSERT, UPDATE, DELETE ON payment_handle TO exchange_app;

-- `payment_offer`: the agreements on which a party shows their payment
-- options to the other. A row is "on"; turning them off deletes it. Per
-- party and per agreement, never part of the terms: it is not in a
-- revision, not in its content hash and not in the history, and it binds
-- nobody. Keyed by the account, not the slot, so that someone who later
-- takes an invited party's place starts with it off.
CREATE TABLE payment_offer (
    exchange_id uuid NOT NULL REFERENCES exchange,
    account_id  uuid NOT NULL REFERENCES account,
    shown_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (exchange_id, account_id)
);

CREATE INDEX payment_offer_account_idx ON payment_offer (account_id);

GRANT SELECT, INSERT, DELETE ON payment_offer TO exchange_app;
