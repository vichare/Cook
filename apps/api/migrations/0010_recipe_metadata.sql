ALTER TABLE recipes
    ADD COLUMN difficulty INTEGER CHECK (difficulty BETWEEN 1 AND 5);
ALTER TABLE recipes
    ADD COLUMN estimated_calories INTEGER
        CHECK (estimated_calories BETWEEN 1 AND 100000);

ALTER TABLE import_drafts
    ADD COLUMN difficulty INTEGER CHECK (difficulty BETWEEN 1 AND 5);
ALTER TABLE import_drafts
    ADD COLUMN estimated_calories INTEGER
        CHECK (estimated_calories BETWEEN 1 AND 100000);

ALTER TABLE recipe_revisions
    ADD COLUMN difficulty INTEGER CHECK (difficulty BETWEEN 1 AND 5);
ALTER TABLE recipe_revisions
    ADD COLUMN estimated_calories INTEGER
        CHECK (estimated_calories BETWEEN 1 AND 100000);
