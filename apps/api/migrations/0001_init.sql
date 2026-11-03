CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX sessions_user_id ON sessions(user_id);

CREATE TABLE recipes (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    instructions TEXT NOT NULL DEFAULT '',
    visibility TEXT NOT NULL DEFAULT 'private' CHECK (visibility IN ('private', 'public')),
    source_url TEXT,
    source_license TEXT,
    source_attribution TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX recipes_owner_id ON recipes(owner_id);
CREATE INDEX recipes_visibility ON recipes(visibility);

CREATE TABLE recipe_ingredients (
    id TEXT PRIMARY KEY,
    recipe_id TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    quantity_note TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL
);
CREATE INDEX recipe_ingredients_recipe_id ON recipe_ingredients(recipe_id);

CREATE TABLE favorites (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recipe_id TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, recipe_id)
);

CREATE TABLE pantry_items (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('have', 'out', 'unknown')),
    quantity_note TEXT NOT NULL DEFAULT '',
    confirmed_at TEXT,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(owner_id, normalized_name)
);
CREATE INDEX pantry_items_owner_id ON pantry_items(owner_id);

CREATE TABLE recommendation_settings (
    owner_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    include_favorites INTEGER NOT NULL DEFAULT 1,
    include_own INTEGER NOT NULL DEFAULT 1,
    include_public INTEGER NOT NULL DEFAULT 1,
    verification_days INTEGER NOT NULL DEFAULT 14
);
