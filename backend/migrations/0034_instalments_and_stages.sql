-- Instalments, stages and progress notes (DESIGN.md §7.1, §7.2, §12).
--
-- Almost nothing is new to the schema: an instalment or a stage is an
-- ordinary contribution, and a progress note is an ordinary event
-- (`PROGRESS_NOTED`, with the note in `exchange_event.note`), so neither
-- changes the record, the content hash or the state machines.
--
-- What the two new reads need is an index each:
--
-- * telling a burst of claims or confirmations, and the day's one progress
--   note message, from what the outbox already holds for a person on a yup;
-- * counting the progress notes on one contribution, which are limited.

CREATE INDEX outbox_burst_idx
    ON outbox (exchange_id, recipient_account_id, created_at)
    WHERE event_sequence IS NOT NULL;

CREATE INDEX exchange_event_progress_idx
    ON exchange_event (exchange_id, contribution_id)
    WHERE type = 'PROGRESS_NOTED';
