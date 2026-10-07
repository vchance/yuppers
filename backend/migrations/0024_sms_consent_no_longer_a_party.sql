-- A party removed from an agreement before the other party confirmed them,
-- or who left it then, no longer gets its text updates: the subscription is
-- removed with the rest of what was theirs alone in it
-- (`exchanges::repo::vacate`), and its ending recorded in `sms_consent` as
-- every other is, with this source.
ALTER TABLE sms_consent DROP CONSTRAINT sms_consent_source_check;
ALTER TABLE sms_consent ADD CONSTRAINT sms_consent_source_check
    CHECK (source IN (
        'WEB', 'IOS', 'ANDROID', 'UNKNOWN',
        'SMS_REPLY', 'ACCOUNT_DELETED', 'PHONE_CHANGED', 'NO_LONGER_A_PARTY'));
