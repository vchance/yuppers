-- Combining two accounts, and removing an email address or phone number
-- (`crate::combine`; README, "Combining accounts").
--
-- Someone signed in to one account (A) proves, with a one-time code, an
-- email address or phone number that belongs to another (B). They control
-- both, and may combine them: B's place in each of its yups passes to A,
-- with its identifier and what else is listed in `crate::combine`, and B
-- stops being an account anyone can sign in to. What B signed and did stays
-- as it was written, under B: history is append-only (DESIGN.md §3). B's row
-- keeps `merged_into`, which is how a question about B is asked of A.
--
-- What this migration adds:
--
--   1. `account`: the status `MERGED`, `merged_into` and `merged_at`. A
--      combined account holds no address or number and cannot be reached.
--   2. `slot_holding`: a holding can end because its account was combined
--      into another (`ended_by_merge`). The place passes to that account
--      under a new holding, and a signature made under the old one still
--      counts while the chain of combined holdings reaches the one open now
--      (`holding_void_since`). Removing a claimant, or the claimant leaving,
--      still ends the chain, as before.
--   3. `account_merge`: the audit of every combination, append-only.
--   4. `account_combine_offer`: the short-lived, single-use token that the
--      proof of B's identifier gives A.
--   5. `combine_notice`: the email addresses told that two accounts were
--      combined, encrypted, until the worker has told them; and
--      `account.combined_notice_at`, the notice shown in the app instead
--      where neither account had an email address. Nothing is texted: the
--      SMS program is agreement updates only.
--   6. `invitation`: the address or number an invitation names, encrypted
--      as well as indexed, so that someone signed in with another can be
--      sent a code to add it ("This invitation was sent to j•••@…").
--   7. `deletion_log`: a line can say that the account was combined into
--      another, so that replaying the log after a restore combines it again.
--   8. `sms_consent`: an opt-out recorded because a combination dropped the
--      number.

------------------------------------------------------------------------------
-- 1. Accounts
------------------------------------------------------------------------------

ALTER TABLE account DROP CONSTRAINT account_status_check;
ALTER TABLE account ADD CONSTRAINT account_status_check
    CHECK (status IN ('ACTIVE', 'SUSPENDED', 'DELETED', 'MERGED'));

ALTER TABLE account
    ADD COLUMN merged_into uuid REFERENCES account,
    ADD COLUMN merged_at   timestamptz,
    -- When accounts were combined into this one with no email address on
    -- either to tell: the app shows it once, until dismissed.
    ADD COLUMN combined_notice_at timestamptz,
    ADD CONSTRAINT account_merged_whole CHECK (
        (status = 'MERGED') = (merged_into IS NOT NULL)
        AND (merged_into IS NULL) = (merged_at IS NULL)
        AND merged_into IS DISTINCT FROM id),
    -- A combined account is reached through the one it was combined into.
    ADD CONSTRAINT account_merged_unreachable CHECK (
        status <> 'MERGED'
        OR num_nonnulls(email_encrypted, email_index, phone_encrypted, phone_index) = 0);

ALTER TABLE account DROP CONSTRAINT account_reachable;
ALTER TABLE account ADD CONSTRAINT account_reachable
    CHECK (status IN ('DELETED', 'MERGED') OR num_nonnulls(email_index, phone_index) >= 1);

CREATE INDEX account_merged_into_idx ON account (merged_into) WHERE merged_into IS NOT NULL;

------------------------------------------------------------------------------
-- 2. A place passes to the account its holder was combined into
------------------------------------------------------------------------------

ALTER TABLE slot_holding ADD COLUMN ended_by_merge boolean NOT NULL DEFAULT false;
ALTER TABLE slot_holding ADD CONSTRAINT slot_holding_merge_ends
    CHECK (NOT ended_by_merge OR ended_at IS NOT NULL);

