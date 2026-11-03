ALTER TABLE auth_identities ADD COLUMN verified_email TEXT;

UPDATE auth_identities
SET verified_email = lower(trim(subject))
WHERE issuer = 'urn:cook:email-otp';

UPDATE auth_identities
SET verified_email = (
    SELECT lower(trim(users.profile_email))
    FROM users
    WHERE users.id = auth_identities.user_id
)
WHERE issuer = 'https://accounts.google.com'
  AND verified_email IS NULL;

CREATE TABLE auth_identity_audit_events (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX auth_identity_audit_events_created_at
    ON auth_identity_audit_events(created_at);
