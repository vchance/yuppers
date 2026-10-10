-- Who a signature is attributed to, and a hash chain over each exchange's
-- history (DESIGN.md §6, §14.1; LEGAL_MEMO.md §2.2, §2.4).
--
-- Additive only: no existing column, content hash or signed byte changes.

-- The identifier a session signed in with, as its blind index (the keyed
-- hash `crate::contact` computes for lookups) and its kind. Never the address
-- or number itself. Sessions from before this stay null.
ALTER TABLE account_session
    ADD COLUMN identifier_hash bytea CHECK (octet_length(identifier_hash) = 32),
    ADD COLUMN identifier_kind text CHECK (identifier_kind IN ('email', 'phone')),
    ADD CONSTRAINT account_session_identifier_whole
        CHECK ((identifier_hash IS NULL) = (identifier_kind IS NULL));

-- What each signature records of the session it was made in. Copied from the
-- session when the acceptance is inserted; null on signatures from before.
-- `session_id` names a session that may since have been deleted, so it is not
-- a foreign key.
ALTER TABLE acceptance
    ADD COLUMN signer_identifier_hash bytea CHECK (octet_length(signer_identifier_hash) = 32),
    ADD COLUMN signer_identifier_kind text CHECK (signer_identifier_kind IN ('email', 'phone')),
    ADD COLUMN session_verified_at timestamptz,
    ADD COLUMN session_id uuid,
    -- The hash is cleared when the signer's account is deleted; the kind stays.
    ADD CONSTRAINT acceptance_signer_identifier_whole
        CHECK (signer_identifier_hash IS NULL OR signer_identifier_kind IS NOT NULL);

-- The chain over the history: SHA-256 of the previous row's hash in the same
-- exchange and this row's stable fields (`crate::chain`). Null on history
-- from before this, until `staff backfill-chain` has run.
ALTER TABLE exchange_event
    ADD COLUMN chain_hash bytea CHECK (octet_length(chain_hash) = 32);

-- The rows the backfill still has to chain.
CREATE INDEX exchange_event_unchained_idx
    ON exchange_event (exchange_id, sequence) WHERE chain_hash IS NULL;

-- The history stays append-only for every role, with one exception: the
-- owner's backfill may set `chain_hash` on a row that has none, once, and
-- change nothing else. It says so for its own transaction
-- (`yuppers.chain_backfill`); the application role has no UPDATE on the table
-- whatever it sets.
CREATE FUNCTION forbid_event_change() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF TG_OP = 'UPDATE' AND current_setting('yuppers.chain_backfill', true) = 'on' THEN
        RETURN NULL;
    END IF;
    RAISE EXCEPTION '% is append-only: % is not allowed', TG_TABLE_NAME, TG_OP
        USING ERRCODE = 'insufficient_privilege';
END;
$$;

DROP TRIGGER exchange_event_append_only ON exchange_event;
CREATE TRIGGER exchange_event_append_only
    BEFORE UPDATE OR DELETE OR TRUNCATE ON exchange_event
    FOR EACH STATEMENT EXECUTE FUNCTION forbid_event_change();

CREATE FUNCTION exchange_event_chain_only() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF current_setting('yuppers.chain_backfill', true) = 'on' THEN
        IF OLD.chain_hash IS NULL
           AND NEW.chain_hash IS NOT NULL
           AND (to_jsonb(OLD) - 'chain_hash') = (to_jsonb(NEW) - 'chain_hash') THEN
            RETURN NEW;
        END IF;
        RAISE EXCEPTION 'exchange_event is append-only: only an empty chain_hash may be set'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER exchange_event_chain_only
    BEFORE UPDATE ON exchange_event
    FOR EACH ROW EXECUTE FUNCTION exchange_event_chain_only();

-- A deleted account must not stay linkable through the signatures it made:
-- deletion sets `signer_identifier_hash` to null on its acceptances (and on
-- those of accounts combined into it), keeping the kind and
-- `session_verified_at`. The signature, the content hash and everything else
-- stay as they were. The table stays append-only for every role, with this one
-- exception: that column may go from a value to null, and nothing else may
-- change. Deletion says so for its own transaction
-- (`yuppers.account_deletion`); the application role can update that column
-- and no other.
CREATE FUNCTION forbid_acceptance_change() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF TG_OP = 'UPDATE' AND current_setting('yuppers.account_deletion', true) = 'on' THEN
        RETURN NULL;
    END IF;
    RAISE EXCEPTION '% is append-only: % is not allowed', TG_TABLE_NAME, TG_OP
        USING ERRCODE = 'insufficient_privilege';
END;
$$;

DROP TRIGGER acceptance_append_only ON acceptance;
CREATE TRIGGER acceptance_append_only
    BEFORE UPDATE OR DELETE OR TRUNCATE ON acceptance
    FOR EACH STATEMENT EXECUTE FUNCTION forbid_acceptance_change();

CREATE FUNCTION acceptance_deletion_only() RETURNS trigger
    LANGUAGE plpgsql AS
$$
BEGIN
    IF current_setting('yuppers.account_deletion', true) = 'on' THEN
        IF OLD.signer_identifier_hash IS NOT NULL
           AND NEW.signer_identifier_hash IS NULL
           AND (to_jsonb(OLD) - 'signer_identifier_hash') = (to_jsonb(NEW) - 'signer_identifier_hash') THEN
            RETURN NEW;
        END IF;
        RAISE EXCEPTION 'acceptance is append-only: only signer_identifier_hash may be cleared'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER acceptance_deletion_only
    BEFORE UPDATE ON acceptance
    FOR EACH ROW EXECUTE FUNCTION acceptance_deletion_only();

GRANT UPDATE (signer_identifier_hash) ON acceptance TO exchange_app;
