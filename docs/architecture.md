# Architecture

## Boundaries

Cook is a same-origin web application: the browser loads React and calls `/api` on the same origin. Host nginx terminates TLS for `cook.vichare.net` and forwards only to `127.0.0.1:18080`. The Compose web container serves static assets and proxies `/api` to the private API container.

SQLite is the single source of truth. WAL mode, foreign keys, a five-second busy timeout, and bounded connection pooling are enabled at startup. Every pantry query is scoped by `owner_id`. Private recipes are visible only to their owner; another account may read or favorite a recipe only when it is public. Update and delete statements always include the authenticated owner.

## Authentication ports

Cook consumes `rust-web-auth-core`, `rust-web-auth-google`, `rust-web-auth-http`, `rust-web-auth-email-otp`, and `rust-web-auth-passkey` at exact revision `b087b47a3ba6fdcc04d67931e97c3214b82e5e6d`. Google Authorization Code OIDC, S256 PKCE, state, nonce, provider verification, secure cookie construction, sliding application sessions, Email OTP security policy, and WebAuthn verification come from the shared library.

Cook implements the consumer ports in SQLite:

- login attempts use insert-if-absent and one-statement `DELETE … RETURNING` consumption;
- identities map only by `(issuer, subject)`, while verified email and display name remain mutable profile data;
- sessions persist only keyed token digests and use version-based CAS renewal/revocation;
- Google login starts use a 10-attempt, 15-minute SQLite rate-limit window keyed by a digest of the trusted client address;
- Email OTP stores one active challenge per purpose and normalized email, applies the shared ten-minute expiry, one-minute resend cooldown, five-attempt limit, atomic replace/verify/consume behavior, and persists separate email/IP and challenge/IP limiter buckets;
- passkey user handles and credentials are persistent, credential IDs are globally unique, and signature counters, backup state, last-used timestamps, names, and revocation are owner-scoped.

Cook owns its users, mappings, attempts, sessions, rate limits, database, keys, and Host-only production cookies. It does not accept or share SOP-Node cookies or databases. Email OTP identities use the issuer `urn:cook:email-otp`; an unauthenticated verified address creates or resolves only that identity. A signed-in user may link an address only by completing a fresh OTP challenge bound to that Cook user. Matching a Google profile email never causes automatic linking or account merging.

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

## Import boundary

`LicensedRecipeImporter` accepts only structured recipes with source URL, license, and attribution. The included fixture returns no copied content. HowToCook prose and images are not stored in this repository.

The AI-client import path is a separate, deliberately narrow boundary. The official skill uses browser or transcript capabilities already present in the user's client; Cook never fetches the submitted URL. A high-entropy Bearer token with only `recipes:import` scope authenticates `POST /api/import/drafts`, derives the Cook owner, and cannot authenticate ordinary session endpoints. Cook stores only its SHA-256 digest and non-secret prefix/metadata, supports expiry and revocation, and rate-limits each token.

All imported fields are untrusted and validated before persistence. Drafts are owner-scoped, private, quota-limited, and retain source attribution, extraction warnings, and confidence. Web-session routes allow only that owner to list, edit, discard, or accept a draft. Acceptance creates a private recipe and marks the draft consumed in one SQLite transaction, preventing duplicate acceptance. The owner must use the normal recipe workflow for any later publication.

## Public profile boundary

OIDC email, display name, issuer, and subject are private. Public recipe DTOs contain a nullable opt-in nickname and an `isOwner` flag instead of the recipe owner's UUID. Null is the stable anonymous state; localized anonymous labels exist only in the UI. Recipe queries join the current nickname dynamically so a profile change updates prior public recipes without rewriting recipe rows.

## Accessibility baseline

The web UI targets WCAG 2.2 AA with 16px or larger default text, 44px minimum control targets, high contrast, visible focus, keyboard operation, textual status cues, and layouts that reflow at 200% zoom. Ingredient suggestions use an ARIA combobox/listbox with active-option tracking and live announcements. Destructive credential, token, and draft actions require explicit confirmation.
