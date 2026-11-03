ALTER TABLE users ADD COLUMN profile_email TEXT;
UPDATE users SET profile_email = email;

DROP TABLE sessions;

CREATE TABLE auth_identities (
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (issuer, subject)
);
CREATE INDEX auth_identities_user_id ON auth_identities(user_id);

CREATE TABLE auth_attempts (
    state_digest BLOB PRIMARY KEY CHECK (length(state_digest) = 32),
    pkce_verifier TEXT NOT NULL,
    nonce TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX auth_attempts_expires_at ON auth_attempts(expires_at);

CREATE TABLE auth_sessions (
    session_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_digest BLOB NOT NULL UNIQUE CHECK (length(token_digest) = 32),
    created_at INTEGER NOT NULL,
    last_activity_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked_at INTEGER,
    version INTEGER NOT NULL
);
CREATE INDEX auth_sessions_user_id ON auth_sessions(user_id);
CREATE INDEX auth_sessions_expires_at ON auth_sessions(expires_at);

CREATE TABLE auth_rate_limits (
    rate_key TEXT PRIMARY KEY,
    window_started_at INTEGER NOT NULL,
    attempt_count INTEGER NOT NULL
);
