ALTER TABLE import_drafts ADD COLUMN source_license TEXT;
ALTER TABLE import_drafts ADD COLUMN source_attribution TEXT;

ALTER TABLE recipe_revisions ADD COLUMN source_url TEXT;
ALTER TABLE recipe_revisions ADD COLUMN source_title TEXT;
ALTER TABLE recipe_revisions ADD COLUMN source_author TEXT;
ALTER TABLE recipe_revisions ADD COLUMN source_license TEXT;
ALTER TABLE recipe_revisions ADD COLUMN source_attribution TEXT;
