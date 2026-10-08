-- The user's own name for an account (Cirrus calls it the "Account Tag"),
-- e.g. "Pratik D". Optional, not secret, at most 64 characters.
ALTER TABLE accounts ADD COLUMN tag TEXT;
