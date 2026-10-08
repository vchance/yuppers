-- A random key of its own for every outbox row, which a provider that
-- drops repeated requests is handed with each try (Resend's
-- `Idempotency-Key`, SMTP's `Message-ID`; `notifications::Email::key`).
--
-- The row's ID was used until now, and IDs start again from 1 after a
-- schema reset, a restore into a new database, or in another environment
-- sending through the same Resend team: Resend would take a new message
-- for a key it had seen within 24 hours as a repeat with other content and
-- refuse it. A random UUID is never seen twice.
--
-- Rows already queued get a key of their own too. The application role
-- reads it with the rest of the row; its grants on `outbox` already cover
-- a new column.
ALTER TABLE outbox ADD COLUMN delivery_key uuid NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE outbox ADD CONSTRAINT outbox_delivery_key_unique UNIQUE (delivery_key);
