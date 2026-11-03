ALTER TABLE pantry_items
    ADD COLUMN auto_add_to_shopping_list_when_out INTEGER NOT NULL DEFAULT 0;
CREATE INDEX pantry_items_owner_auto_restock
    ON pantry_items(owner_id, status, auto_add_to_shopping_list_when_out)
    WHERE auto_add_to_shopping_list_when_out = 1;

CREATE TABLE shopping_items (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    canonical_ingredient_id TEXT REFERENCES ingredient_catalog(id) ON DELETE SET NULL,
    completed_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX shopping_items_owner_completed
    ON shopping_items(owner_id, completed_at, updated_at);
CREATE UNIQUE INDEX shopping_items_owner_canonical
    ON shopping_items(owner_id, canonical_ingredient_id)
    WHERE canonical_ingredient_id IS NOT NULL;
CREATE UNIQUE INDEX shopping_items_owner_normalized_unresolved
    ON shopping_items(owner_id, normalized_name)
    WHERE canonical_ingredient_id IS NULL;

CREATE TABLE shopping_item_sources (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL REFERENCES shopping_items(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL CHECK (source_type IN ('manual', 'recipe')),
    recipe_id TEXT REFERENCES recipes(id) ON DELETE SET NULL,
    label TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX shopping_item_sources_unique
    ON shopping_item_sources(item_id, source_type, ifnull(recipe_id, ''));
