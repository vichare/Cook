ALTER TABLE import_tokens ADD COLUMN scopes TEXT NOT NULL DEFAULT 'recipes:import';
ALTER TABLE recipes ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

CREATE TABLE ai_recipe_requests (
    token_id TEXT NOT NULL REFERENCES import_tokens(id) ON DELETE CASCADE,
    action TEXT NOT NULL,
    client_request_id TEXT NOT NULL,
    recipe_id TEXT REFERENCES recipes(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (token_id, action, client_request_id)
);

CREATE TABLE ai_token_audit_events (
    id TEXT PRIMARY KEY,
    token_id TEXT NOT NULL REFERENCES import_tokens(id) ON DELETE CASCADE,
    token_prefix TEXT NOT NULL,
    action TEXT NOT NULL,
    recipe_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX ai_token_audit_events_token_created
    ON ai_token_audit_events(token_id, created_at);

CREATE TABLE ai_token_rate_limits (
    token_id TEXT NOT NULL REFERENCES import_tokens(id) ON DELETE CASCADE,
    bucket TEXT NOT NULL,
    window_started_at INTEGER NOT NULL,
    request_count INTEGER NOT NULL,
    PRIMARY KEY (token_id, bucket)
);

CREATE TABLE recipe_images (
    id TEXT PRIMARY KEY,
    recipe_id TEXT NOT NULL UNIQUE REFERENCES recipes(id) ON DELETE CASCADE,
    media_type TEXT NOT NULL CHECK (media_type = 'image/jpeg'),
    image_bytes BLOB NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    sha256 BLOB NOT NULL CHECK (length(sha256) = 32),
    alt_text TEXT NOT NULL,
    source_url TEXT,
    source_license TEXT,
    source_attribution TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    updated_at INTEGER NOT NULL
);

CREATE TABLE recipe_revisions (
    id TEXT PRIMARY KEY,
    recipe_id TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_id TEXT NOT NULL REFERENCES import_tokens(id) ON DELETE CASCADE,
    expected_version INTEGER NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL DEFAULT '',
    image_action TEXT NOT NULL DEFAULT 'keep'
        CHECK (image_action IN ('keep', 'replace', 'delete')),
    image_media_type TEXT,
    image_bytes BLOB,
    image_width INTEGER,
    image_height INTEGER,
    image_sha256 BLOB,
    image_alt_text TEXT,
    image_source_url TEXT,
    image_source_license TEXT,
    image_source_attribution TEXT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'accepted', 'rejected')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    resolved_at INTEGER
);
CREATE INDEX recipe_revisions_owner_status
    ON recipe_revisions(owner_id, status, updated_at);
CREATE UNIQUE INDEX recipe_revisions_one_pending_per_recipe
    ON recipe_revisions(recipe_id)
    WHERE status = 'pending';

CREATE TABLE recipe_revision_ingredients (
    id TEXT PRIMARY KEY,
    revision_id TEXT NOT NULL REFERENCES recipe_revisions(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    quantity_note TEXT NOT NULL DEFAULT '',
    canonical_ingredient_id TEXT REFERENCES ingredient_catalog(id) ON DELETE SET NULL,
    sort_order INTEGER NOT NULL
);
CREATE INDEX recipe_revision_ingredients_revision
    ON recipe_revision_ingredients(revision_id, sort_order);
