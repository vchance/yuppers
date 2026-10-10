-- Acceptance of the Terms and the Privacy policy at sign-in
-- (`crate::terms`; README, "Signing in").
--
-- Signing in is the assent: the page says, in one sentence above the button,
-- that continuing means agreeing to both. When a sign-in completes, the
-- version the client showed is stored here, append-only, one row per
-- sign-in, and the account keeps the latest for cheap reads (`GET /v1/me`).
-- Nothing is backfilled: an existing account gets its first row on its next
-- sign-in.
--
-- Deleting an account removes its rows like its other personal rows
-- (`crate::deletion`). A combined account's rows stay under it, as what it
-- did does; the account it was combined into keeps its own latest.

CREATE TABLE terms_acceptance (
    id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id    uuid NOT NULL REFERENCES account,
    terms_version text NOT NULL CHECK (char_length(terms_version) BETWEEN 1 AND 32),
    language      text NOT NULL CHECK (language ~ '^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$'),
    accepted_at   timestamptz NOT NULL DEFAULT now(),
    -- The session the sign-in made, which goes when sessions are swept.
    session_id    uuid
);

CREATE INDEX terms_acceptance_account_idx ON terms_acceptance (account_id, accepted_at);

ALTER TABLE account
    ADD COLUMN terms_version     text CHECK (char_length(terms_version) BETWEEN 1 AND 32),
    ADD COLUMN terms_accepted_at timestamptz,
    ADD CONSTRAINT account_terms_whole CHECK ((terms_version IS NULL) = (terms_accepted_at IS NULL));

GRANT SELECT, INSERT, DELETE ON terms_acceptance TO exchange_app;
GRANT USAGE ON SEQUENCE terms_acceptance_id_seq TO exchange_app;
