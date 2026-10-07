-- One-time codes by text, made, sent and checked by Twilio Verify
-- (backend/src/notifications/verify.rs). For such a code the service never
-- sees the code it sends, so it has no hash to store: the row records only
-- that a code was asked for, for what identifier and purpose, when, and
-- until when it works, so that every limit counted from these rows (codes
-- per identifier per hour, the newest few live, wrong guesses per code)
-- holds as before.
--
--   checked_by   SERVICE: the service made the code and keeps its keyed
--                hash, as every row did before (email, and codes written to
--                the development log); TWILIO_VERIFY: Twilio made it and
--                checks it;
--   code_hash    for TWILIO_VERIFY, empty until Twilio approves a code
--                offered back, and then the keyed hash of that code, so
--                that the same code offered again before it is used up
--                (a deletion that found the account busy, and is retried)
--                is recognised without asking Twilio, which forgets a
--                verification once it is approved.
ALTER TABLE one_time_code
    ADD COLUMN checked_by text NOT NULL DEFAULT 'SERVICE'
        CHECK (checked_by IN ('SERVICE', 'TWILIO_VERIFY')),
    ALTER COLUMN code_hash DROP NOT NULL,
    ADD CONSTRAINT one_time_code_hash_unless_verify
        CHECK (code_hash IS NOT NULL OR checked_by = 'TWILIO_VERIFY');