-- As in 0006, and one more way for the participant row to change: from an
-- account to the account it was combined into, which must already be marked
-- so on the first account's row. That ends the first holding, as combined,
-- and opens the next for the second account.
CREATE OR REPLACE FUNCTION participant_holding() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF OLD.initiator_confirmed_at IS NOT NULL AND NEW.initiator_confirmed_at IS NULL THEN
            RAISE EXCEPTION 'a confirmation is never taken back'
                USING ERRCODE = 'check_violation';
        END IF;
        IF NEW.account_id IS NOT DISTINCT FROM OLD.account_id THEN
            RETURN NULL;
        END IF;
        IF OLD.account_id IS NOT NULL THEN
            IF NEW.account_id IS NOT NULL THEN
                IF NOT EXISTS (SELECT 1 FROM account a
                               WHERE a.id = OLD.account_id AND a.status = 'MERGED'
                                 AND a.merged_into = NEW.account_id) THEN
                    RAISE EXCEPTION 'a slot is emptied before another account takes it'
                        USING ERRCODE = 'check_violation';
                END IF;
                UPDATE slot_holding SET ended_at = now(), ended_by_merge = true
                WHERE exchange_id = OLD.exchange_id AND slot = OLD.slot AND ended_at IS NULL;
                INSERT INTO slot_holding (exchange_id, slot, holding, account_id)
                SELECT NEW.exchange_id, NEW.slot, coalesce(max(holding), 0) + 1, NEW.account_id
                FROM slot_holding
                WHERE exchange_id = NEW.exchange_id AND slot = NEW.slot;
                RETURN NULL;
            END IF;
            IF OLD.slot <> 'B' OR OLD.initiator_confirmed_at IS NOT NULL THEN
                RAISE EXCEPTION 'only an invited party the initiator has not confirmed can be removed'
                    USING ERRCODE = 'check_violation';
            END IF;
            UPDATE slot_holding SET ended_at = now()
            WHERE exchange_id = OLD.exchange_id AND slot = OLD.slot AND ended_at IS NULL;
            RETURN NULL;
        END IF;
    END IF;

    IF NEW.account_id IS NOT NULL THEN
        INSERT INTO slot_holding (exchange_id, slot, holding, account_id)
        SELECT NEW.exchange_id, NEW.slot, coalesce(max(holding), 0) + 1, NEW.account_id
        FROM slot_holding
        WHERE exchange_id = NEW.exchange_id AND slot = NEW.slot;
    END IF;
    RETURN NULL;
END;
$$;

-- As in 0006: a holding begins, open, for the account the participant row
-- names, and ends once, when the row names nobody. Now also: it ends, as
-- combined, when the row names the account its holder was combined into.
-- Nothing else about it can change, for any role.
CREATE OR REPLACE FUNCTION slot_holding_follows_participant() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    holder uuid;
BEGIN
    IF TG_OP <> 'DELETE' THEN
        SELECT p.account_id INTO holder FROM participant p
        WHERE p.exchange_id = NEW.exchange_id AND p.slot = NEW.slot;

        IF TG_OP = 'INSERT' THEN
            IF NEW.ended_at IS NULL AND NOT NEW.ended_by_merge AND holder = NEW.account_id THEN
                RETURN NEW;
            END IF;
        ELSIF OLD.ended_at IS NULL AND NEW.ended_at IS NOT NULL AND NOT OLD.ended_by_merge
              AND (NEW.exchange_id, NEW.slot, NEW.holding, NEW.account_id, NEW.began_at)
                  = (OLD.exchange_id, OLD.slot, OLD.holding, OLD.account_id, OLD.began_at)
              AND ((holder IS NULL AND NOT NEW.ended_by_merge)
                   OR (NEW.ended_by_merge AND holder IS NOT NULL
                       AND holder = (SELECT a.merged_into FROM account a
                                     WHERE a.id = OLD.account_id AND a.status = 'MERGED'))) THEN
            RETURN NEW;
        END IF;
    END IF;
    RAISE EXCEPTION 'slot_holding follows the participant row: % is not allowed here', TG_OP
        USING ERRCODE = 'insufficient_privilege';
END;
$$;

-- When a signature made under `holding` stopped counting: never (null)
-- while the holding is open, or while it was ended only by combining its
-- account into the account that holds the place now, through as many
-- combinations as there were. Otherwise when the chain was broken: the
-- claimant was removed, or left.
CREATE FUNCTION holding_void_since(p_exchange uuid, p_slot text, p_holding integer)
    RETURNS timestamptz
    LANGUAGE sql STABLE SET search_path = public, pg_temp AS
$$
    WITH RECURSIVE chain AS (
        SELECT h.holding, h.ended_at, h.ended_by_merge
        FROM slot_holding h
        WHERE h.exchange_id = p_exchange AND h.slot = p_slot AND h.holding = p_holding
        UNION ALL
        SELECT h.holding, h.ended_at, h.ended_by_merge
        FROM slot_holding h
        JOIN chain c ON h.holding = c.holding + 1
        WHERE h.exchange_id = p_exchange AND h.slot = p_slot AND c.ended_by_merge
    )
    SELECT ended_at FROM chain ORDER BY holding DESC LIMIT 1;
$$;

