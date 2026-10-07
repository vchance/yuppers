-- The texts received at the webhook (`POST /v1/sms/inbound`) that changed
-- something, by Twilio's MessageSid: a STOP or a START. Twilio signs each
-- request, but the signature has no time in it, so a request captured once
-- would check out again: a START replayed after a later STOP would lift the
-- opt-out. A MessageSid already here is answered as taken and does nothing.
--
-- Only the SID and when it came: no number, no text. The worker removes a
-- row after `INBOUND_SEEN_RETENTION`
-- (backend/src/notifications/sms_updates.rs).
CREATE TABLE sms_inbound_seen (
    message_sid text PRIMARY KEY CHECK (char_length(message_sid) BETWEEN 1 AND 64),
    received_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX sms_inbound_seen_received_idx ON sms_inbound_seen (received_at);

GRANT SELECT, INSERT, DELETE ON sms_inbound_seen TO exchange_app;
