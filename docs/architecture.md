# Architecture

## Boundaries

Cook is a same-origin web application: the browser loads React and calls `/api` on the same origin. Host nginx terminates TLS for `cook.vichare.net` and forwards only to `127.0.0.1:18080`. The Compose web container serves static assets and proxies `/api` to the private API container.

SQLite is the single source of truth. WAL mode, foreign keys, a five-second busy timeout, and bounded connection pooling are enabled at startup. Every pantry query is scoped by `owner_id`. Private recipes are visible only to their owner; another account may read or favorite a recipe only when it is public. Update and delete statements always include the authenticated owner.

## Authentication ports

Cook consumes `rust-web-auth-core`, `rust-web-auth-google`, `rust-web-auth-http`, `rust-web-auth-email-otp`, and `rust-web-auth-passkey` at exact revision `b087b47a3ba6fdcc04d67931e97c3214b82e5e6d`. Google Authorization Code OIDC, S256 PKCE, state, nonce, provider verification, secure cookie construction, sliding application sessions, Email OTP security policy, and WebAuthn verification come from the shared library.

Cook implements the consumer ports in SQLite:

- login attempts use insert-if-absent and one-statement `DELETE … RETURNING` consumption;
- existing identities resolve by `(issuer, subject)` first; a narrowly scoped first-login policy may attach a new Google identity to an existing Email OTP user only for an exact normalized `gmail.com` address;
- sessions persist only keyed token digests and use version-based CAS renewal/revocation;
- Google login starts use a 10-attempt, 15-minute SQLite rate-limit window keyed by a digest of the trusted client address;
- Email OTP stores one active challenge per purpose and normalized email, applies the shared ten-minute expiry, one-minute resend cooldown, five-attempt limit, atomic replace/verify/consume behavior, and persists separate email/IP and challenge/IP limiter buckets;
- passkey user handles and credentials are persistent, credential IDs are globally unique, and signature counters, backup state, last-used timestamps, names, and revocation are owner-scoped.

Cook owns its users, mappings, attempts, sessions, rate limits, database, keys, and Host-only production cookies. It does not accept or share SOP-Node cookies or databases. Email OTP identities use the issuer `urn:cook:email-otp`; an unauthenticated verified address creates or resolves only that identity. A Google-first user may link Email OTP only by completing a fresh OTP challenge for the exact normalized email stored on that Google identity. An email-first user may gain Google sign-in automatically only when a previously unseen Google subject reports the exact same normalized `gmail.com` address. The mapper does not apply Gmail dot folding, remove `+tags`, treat `googlemail.com` as `gmail.com`, or link a matching non-Gmail email-first account.

Google identity selection is rechecked inside the same SQLite transaction that creates or attaches the identity and inserts the Cook session. The `(issuer, subject)` primary key handles concurrent callbacks. If the Google subject and matching Email OTP identity already belong to different users, or if a matching non-Gmail email-first identity exists, Cook creates no user or session, records a PII-free audit event, and redirects to a generic recovery message. Existing accounts are never batch-merged or migrated; passkey handles and credentials stay on the canonical Cook user selected by the allowed linking rule.

Passkey registration requires an authenticated account with an explicitly linked Email OTP identity. Login is username-first through that identity. WebAuthn uses exact RP ID/origin validation and requires user verification. Non-serializable five-minute ceremony state remains in process memory; only a random opaque token is placed in a short-lived HttpOnly cookie. Restarting the process safely invalidates in-flight ceremonies without affecting credentials.

Production SMTP uses TLS and is configured only by environment variables. OTP codes and SMTP passwords are never logged. Development/test may use an explicit capture sender, but production cannot construct it. Request tracing records only method and path, never callback query strings, OTP codes, cookies, authorization headers, or SMTP credentials.

Development login is selectable only with `AUTH_MODE=dev` and a loopback bind. Production requires Google mode, exact HTTPS origin and passkey RP values, Google credentials, complete SMTP settings, and four distinct random digest keys for OAuth state, sessions, Email OTP, and passkey ceremonies. Incomplete configuration fails startup.

## Recommendation rules

The recommendation engine is deterministic:

1. Select candidates from enabled sources: favorites, owned recipes, and other users' public recipes.
2. Apply hard filters before scoring: excluded ingredients remove a recipe; every required ingredient must be present in the recipe.
3. Classify each ingredient from the current user's pantry:
   - recently confirmed, unexpired `have`: available;
   - expired or expiring-today `have`: verify with an explicit date reason;
   - stale `have` or `unknown`: verify;
   - absent or `out`: missing.