REVOKE ALL ON FUNCTION holding_void_since(uuid, text, integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION holding_void_since(uuid, text, integer) TO exchange_app;

-- As in 0006, counting each slot once, by the signatures that still count.
-- After a combination, the place's signature may have been given under the
-- holding before it; it counts as the current holder's own.
CREATE OR REPLACE FUNCTION exchange_in_force_signed() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
BEGIN
    IF NEW.in_force_revision_id IS NULL
       OR (TG_OP = 'UPDATE'
           AND NEW.in_force_revision_id IS NOT DISTINCT FROM OLD.in_force_revision_id) THEN
        RETURN NULL;
    END IF;

    IF (SELECT count(DISTINCT a.slot)
        FROM acceptance a
        WHERE a.exchange_id = NEW.id AND a.revision_id = NEW.in_force_revision_id
          AND holding_void_since(a.exchange_id, a.slot, a.holding) IS NULL) < 2 THEN
        RAISE EXCEPTION 'a revision comes into force only when both current parties have signed it'
            USING ERRCODE = 'check_violation';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM participant p
                   WHERE p.exchange_id = NEW.id AND p.slot = 'B'
                     AND p.initiator_confirmed_at IS NOT NULL) THEN
        RAISE EXCEPTION 'a revision comes into force only once the invited party is confirmed'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END;
$$;

------------------------------------------------------------------------------
-- 3. The audit of every combination
------------------------------------------------------------------------------

-- One row per combination: which account was combined into which, by which
-- kind of identifier, when, and whether the person did it or a replay of
-- the deletion log after a restore did it again. Never changed or removed.
CREATE TABLE account_merge (
    id                bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    merged_account_id uuid NOT NULL UNIQUE REFERENCES account,
    into_account_id   uuid NOT NULL REFERENCES account,
    identifier_kind   text NOT NULL CHECK (identifier_kind IN ('EMAIL', 'PHONE')),
    source            text NOT NULL CHECK (source IN ('PERSON', 'REPLAY')),
    merged_at         timestamptz NOT NULL DEFAULT now(),
    CHECK (merged_account_id <> into_account_id)
);

CREATE INDEX account_merge_into_idx ON account_merge (into_account_id);

CREATE TRIGGER account_merge_append_only
    BEFORE UPDATE OR DELETE OR TRUNCATE ON account_merge
    FOR EACH STATEMENT EXECUTE FUNCTION forbid_change();

------------------------------------------------------------------------------
-- 4. The offer to combine
------------------------------------------------------------------------------

-- What a verified code for another account's identifier gives the account
-- that entered it: a token, kept as its hash, good once, for a few minutes,
-- for combining exactly these two accounts by exactly that identifier.
-- `identifier_index` is the blind index of the identifier proved: if it no
-- longer belongs to the other account, the offer is void.
CREATE TABLE account_combine_offer (
    token_hash       bytea PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    account_id       uuid NOT NULL REFERENCES account,
    other_account_id uuid NOT NULL REFERENCES account,
    identifier_kind  text NOT NULL CHECK (identifier_kind IN ('EMAIL', 'PHONE')),
    identifier_index bytea NOT NULL CHECK (octet_length(identifier_index) = 32),
    created_at       timestamptz NOT NULL DEFAULT now(),
    expires_at       timestamptz NOT NULL,
    used_at          timestamptz,
    CHECK (account_id <> other_account_id),
    CHECK (expires_at > created_at)
);

CREATE INDEX account_combine_offer_account_idx ON account_combine_offer (account_id);
CREATE INDEX account_combine_offer_other_idx ON account_combine_offer (other_account_id);
CREATE INDEX account_combine_offer_expires_idx ON account_combine_offer (expires_at);

------------------------------------------------------------------------------
-- 5. Telling every email address that two accounts were combined
------------------------------------------------------------------------------

