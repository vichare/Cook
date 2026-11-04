CREATE TABLE shopping_delete_actions (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    shopping_item_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_fingerprint BLOB NOT NULL,
    response_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(owner_id, idempotency_key)
);

CREATE INDEX shopping_delete_actions_owner_created
    ON shopping_delete_actions(owner_id, created_at);
