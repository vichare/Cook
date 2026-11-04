ALTER TABLE users ADD COLUMN authorization_version INTEGER NOT NULL DEFAULT 1;

ALTER TABLE auth_sessions ADD COLUMN authorization_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE auth_sessions ADD COLUMN passkey_verified_at INTEGER;

ALTER TABLE recipes ADD COLUMN publication_intent TEXT NOT NULL DEFAULT 'private'
    CHECK (publication_intent IN ('private', 'public'));
ALTER TABLE recipes ADD COLUMN moderation_state TEXT NOT NULL DEFAULT 'clear'
    CHECK (moderation_state IN ('clear', 'restricted'));
ALTER TABLE recipes ADD COLUMN moderation_review_state TEXT NOT NULL DEFAULT 'unreviewed'
    CHECK (moderation_review_state IN ('unreviewed', 'reviewed'));
ALTER TABLE recipes ADD COLUMN moderation_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE recipes ADD COLUMN restriction_epoch INTEGER NOT NULL DEFAULT 0;

UPDATE recipes
SET publication_intent = visibility,
    moderation_state = 'clear',
    moderation_review_state = 'unreviewed',
    moderation_version = 1,
    restriction_epoch = 0;

CREATE TRIGGER recipes_effective_visibility_insert
BEFORE INSERT ON recipes
WHEN NEW.visibility <> CASE
    WHEN NEW.publication_intent = 'public' AND NEW.moderation_state = 'clear'
    THEN 'public' ELSE 'private' END
BEGIN
    SELECT RAISE(ABORT, 'recipe effective visibility invariant');
END;

CREATE TRIGGER recipes_effective_visibility_update
BEFORE UPDATE OF visibility, publication_intent, moderation_state ON recipes
WHEN NEW.visibility <> CASE
    WHEN NEW.publication_intent = 'public' AND NEW.moderation_state = 'clear'
    THEN 'public' ELSE 'private' END
BEGIN
    SELECT RAISE(ABORT, 'recipe effective visibility invariant');
END;

CREATE VIEW public_eligible_recipes AS
SELECT *
FROM recipes
WHERE publication_intent = 'public'
  AND moderation_state = 'clear'
  AND visibility = 'public';

CREATE TABLE capability_assignments (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    capability TEXT NOT NULL CHECK (capability = 'content_moderation'),
    created_at INTEGER NOT NULL,
    revoked_at INTEGER,
    UNIQUE(user_id, capability)
);
CREATE INDEX capability_assignments_active_user
    ON capability_assignments(user_id, capability)
    WHERE revoked_at IS NULL;

CREATE TABLE capability_claim_codes (
    id TEXT PRIMARY KEY,
    capability TEXT NOT NULL CHECK (capability = 'content_moderation'),
    code_digest BLOB NOT NULL UNIQUE CHECK (length(code_digest) = 32),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at INTEGER,
    consumed_by_assignment_id TEXT REFERENCES capability_assignments(id)
);
CREATE INDEX capability_claim_codes_expiry ON capability_claim_codes(expires_at);

CREATE TABLE admin_passkey_attempts (
    token_digest BLOB PRIMARY KEY CHECK (length(token_digest) = 32),
    session_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose TEXT NOT NULL CHECK (purpose IN ('admin_reauth', 'moderator_claim')),
    claim_code_digest BLOB,
    expires_at INTEGER NOT NULL
);
CREATE INDEX admin_passkey_attempts_expiry ON admin_passkey_attempts(expires_at);

CREATE TABLE moderation_cases (
    id TEXT PRIMARY KEY,
    recipe_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN (
            'open', 'restricted', 'resolved', 'reinstated', 'upheld', 'content_removed'
        )),
    version INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    resolved_at INTEGER
);
CREATE UNIQUE INDEX moderation_cases_one_active_recipe
    ON moderation_cases(recipe_id)
    WHERE status IN ('open', 'restricted');
CREATE INDEX moderation_cases_status_updated
    ON moderation_cases(status, updated_at DESC, id DESC);

CREATE TABLE moderation_reports (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES moderation_cases(id) ON DELETE CASCADE,
    recipe_id TEXT NOT NULL,
    reporter_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recipe_version INTEGER NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN (
        'spam', 'unsafe_or_harmful', 'harassment_or_hate', 'privacy',
        'copyright_or_attribution', 'misleading', 'other'
    )),
    details TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    UNIQUE(reporter_user_id, recipe_id, recipe_version, reason)
);
CREATE INDEX moderation_reports_case_created
    ON moderation_reports(case_id, created_at);

CREATE TABLE moderation_evidence_snapshots (
    case_id TEXT PRIMARY KEY REFERENCES moderation_cases(id) ON DELETE CASCADE,
    recipe_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    recipe_version INTEGER NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    instructions TEXT NOT NULL,
    author_handle TEXT NOT NULL,
    image_id TEXT,
    image_bytes BLOB CHECK (image_bytes IS NULL OR length(image_bytes) <= 2500000),
    image_media_type TEXT,
    image_sha256 BLOB CHECK (image_sha256 IS NULL OR length(image_sha256) = 32),
    captured_at INTEGER NOT NULL,
    CHECK (
        (image_bytes IS NULL AND image_media_type IS NULL AND image_sha256 IS NULL)
        OR
        (image_bytes IS NOT NULL AND image_media_type IS NOT NULL AND image_sha256 IS NOT NULL)
    )
);

CREATE TRIGGER moderation_refresh_active_case_evidence
AFTER UPDATE OF version ON recipes
WHEN NEW.version <> OLD.version
 AND EXISTS (
    SELECT 1 FROM moderation_cases
    WHERE recipe_id = NEW.id AND status IN ('open', 'restricted')
 )
