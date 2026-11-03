ALTER TABLE users ADD COLUMN public_handle TEXT;
UPDATE users
SET public_handle = lower(hex(randomblob(16)))
WHERE public_handle IS NULL;
CREATE UNIQUE INDEX users_public_handle ON users(public_handle);
CREATE TRIGGER users_public_handle_after_insert
AFTER INSERT ON users
WHEN NEW.public_handle IS NULL
BEGIN
    UPDATE users
    SET public_handle = lower(hex(randomblob(16)))
    WHERE id = NEW.id;
END;
CREATE TRIGGER users_public_handle_not_null
BEFORE UPDATE OF public_handle ON users
WHEN NEW.public_handle IS NULL
BEGIN
    SELECT RAISE(ABORT, 'public_handle cannot be null');
END;

CREATE TABLE favorite_accounts (
    viewer_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (viewer_id, author_id),
    CHECK (viewer_id <> author_id)
);
CREATE INDEX favorite_accounts_author_viewer
    ON favorite_accounts(author_id, viewer_id);
CREATE INDEX recipes_owner_visibility_created
    ON recipes(owner_id, visibility, created_at DESC, id DESC);

CREATE TABLE public_api_rate_limits (
    rate_key TEXT PRIMARY KEY,
    window_started_at INTEGER NOT NULL,
    request_count INTEGER NOT NULL
);

ALTER TABLE recommendation_settings
    ADD COLUMN include_favorite_accounts INTEGER NOT NULL DEFAULT 1;
