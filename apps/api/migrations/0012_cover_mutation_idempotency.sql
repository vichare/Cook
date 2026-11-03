CREATE TABLE recipe_cover_mutations (
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recipe_id TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    operation TEXT NOT NULL,
    request_fingerprint BLOB NOT NULL,
    result_version INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (owner_id, recipe_id, idempotency_key)
);

CREATE INDEX idx_recipe_cover_mutations_recipe_created
    ON recipe_cover_mutations(recipe_id, created_at);