4. Classify recipes as `READY`, `VERIFY`, `SHOP`, or `BLOCKED`. Pantry-only mode marks recipes with any missing or not-yet-confirmed ingredient as `BLOCKED`.
5. Sort by status, preferred-ingredient matches, confirmed ingredient count, then title.

Responses include available, verification, and missing lists plus human-readable reasons.

Ingredient matching uses canonical identities selected by the user, with normalized-name fallback for existing or unbound entries. The authoritative normalization pipeline performs Unicode NFKC, Unicode lowercasing where applicable, whitespace normalization, trimming, and collapse. It also splits pasted comma, full-width comma, Chinese enumeration comma, semicolon, full-width semicolon, and newline lists. Owner-confirmed aliases may participate in hard matching; bounded fuzzy matching is limited to input suggestions and is never applied automatically.

Suggestion candidates come only from the current account's pantry and recipes, visible public recipe ingredients, and that account's private aliases. Selecting a suggestion can create an owner-scoped alias; retaining the original input creates no alias. Pantry expiration is stored as ISO date-only text, and recommendation requests include the browser's local calendar date so “today” does not depend on server timezone. If an older client omits it, the API uses the server UTC date.

## AI recipe boundary

`LicensedRecipeImporter` accepts only structured recipes with source URL, license, and attribution. The included fixture returns no copied content. HowToCook prose and images are not stored in this repository.

The AI-client path is a separate, deliberately narrow boundary. The official skill uses browser or transcript capabilities already present in the user's client; Cook never fetches submitted source or image URLs. The default high-entropy token has only `recipes:import`. The optional management preset grants only `recipes:read`, `recipes:write`, and `recipes:images:write`. Neither preset grants publish, recipe deletion, pantry, or account access. Every endpoint derives the owner from the token; cross-account recipe IDs return 404. Cook stores only the token's SHA-256 digest and non-secret prefix/metadata, supports expiry and revocation, uses persisted per-scope rate-limit buckets, and records token prefix/action/recipe audit metadata without authorization headers or recipe content.

All imported fields are untrusted and validated before persistence. Drafts are owner-scoped, private, quota-limited, and retain source attribution, extraction warnings, and confidence. Web-session routes allow only that owner to list, edit, discard, or accept a draft. Acceptance creates a private recipe and marks the draft consumed in one SQLite transaction, preventing duplicate acceptance. The owner must use the normal recipe workflow for any later publication.

Management creates private recipes with an `Idempotency-Key` and updates against an explicit recipe version. Private changes may apply directly. A proposed change to a public recipe is stored as a pending full-snapshot revision; only the owner session can accept it, and acceptance atomically checks the live version, applies text/ingredients/image changes, increments the version, and resolves the revision. Tokens cannot mutate live public content.

Each recipe has at most one cover image in `recipe_images`. Cook stores only a normalized JPEG BLOB plus random image ID, dimensions, SHA-256, alt text, source/license/attribution, and version. Uploads are limited to 8 MiB, JPEG/PNG/WebP magic-byte decoding, 20 megapixels, 10,000px decoder dimensions, 128 MiB decoder allocation, a 1,600px normalized longest edge, and 2.5 MB stored bytes. Re-encoding removes EXIF/GPS/ICC and other source metadata. Private image reads require the owner session or an owner token with `recipes:read`; public image reads use immutable caching. Replacing an image or changing a public recipe back to private rotates the random image ID so the old server URL no longer resolves. SQLite BLOB storage keeps image data in the existing atomic backup. A future object-storage migration can preserve random IDs and metadata while moving bytes behind the same authorization layer.

## Public profile boundary

OIDC email, display name, issuer, and subject are private. Public recipe DTOs contain a nullable opt-in nickname and an `isOwner` flag instead of the recipe owner's UUID. Null is the stable anonymous state; localized anonymous labels exist only in the UI. Recipe queries join the current nickname dynamically so a profile change updates prior public recipes without rewriting recipe rows.

## Accessibility baseline

The web UI targets WCAG 2.2 AA with 16px or larger default text, 44px minimum control targets, high contrast, visible focus, keyboard operation, textual status cues, and layouts that reflow at 200% zoom. Ingredient suggestions use an ARIA combobox/listbox with active-option tracking and live announcements. Destructive credential, token, and draft actions require explicit confirmation.
