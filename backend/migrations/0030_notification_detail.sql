-- Whether the holder wants texts and phone notifications to say who a yup is
-- with and what step happened, with the yup's code (DESIGN.md §12). Off by
-- default: a text or notification can be read on a lock screen by whoever
-- holds the phone, so it says only that a yup has an update, and its code,
-- until the holder chooses otherwise on their account. Even then it never
-- carries terms, amounts, dates or anything written in a yup.
--
-- The application role's grants on `account` already cover a new column.
ALTER TABLE account ADD COLUMN notification_detail boolean NOT NULL DEFAULT false;
