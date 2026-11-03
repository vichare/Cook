ALTER TABLE pantry_items ADD COLUMN expires_on TEXT;

CREATE TABLE ingredient_catalog (
    id TEXT PRIMARY KEY,
    normalized_name TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

INSERT INTO ingredient_catalog (id, normalized_name, display_name, created_at)
SELECT lower(hex(randomblob(16))), normalized_name, MIN(name), unixepoch()
FROM (
    SELECT normalized_name, name FROM recipe_ingredients
    UNION ALL
    SELECT normalized_name, name FROM pantry_items
)
WHERE normalized_name <> ''
GROUP BY normalized_name;

ALTER TABLE recipe_ingredients ADD COLUMN canonical_ingredient_id TEXT;
ALTER TABLE pantry_items ADD COLUMN canonical_ingredient_id TEXT;

UPDATE recipe_ingredients
SET canonical_ingredient_id = (
    SELECT id FROM ingredient_catalog
    WHERE ingredient_catalog.normalized_name = recipe_ingredients.normalized_name
);

UPDATE pantry_items
SET canonical_ingredient_id = (
    SELECT id FROM ingredient_catalog
    WHERE ingredient_catalog.normalized_name = pantry_items.normalized_name
);

CREATE INDEX recipe_ingredients_canonical_id
    ON recipe_ingredients(canonical_ingredient_id);
CREATE INDEX pantry_items_canonical_id
    ON pantry_items(canonical_ingredient_id);
CREATE UNIQUE INDEX pantry_items_owner_canonical
    ON pantry_items(owner_id, canonical_ingredient_id);

CREATE TABLE ingredient_aliases (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    alias_name TEXT NOT NULL,
    alias_normalized TEXT NOT NULL,
    canonical_ingredient_id TEXT NOT NULL REFERENCES ingredient_catalog(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    UNIQUE(owner_id, alias_normalized)
);
CREATE INDEX ingredient_aliases_owner_id ON ingredient_aliases(owner_id);

CREATE TABLE app_data_migrations (
    name TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
