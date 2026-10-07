-- Email addresses and phone numbers encrypted at rest (backend/src/contact;
-- README, "Contact details at rest"). A dump, a backup or a read of the
-- database shows none in the clear.
--
-- Each place that held one in plaintext holds instead:
--
--   *_encrypted  where the value must be read back, to send to it or to show
--                its owner: XChaCha20-Poly1305 under a key derived from
--                CONTACT_DATA_KEY, with the table and column (and for the
--                records of consent the row's ID) as associated data, so
--                that a value cannot be moved to another column or record.
--                One byte naming the key, the 24-byte nonce, the ciphertext
--                and its 16-byte tag;
--   *_index      where it must be found or compared: HMAC-SHA256 of the
--                normalized value (a lower-case email address, an E.164
--                phone number) under the blind-index key, with the kind of
--                value in what is hashed. The same value has the same index
--                in every table, so tables are still joined on it, and the
--                unique constraints are on it.
--
--   account.email, account.phone        encrypted and indexed (unique)
--   invitation.bound_email, bound_phone  indexed only: a claim is compared
--                                        with it, nothing reads it back
--   one_time_code.identifier            indexed only: the code is sent from
--                                        the request, never from the row
--   sms_update.phone                    indexed only
--   sms_opt_out.phone                   indexed only (the key)
--   sms_consent.phone                   encrypted and indexed: the record of
--                                        consent names the number; an
--                                        opt-out the service writes for a
--                                        subscription it ends has the index
--                                        alone, which ties it to the opt-in
--   sms_code_consent.phone              encrypted; `phone_hash` (HMAC under
--                                        APP_SECRET) is already keyed and
--                                        stays as it is
--
-- **A fresh start.** The plaintext columns are dropped, with what they hold:
-- nothing stored before this migration is converted. So it refuses to run,
-- changing nothing, on a database where any of these tables holds a row.
-- Such a database is reset first (docs/deploy-render.md, "Contact data
-- key"); a development database too (README, "Contact details at rest").

DO $$
DECLARE
    place text;
    held  boolean;
BEGIN
    FOREACH place IN ARRAY ARRAY['account', 'invitation', 'one_time_code', 'sms_update',
                                 'sms_opt_out', 'sms_consent', 'sms_code_consent'] LOOP
        EXECUTE format('SELECT EXISTS (SELECT 1 FROM %I)', place) INTO held;
        IF held THEN
            RAISE EXCEPTION 'migration 0025 drops the plaintext email addresses and phone numbers, '
                'and % holds rows: nothing was changed. Reset the database first '
                '(docs/deploy-render.md, "Contact data key"; for development, README, '
                '"Contact details at rest"), then migrate again', place
                USING ERRCODE = 'object_not_in_prerequisite_state';
        END IF;
    END LOOP;
END;
$$;

------------------------------------------------------------------------------
-- The blind-index key
------------------------------------------------------------------------------

-- The key the indexes are computed under, encrypted under CONTACT_DATA_KEY
-- (associated data `contact_key.index_key`). It is derived once, from the
-- first CONTACT_DATA_KEY, when `migrate` first runs with one, and never
-- changes: rotating CONTACT_DATA_KEY re-encrypts this row with the rest
-- (`contact-data rotate`), so every index stays comparable with every other
-- and no lookup or join has to consider two keys. Without CONTACT_DATA_KEY
-- the row is noise. Only the schema owner writes it; the service reads it at
-- start, which is also how a wrong key is caught before anything is served.
CREATE TABLE contact_key (
    id           boolean PRIMARY KEY DEFAULT true CHECK (id),
    index_key    bytea NOT NULL CHECK (octet_length(index_key) = 1 + 24 + 32 + 16),
    created_at   timestamptz NOT NULL DEFAULT now(),
    rewrapped_at timestamptz
);

GRANT SELECT ON contact_key TO exchange_app;

------------------------------------------------------------------------------
-- The columns
------------------------------------------------------------------------------

-- 1 key byte, 24 nonce bytes, at least one byte of text, 16 tag bytes: an
-- email address is at most 254 bytes, an E.164 number 16.
ALTER TABLE account
    DROP COLUMN email,
    DROP COLUMN phone,
    ADD COLUMN email_encrypted bytea CHECK (octet_length(email_encrypted) BETWEEN 42 AND 295),
    ADD COLUMN email_index     bytea CHECK (octet_length(email_index) = 32),
    ADD COLUMN phone_encrypted bytea CHECK (octet_length(phone_encrypted) BETWEEN 42 AND 72),
    ADD COLUMN phone_index     bytea CHECK (octet_length(phone_index) = 32),
    ADD CONSTRAINT account_email_sealed_whole CHECK ((email_encrypted IS NULL) = (email_index IS NULL)),
    ADD CONSTRAINT account_phone_sealed_whole CHECK ((phone_encrypted IS NULL) = (phone_index IS NULL)),
    -- An account that is not deleted can be reached (it was account_check,
    -- which went with the columns).
    ADD CONSTRAINT account_reachable
        CHECK (status = 'DELETED' OR num_nonnulls(email_index, phone_index) >= 1);
-- One account per address and per number.
CREATE UNIQUE INDEX account_email_index_key ON account (email_index);
CREATE UNIQUE INDEX account_phone_index_key ON account (phone_index);

ALTER TABLE invitation
    DROP COLUMN bound_email,
    DROP COLUMN bound_phone,
    ADD COLUMN bound_email_index bytea CHECK (octet_length(bound_email_index) = 32),
    ADD COLUMN bound_phone_index bytea CHECK (octet_length(bound_phone_index) = 32),
    ADD CONSTRAINT invitation_bound_once
        CHECK (num_nonnulls(bound_email_index, bound_phone_index) <= 1);

ALTER TABLE one_time_code
    DROP COLUMN identifier,
    ADD COLUMN identifier_index bytea NOT NULL CHECK (octet_length(identifier_index) = 32);
CREATE INDEX one_time_code_identifier_index_idx
    ON one_time_code (identifier_index, purpose, created_at DESC);

ALTER TABLE sms_update
    DROP COLUMN phone,
    ADD COLUMN phone_index bytea NOT NULL CHECK (octet_length(phone_index) = 32);
CREATE INDEX sms_update_phone_index_idx ON sms_update (phone_index);

ALTER TABLE sms_opt_out
    DROP COLUMN phone,
    ADD COLUMN phone_index bytea PRIMARY KEY CHECK (octet_length(phone_index) = 32);

ALTER TABLE sms_consent
    DROP COLUMN phone,
    ADD COLUMN phone_encrypted bytea CHECK (octet_length(phone_encrypted) BETWEEN 42 AND 72),
    ADD COLUMN phone_index     bytea NOT NULL CHECK (octet_length(phone_index) = 32);
CREATE INDEX sms_consent_phone_index_idx ON sms_consent (phone_index);

ALTER TABLE sms_code_consent
    DROP COLUMN phone,
    ADD COLUMN phone_encrypted bytea CHECK (octet_length(phone_encrypted) BETWEEN 42 AND 72),
    -- In full only beside the account it belongs to, as before.
    ADD CONSTRAINT sms_code_consent_encrypted_owned
        CHECK (phone_encrypted IS NULL OR account_id IS NOT NULL);

-- A record of consent's number is encrypted with the record's ID in its
-- associated data, so a ciphertext copied from one record does not decrypt
-- in another. The service takes the ID before it writes the row, and so may
-- draw from the two sequences.
GRANT USAGE ON SEQUENCE sms_consent_id_seq, sms_code_consent_id_seq TO exchange_app;