BEGIN
    UPDATE moderation_evidence_snapshots
    SET recipe_version = NEW.version,
        title = substr(NEW.title, 1, 120),
        description = substr(NEW.description, 1, 2000),
        instructions = substr(NEW.instructions, 1, 20000),
        image_id = (SELECT id FROM recipe_images WHERE recipe_id = NEW.id),
        image_bytes = (SELECT image_bytes FROM recipe_images WHERE recipe_id = NEW.id),
        image_media_type = (SELECT media_type FROM recipe_images WHERE recipe_id = NEW.id),
        image_sha256 = (SELECT sha256 FROM recipe_images WHERE recipe_id = NEW.id),
        captured_at = unixepoch()
    WHERE recipe_id = NEW.id
      AND case_id IN (
        SELECT id FROM moderation_cases
        WHERE recipe_id = NEW.id AND status IN ('open', 'restricted')
      );

    UPDATE moderation_cases
    SET version = version + 1,
        updated_at = unixepoch()
    WHERE recipe_id = NEW.id AND status IN ('open', 'restricted');
END;

CREATE TRIGGER moderation_refresh_active_case_evidence_image_insert
AFTER INSERT ON recipe_images
WHEN EXISTS (
    SELECT 1 FROM moderation_cases
    WHERE recipe_id = NEW.recipe_id AND status IN ('open', 'restricted')
)
BEGIN
    UPDATE moderation_evidence_snapshots
    SET image_id = NEW.id,
        image_bytes = NEW.image_bytes,
        image_media_type = NEW.media_type,
        image_sha256 = NEW.sha256
    WHERE recipe_id = NEW.recipe_id
      AND case_id IN (
        SELECT id FROM moderation_cases
        WHERE recipe_id = NEW.recipe_id AND status IN ('open', 'restricted')
      );
END;

CREATE TRIGGER moderation_refresh_active_case_evidence_image_update
AFTER UPDATE OF id, image_bytes, media_type, sha256 ON recipe_images
WHEN EXISTS (
    SELECT 1 FROM moderation_cases
    WHERE recipe_id = NEW.recipe_id AND status IN ('open', 'restricted')
)
BEGIN
    UPDATE moderation_evidence_snapshots
    SET image_id = NEW.id,
        image_bytes = NEW.image_bytes,
        image_media_type = NEW.media_type,
        image_sha256 = NEW.sha256
    WHERE recipe_id = NEW.recipe_id
      AND case_id IN (
        SELECT id FROM moderation_cases
        WHERE recipe_id = NEW.recipe_id AND status IN ('open', 'restricted')
      );
END;

CREATE TRIGGER moderation_refresh_active_case_evidence_image_delete
AFTER DELETE ON recipe_images
BEGIN
    UPDATE moderation_evidence_snapshots
    SET image_id = NULL,
        image_bytes = NULL,
        image_media_type = NULL,
        image_sha256 = NULL
    WHERE recipe_id = OLD.recipe_id
      AND case_id IN (
        SELECT id FROM moderation_cases
        WHERE recipe_id = OLD.recipe_id AND status IN ('open', 'restricted')
      );
END;

CREATE TABLE moderation_appeals (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES moderation_cases(id) ON DELETE CASCADE,
    recipe_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    restriction_epoch INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'reinstated', 'upheld', 'content_removed')),
    reason TEXT NOT NULL,
    resolution_reason TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    resolved_at INTEGER,
    UNIQUE(recipe_id, restriction_epoch)
);
CREATE UNIQUE INDEX moderation_appeals_one_active_recipe
    ON moderation_appeals(recipe_id)
    WHERE status = 'active';

CREATE TABLE owner_notifications (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    recipe_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN (
        'recipe_restricted', 'recipe_reinstated', 'appeal_upheld', 'content_removed'
    )),
    reason_code TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    read_at INTEGER
);
CREATE INDEX owner_notifications_owner_created
    ON owner_notifications(owner_user_id, created_at DESC, id DESC);

CREATE TABLE admin_mutation_requests (
    assignment_id TEXT NOT NULL REFERENCES capability_assignments(id) ON DELETE CASCADE,
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    request_fingerprint BLOB NOT NULL CHECK (length(request_fingerprint) = 32),
    result_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (assignment_id, action, idempotency_key)
);
CREATE INDEX admin_mutation_requests_created
    ON admin_mutation_requests(created_at);

CREATE TABLE moderation_rate_limits (
    rate_key TEXT PRIMARY KEY,
    window_started_at INTEGER NOT NULL,
    request_count INTEGER NOT NULL
);

CREATE TABLE moderation_audit_events (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    assignment_id TEXT,
    actor_user_id TEXT,
    recipe_id TEXT,
    case_id TEXT,
    appeal_id TEXT,
    reason_code TEXT,
    recipe_version INTEGER,
    moderation_version INTEGER,
    case_version INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX moderation_audit_created
    ON moderation_audit_events(created_at DESC, id DESC);

CREATE TRIGGER moderation_audit_events_no_update
BEFORE UPDATE ON moderation_audit_events
BEGIN
    SELECT RAISE(ABORT, 'moderation audit events are append-only');
END;

CREATE TRIGGER moderation_audit_events_no_delete
BEFORE DELETE ON moderation_audit_events
BEGIN
    SELECT RAISE(ABORT, 'moderation audit events are append-only');
END;

CREATE TABLE release_compatibility (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    minimum_release TEXT,
    first_restriction_at INTEGER
);
INSERT INTO release_compatibility (id) VALUES (1);
