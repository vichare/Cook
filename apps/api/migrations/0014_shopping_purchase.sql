CREATE TABLE pantry_items_v14 (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('have', 'out', 'unknown')),
    quantity_note TEXT NOT NULL DEFAULT '',
    confirmed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_on TEXT,
    canonical_ingredient_id TEXT,
    auto_add_to_shopping_list_when_out INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1
);

INSERT INTO pantry_items_v14
    (id, owner_id, name, normalized_name, status, quantity_note, confirmed_at, updated_at,
     expires_on, canonical_ingredient_id, auto_add_to_shopping_list_when_out)
SELECT id, owner_id, name, normalized_name, status, quantity_note, confirmed_at, updated_at,
       expires_on, canonical_ingredient_id, auto_add_to_shopping_list_when_out
FROM pantry_items;

DROP TABLE pantry_items;
ALTER TABLE pantry_items_v14 RENAME TO pantry_items;
CREATE INDEX pantry_items_owner_id ON pantry_items(owner_id);
CREATE INDEX pantry_items_canonical_id ON pantry_items(canonical_ingredient_id);
CREATE UNIQUE INDEX pantry_items_owner_normalized
    ON pantry_items(owner_id, normalized_name);
CREATE UNIQUE INDEX pantry_items_owner_canonical
    ON pantry_items(owner_id, canonical_ingredient_id)
    WHERE canonical_ingredient_id IS NOT NULL;
CREATE INDEX pantry_items_owner_auto_restock
    ON pantry_items(owner_id, status, auto_add_to_shopping_list_when_out)
    WHERE auto_add_to_shopping_list_when_out = 1;

ALTER TABLE shopping_items ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

CREATE TABLE shopping_purchase_actions (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    shopping_item_id TEXT NOT NULL,
    pantry_item_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_fingerprint BLOB NOT NULL,
    disposition TEXT NOT NULL CHECK (disposition IN ('remove', 'keep')),
    pantry_before_json TEXT,
    shopping_before_json TEXT,
    result_pantry_version INTEGER NOT NULL,
    result_shopping_version INTEGER,
    response_json TEXT NOT NULL,
    undone_at INTEGER,
    created_at INTEGER NOT NULL,
    UNIQUE(owner_id, idempotency_key)
);

CREATE INDEX shopping_purchase_actions_owner_created
    ON shopping_purchase_actions(owner_id, created_at);
