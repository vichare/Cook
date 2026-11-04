CREATE TABLE recipe_ingredient_groups (
    id TEXT PRIMARY KEY,
    recipe_id TEXT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    optional INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL,
    UNIQUE(recipe_id, sort_order),
    UNIQUE(recipe_id, external_id)
);
CREATE INDEX recipe_ingredient_groups_recipe
    ON recipe_ingredient_groups(recipe_id, sort_order);

CREATE TABLE recipe_ingredient_alternatives (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES recipe_ingredient_groups(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    quantity_note TEXT NOT NULL DEFAULT '',
    canonical_ingredient_id TEXT REFERENCES ingredient_catalog(id) ON DELETE SET NULL,
    sort_order INTEGER NOT NULL,
    UNIQUE(group_id, sort_order),
    UNIQUE(group_id, external_id)
);
CREATE INDEX recipe_ingredient_alternatives_group
    ON recipe_ingredient_alternatives(group_id, sort_order);
CREATE INDEX recipe_ingredient_alternatives_canonical
    ON recipe_ingredient_alternatives(canonical_ingredient_id);

INSERT INTO recipe_ingredient_groups
    (id, recipe_id, external_id, optional, note, sort_order)
SELECT 'legacy-group:' || id, recipe_id, 'legacy-group:' || id, 0, '', sort_order
FROM recipe_ingredients;

INSERT INTO recipe_ingredient_alternatives
    (id, group_id, external_id, name, normalized_name, quantity_note,
     canonical_ingredient_id, sort_order)
SELECT id, 'legacy-group:' || id, id, name, normalized_name, quantity_note,
       canonical_ingredient_id, 0
FROM recipe_ingredients;

CREATE TABLE recipe_revision_ingredient_groups (
    id TEXT PRIMARY KEY,
    revision_id TEXT NOT NULL REFERENCES recipe_revisions(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    optional INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL,
    UNIQUE(revision_id, sort_order),
    UNIQUE(revision_id, external_id)
);
CREATE INDEX recipe_revision_ingredient_groups_revision
    ON recipe_revision_ingredient_groups(revision_id, sort_order);

CREATE TABLE recipe_revision_ingredient_alternatives (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES recipe_revision_ingredient_groups(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    name TEXT NOT NULL,
    quantity_note TEXT NOT NULL DEFAULT '',
    canonical_ingredient_id TEXT REFERENCES ingredient_catalog(id) ON DELETE SET NULL,
    sort_order INTEGER NOT NULL,
    UNIQUE(group_id, sort_order),
    UNIQUE(group_id, external_id)
);
CREATE INDEX recipe_revision_ingredient_alternatives_group
    ON recipe_revision_ingredient_alternatives(group_id, sort_order);

CREATE TABLE import_draft_ingredient_groups (
    id TEXT PRIMARY KEY,
    draft_id TEXT NOT NULL REFERENCES import_drafts(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    optional INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL,
    UNIQUE(draft_id, sort_order),
    UNIQUE(draft_id, external_id)
);
CREATE INDEX import_draft_ingredient_groups_draft
    ON import_draft_ingredient_groups(draft_id, sort_order);

CREATE TABLE import_draft_ingredient_alternatives (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES import_draft_ingredient_groups(id) ON DELETE CASCADE,
    external_id TEXT NOT NULL,
    name TEXT NOT NULL,
    quantity_note TEXT NOT NULL DEFAULT '',
    canonical_ingredient_id TEXT REFERENCES ingredient_catalog(id) ON DELETE SET NULL,
    sort_order INTEGER NOT NULL,
    UNIQUE(group_id, sort_order),
    UNIQUE(group_id, external_id)
);
CREATE INDEX import_draft_ingredient_alternatives_group
    ON import_draft_ingredient_alternatives(group_id, sort_order);

INSERT INTO recipe_revision_ingredient_groups
    (id, revision_id, external_id, optional, note, sort_order)
SELECT 'legacy-revision-group:' || id, revision_id, 'legacy-revision-group:' || id, 0, '', sort_order
FROM recipe_revision_ingredients;
INSERT INTO recipe_revision_ingredient_alternatives
    (id, group_id, external_id, name, quantity_note, canonical_ingredient_id, sort_order)
SELECT id, 'legacy-revision-group:' || id, id, name, quantity_note, canonical_ingredient_id, 0
FROM recipe_revision_ingredients;

INSERT INTO import_draft_ingredient_groups
    (id, draft_id, external_id, optional, note, sort_order)
SELECT 'legacy-draft-group:' || id, draft_id, 'legacy-draft-group:' || id, 0, '', sort_order
FROM import_draft_ingredients;
INSERT INTO import_draft_ingredient_alternatives
    (id, group_id, external_id, name, quantity_note, canonical_ingredient_id, sort_order)
SELECT id, 'legacy-draft-group:' || id, id, name, quantity_note, NULL, 0
FROM import_draft_ingredients;

ALTER TABLE shopping_items ADD COLUMN choice_key TEXT;
ALTER TABLE shopping_items ADD COLUMN choice_json TEXT;
CREATE UNIQUE INDEX shopping_items_owner_choice
    ON shopping_items(owner_id, choice_key)
    WHERE choice_key IS NOT NULL;
