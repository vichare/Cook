ALTER TABLE users ADD COLUMN public_nickname TEXT;
ALTER TABLE users ADD COLUMN profile_onboarding_completed INTEGER NOT NULL DEFAULT 0;

ALTER TABLE recipes ADD COLUMN source_title TEXT;
ALTER TABLE recipes ADD COLUMN source_author TEXT;
ALTER TABLE recipes ADD COLUMN source_warnings TEXT;
ALTER TABLE recipes ADD COLUMN source_confidence REAL;

CREATE TABLE import_tokens (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_digest BLOB NOT NULL UNIQUE CHECK (length(token_digest) = 32),
    token_prefix TEXT NOT NULL,
    scope TEXT NOT NULL CHECK (scope = 'recipes:import'),
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    expires_at INTEGER,
    revoked_at INTEGER
);
CREATE INDEX import_tokens_owner_id ON import_tokens(owner_id);

CREATE TABLE import_token_rate_limits (
    token_id TEXT PRIMARY KEY REFERENCES import_tokens(id) ON DELETE CASCADE,
    window_started_at INTEGER NOT NULL,
    request_count INTEGER NOT NULL
);

CREATE TABLE import_drafts (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source_url TEXT NOT NULL,
    source_title TEXT,
    source_author TEXT,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL DEFAULT '',
    warnings_json TEXT NOT NULL DEFAULT '[]',
    confidence REAL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'consumed', 'discarded')),
    accepted_recipe_id TEXT REFERENCES recipes(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    consumed_at INTEGER
);
CREATE INDEX import_drafts_owner_id ON import_drafts(owner_id);
CREATE INDEX import_drafts_owner_status ON import_drafts(owner_id, status);

CREATE TABLE import_draft_ingredients (
    id TEXT PRIMARY KEY,
    draft_id TEXT NOT NULL REFERENCES import_drafts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    quantity_note TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL
);
CREATE INDEX import_draft_ingredients_draft_id ON import_draft_ingredients(draft_id);
