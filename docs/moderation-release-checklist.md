# v0.6.0 content moderation release checklist

Release is blocked until each item has automated coverage where practical and a recorded manual result for browser/assistive-technology checks.

## Database and compatibility

- Migration 0017 applies to a populated v0.5 database and is safe to reopen.
- Existing public recipes backfill to public intent + clear + unreviewed; private recipes remain private + clear.
- Insert/update triggers reject any effective visibility inconsistent with publication intent and moderation state.
- The first restriction sets `minimum_release = v0.6.0`; deployment refuses an older tag.
- Backup, restore, WAL consistency, and rollback-before-first-restriction are verified.

## Authorization and privacy

- Non-moderators receive 404 from every `/api/admin/**` route.
- Moderator capability appears only in the current user's `/api/me`; public DTOs expose no role or assignment.
- Queue/case/audit cursors are bounded, account-isolated, and reject malformed input.
- Moderators cannot view or act on their own cases/recipes through moderation APIs.
- Case/report DTOs contain no reporter ID, email, nickname, IP, cookie, Passkey, AI-token identity, or unrestricted private content.
- Report and appeal text renders as plain text where submitted; recipe snapshot Markdown uses the safe renderer.

## Passkey and session security

- Claim requires an existing user, at least one Passkey, a valid 15-minute code, and a fresh assertion bound to the current session.
- Claim code plaintext is shown once, never logged, stored only as a digest, and atomically consumed.
- Case detail, restrict, reinstate, appeal resolution, audit detail, and export reject stale assurance with `403 admin_passkey_reauth_required`.
- Google and Email OTP login alone cannot authorize a sensitive moderation action.
- Assignment revocation increments authorization version and invalidates every session immediately.

## State machine and atomicity

- New public recipes and substantive updates become unreviewed without becoming private.
- Restricted owners can read/edit and preserve public intent; browser, legacy, revision, image, import, and AI writes cannot clear restriction.
- Report deduplication is reporter + recipe + recipe version + reason and concurrent first reports create/reuse one active case/snapshot.
- Every edit with an active open/restricted case refreshes bounded current-version evidence and increments the case version; stale review/restrict/reinstate/appeal decisions cannot act on another version.
- Persisted case evidence retains original owner identity, so deleted recipes remain protected from self-moderation.
- Evidence images are bounded immutable normalized byte/media/digest copies retained with the case and cascade-cleaned with it; rotation, reinstate, replacement, and recipe/image deletion do not alter the evidence.
- Recipe deletion transitions active cases/appeals to `content_removed` before deleting recipe/image rows; forced delete failure rolls back statuses, content, and evidence together.
- Evidence images are readable only through the case-bound, fresh-Passkey, non-owner moderator endpoint with `private, no-store`; ordinary private image URLs remain denied.
- Restrict/reinstate/appeal resolution require idempotency keys and optimistic recipe/moderation/case/appeal versions.
- Exact retries replay; keys are bound to action + resource type + resource ID + body, and cross-resource reuse, changed fingerprints, and stale versions return 409 without partial writes.
- An existing active case requires an exact non-null case version. Mark-reviewed closes an open case atomically as resolved and removes it from the queue.
- Transaction failure rolls back recipe state, case, appeal, notification, image rotation, compatibility marker, replay record, and audit event.
- Owner deletion closes active cases/appeals as `content_removed`.
- Audit rows cannot be updated or deleted.

## Public surfaces

- Execute every row in `public-query-inventory.md` for clear, restricted, reinstated, owner, other-user, and anonymous/session-expired states.
- Restriction makes detail/list/author/search/recommendation/favorite/shopping/ingredient/image results disappear immediately.
- Old public image IDs return 404 after restriction; reinstate exposes only a new ID.
- Recommendation selection/hydration races cannot return content restricted between phases.

## Abuse resistance and accessibility

- Report, claim, reauth, queue, audit, and mutation rate limits are enforced without cross-account bucket collisions.
- Reason enums and details length limits reject unsupported/control-character input.
- English and Simplified Chinese cover report, restriction, appeal, notification, claim, reauth, queue, case, and errors.
- Keyboard-only and screen-reader flows cover every dialog/form/action; status includes text, never color alone.
- Controls meet 44px targets, visible focus, WCAG 2.2 AA contrast, 200% zoom, and 320px reflow.
- Mobile moderation queue renders as cards without horizontal scrolling.