-- One row per email address told, encrypted with the row's ID as
-- associated data, as the records of consent are (`crate::contact`). An
-- outbox row names it (`{"combine_notice": id}`); the worker removes it
-- once it is a week old, sent or not. Phone numbers are not told: the SMS
-- program covers agreement updates only.
CREATE TABLE combine_notice (
    id              bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id      uuid NOT NULL REFERENCES account,
    email_encrypted bytea NOT NULL CHECK (octet_length(email_encrypted) BETWEEN 42 AND 295),
    language        text NOT NULL CHECK (language ~ '^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$'),
    created_at      timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX combine_notice_account_idx ON combine_notice (account_id);
CREATE INDEX combine_notice_created_idx ON combine_notice (created_at);

------------------------------------------------------------------------------
-- 6. Whom an invitation names, so that a code can be sent there
------------------------------------------------------------------------------

-- Encrypted with the invitation's ID as associated data. Kept only while
-- the invitation can still be claimed: the worker empties it once the
-- invitation is claimed, revoked or expired, and a deletion empties it with
-- the index. An invitation issued before this has the index alone, and is
-- refused as before to anyone signed in with another address.
ALTER TABLE invitation
    ADD COLUMN bound_email_encrypted bytea
        CHECK (octet_length(bound_email_encrypted) BETWEEN 42 AND 295),
    ADD COLUMN bound_phone_encrypted bytea
        CHECK (octet_length(bound_phone_encrypted) BETWEEN 42 AND 72),
    ADD CONSTRAINT invitation_bound_encrypted_indexed CHECK (
        (bound_email_encrypted IS NULL OR bound_email_index IS NOT NULL)
        AND (bound_phone_encrypted IS NULL OR bound_phone_index IS NOT NULL));

CREATE INDEX invitation_bound_encrypted_idx ON invitation (id)
    WHERE bound_email_encrypted IS NOT NULL OR bound_phone_encrypted IS NOT NULL;

------------------------------------------------------------------------------
-- 7. The deletion log names combined accounts too
------------------------------------------------------------------------------

-- A line with `merged_into` says the account was combined into that one at
-- that time, by its email address or its phone number (`merged_by`), which
-- decides whose address or number the combined account kept. Replaying it
-- after a restore combines the two again, through the service's own code
-- (`crate::combine::replay`).
ALTER TABLE deletion_log
    ADD COLUMN merged_into uuid REFERENCES account,
    ADD COLUMN merged_by   text CHECK (merged_by IN ('EMAIL', 'PHONE')),
    ADD CONSTRAINT deletion_log_merged_whole CHECK ((merged_into IS NULL) = (merged_by IS NULL));

CREATE OR REPLACE FUNCTION deletion_log_account_deleted() RETURNS trigger
    LANGUAGE plpgsql SET search_path = public, pg_temp AS
$$
BEGIN
    IF NEW.merged_into IS NULL THEN
        IF NOT EXISTS (SELECT 1 FROM account WHERE id = NEW.account_id AND status = 'DELETED') THEN
            RAISE EXCEPTION 'only a deleted account is added to the deletion log'
                USING ERRCODE = 'insufficient_privilege';
        END IF;
    ELSIF NOT EXISTS (SELECT 1 FROM account
                      WHERE id = NEW.account_id AND status = 'MERGED'
                        AND merged_into = NEW.merged_into) THEN
        RAISE EXCEPTION 'only an account combined into that one is logged as combined into it'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END;
$$;

-- As in 0018, with a combined account counting as done: its line is
-- replayed by combining it again, after which it is `MERGED`.
CREATE OR REPLACE FUNCTION restore_replay_done() RETURNS boolean
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS
$$
DECLARE
    live bigint;
BEGIN
    LOCK TABLE restore_marker IN SHARE ROW EXCLUSIVE MODE;
    IF NOT restore_replay_pending() THEN
        RETURN false;
    END IF;
    SELECT count(*) INTO live
    FROM deletion_log l JOIN account a ON a.id = l.account_id
    WHERE CASE WHEN l.merged_into IS NULL THEN a.status <> 'DELETED'
               ELSE a.status <> 'MERGED' END;
    IF live > 0 THEN
        RAISE EXCEPTION '% accounts in the deletion log are not deleted; replay again', live
            USING ERRCODE = 'check_violation';
    END IF;
    INSERT INTO restore_marker (event, note) VALUES ('REPLAYED', 'replay-deletions');
    RETURN true;
END;
$$;

------------------------------------------------------------------------------
-- 8. Text updates ended by a combination
------------------------------------------------------------------------------

ALTER TABLE sms_consent DROP CONSTRAINT sms_consent_source_check;
ALTER TABLE sms_consent ADD CONSTRAINT sms_consent_source_check
    CHECK (source IN (
        'WEB', 'IOS', 'ANDROID', 'UNKNOWN',
        'SMS_REPLY', 'ACCOUNT_DELETED', 'PHONE_CHANGED', 'NO_LONGER_A_PARTY',
        'PHONE_REMOVED', 'ACCOUNTS_COMBINED'));

------------------------------------------------------------------------------
-- Application role
------------------------------------------------------------------------------

GRANT SELECT, INSERT ON account_merge TO exchange_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON account_combine_offer TO exchange_app;
GRANT SELECT, INSERT, DELETE ON combine_notice TO exchange_app;
-- The notice's address is encrypted with its row's ID, which the service
-- takes before it writes the row.
GRANT USAGE ON SEQUENCE combine_notice_id_seq TO exchange_app;
