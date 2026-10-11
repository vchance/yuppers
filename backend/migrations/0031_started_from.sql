-- How the author began a yup: a template and its version
-- (`job-deposit-balance@1`), `blank` or `copy` (DESIGN.md §4.4,
-- "Measurement"). Written once, when the exchange is created, from what the
-- client sends; null for exchanges made before this and by clients that
-- send nothing.
--
-- For aggregate reporting only. No query that builds a view, a revision, a
-- content hash, a record, an export or a notification reads it, and the API
-- does not return it. The funnel counters read it by entry.
--
-- The application role's grants on `exchange` already cover a new column;
-- nothing updates it after the insert.
ALTER TABLE exchange ADD COLUMN started_from text
    CONSTRAINT exchange_started_from_form
    CHECK (started_from ~ '^(blank|copy|[a-z][a-z0-9-]{0,39}@[0-9]{1,4})$');
