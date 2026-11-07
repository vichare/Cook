ALTER TABLE auth_identities ADD COLUMN verification_provenance TEXT NOT NULL DEFAULT 'mailbox_verified'
    CHECK (verification_provenance IN ('mailbox_verified', 'admin_assisted', 'provider_verified'));

UPDATE auth_identities
SET verification_provenance = 'provider_verified'
WHERE issuer = 'https://accounts.google.com';

ALTER TABLE auth_sessions ADD COLUMN auth_method TEXT NOT NULL DEFAULT 'standard'
    CHECK (auth_method IN ('standard', 'admin_assisted_otp'));
ALTER TABLE auth_sessions ADD COLUMN issuer_assignment_id TEXT;
ALTER TABLE auth_sessions ADD COLUMN issuer_assignment_alias TEXT;
ALTER TABLE auth_sessions ADD COLUMN authenticated_at INTEGER;

CREATE TABLE account_assistant_assignments (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    alias TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE UNIQUE INDEX account_assistant_assignments_active_user
    ON account_assistant_assignments(user_id)
    WHERE revoked_at IS NULL;

CREATE TABLE account_assistant_claim_codes (
    id TEXT PRIMARY KEY,
    code_digest BLOB NOT NULL UNIQUE CHECK (length(code_digest) = 32),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at INTEGER,
    consumed_by_assignment_id TEXT REFERENCES account_assistant_assignments(id)
);
CREATE INDEX account_assistant_claim_codes_expiry
    ON account_assistant_claim_codes(expires_at);

ALTER TABLE admin_passkey_attempts RENAME TO admin_passkey_attempts_v17;
CREATE TABLE admin_passkey_attempts (
    token_digest BLOB PRIMARY KEY CHECK (length(token_digest) = 32),
    session_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose TEXT NOT NULL CHECK (
        purpose IN ('admin_reauth', 'moderator_claim', 'account_assistant_claim')
    ),
    claim_code_digest BLOB,
    expires_at INTEGER NOT NULL
);
INSERT INTO admin_passkey_attempts
    (token_digest, session_id, user_id, purpose, claim_code_digest, expires_at)
SELECT token_digest, session_id, user_id, purpose, claim_code_digest, expires_at
FROM admin_passkey_attempts_v17;
DROP TABLE admin_passkey_attempts_v17;
CREATE INDEX admin_passkey_attempts_expiry ON admin_passkey_attempts(expires_at);

CREATE TABLE account_assistance_otp_challenges (
    challenge_digest BLOB PRIMARY KEY CHECK (length(challenge_digest) = 32),
    email_digest BLOB NOT NULL UNIQUE CHECK (length(email_digest) = 32),
    normalized_email TEXT NOT NULL,
    code_digest BLOB NOT NULL CHECK (length(code_digest) = 32),
    issuer_assignment_id TEXT NOT NULL REFERENCES account_assistant_assignments(id),
    issuer_assignment_alias TEXT NOT NULL,
    reason_code TEXT NOT NULL CHECK (
        reason_code IN ('in_person_accessibility', 'mailbox_unavailable', 'device_setup')
    ),
    note_present INTEGER NOT NULL DEFAULT 0 CHECK (note_present IN (0, 1)),
    target_existed_at_issue INTEGER NOT NULL CHECK (target_existed_at_issue IN (0, 1)),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    attempts_remaining INTEGER NOT NULL CHECK (attempts_remaining BETWEEN 1 AND 5)
);
CREATE INDEX account_assistance_otp_expiry
    ON account_assistance_otp_challenges(expires_at);

CREATE TABLE account_assistance_rate_limits (
    rate_key TEXT PRIMARY KEY,
    window_started_at INTEGER NOT NULL,
    request_count INTEGER NOT NULL
);

CREATE TABLE account_assistance_notifications (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (
        kind IN ('account_assistance_issued', 'account_assistance_consumed')
    ),
    issuer_alias TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    read_at INTEGER
);
CREATE INDEX account_assistance_notifications_user_created
    ON account_assistance_notifications(user_id, created_at DESC, id DESC);

CREATE TABLE account_assistance_audit_events (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL CHECK (
        event_type IN (
            'account_assistant_claimed',
            'account_assistant_revoked',
            'assisted_otp_issued',
            'assisted_otp_consumed'
        )
    ),
    assignment_id TEXT,
    assignment_alias TEXT,
    actor_user_id TEXT,
    target_email_digest BLOB CHECK (
        target_email_digest IS NULL OR length(target_email_digest) = 32
    ),
    target_existed INTEGER CHECK (target_existed IS NULL OR target_existed IN (0, 1)),
    reason_code TEXT,
    note_present INTEGER CHECK (note_present IS NULL OR note_present IN (0, 1)),
    created_at INTEGER NOT NULL
);
CREATE INDEX account_assistance_audit_created
    ON account_assistance_audit_events(created_at DESC, id DESC);

CREATE TRIGGER account_assistance_audit_no_update
BEFORE UPDATE ON account_assistance_audit_events
BEGIN
    SELECT RAISE(ABORT, 'account assistance audit events are append-only');
END;

CREATE TRIGGER account_assistance_audit_no_delete
BEFORE DELETE ON account_assistance_audit_events
BEGIN
    SELECT RAISE(ABORT, 'account assistance audit events are append-only');
END;
