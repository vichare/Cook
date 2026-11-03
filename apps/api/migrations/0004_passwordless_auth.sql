CREATE TABLE email_otp_attempts (
    challenge_digest BLOB PRIMARY KEY CHECK (length(challenge_digest) = 32),
    email_digest BLOB NOT NULL CHECK (length(email_digest) = 32),
    email TEXT NOT NULL,
    purpose TEXT NOT NULL,
    continue_path TEXT NOT NULL,
    code_digest BLOB NOT NULL CHECK (length(code_digest) = 32),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    resend_allowed_at INTEGER NOT NULL,
    attempts_remaining INTEGER NOT NULL CHECK (attempts_remaining BETWEEN 1 AND 5),
    UNIQUE(email_digest, purpose)
);
CREATE INDEX email_otp_attempts_expires_at ON email_otp_attempts(expires_at);

CREATE TABLE email_otp_rate_limits (
    limit_kind TEXT NOT NULL,
    rate_key TEXT NOT NULL,
    window_started_at INTEGER NOT NULL,
    attempt_count INTEGER NOT NULL,
    PRIMARY KEY (limit_kind, rate_key)
);

CREATE TABLE email_otp_link_targets (
    request_digest BLOB PRIMARY KEY CHECK (length(request_digest) = 32),
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);
CREATE INDEX email_otp_link_targets_expires_at ON email_otp_link_targets(expires_at);

ALTER TABLE users ADD COLUMN passkey_user_handle TEXT;
CREATE UNIQUE INDEX users_passkey_user_handle
    ON users(passkey_user_handle)
    WHERE passkey_user_handle IS NOT NULL;

CREATE TABLE passkey_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id BLOB NOT NULL UNIQUE,
    credential_json TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER
);
CREATE INDEX passkey_credentials_user_id ON passkey_credentials(user_id);
