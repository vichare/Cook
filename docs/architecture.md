# Architecture

## Boundaries

Cook is a same-origin web application: the browser loads React and calls `/api` on the same origin. Host nginx terminates TLS for `cook.vichare.net` and forwards only to `127.0.0.1:18080`. The Compose web container serves static assets and proxies `/api` to the private API container.

SQLite is the single source of truth. WAL mode, foreign keys, a five-second busy timeout, and bounded connection pooling are enabled at startup. Every pantry query is scoped by `owner_id`. Private recipes are visible only to their owner; another account may read or favorite a recipe only when it is public. Update and delete statements always include the authenticated owner.

## Authentication port

Cook consumes `rust-web-auth-core`, `rust-web-auth-google`, and `rust-web-auth-http` at exact revision `113d1f607f8e99efba5e888204facd53304d5b7f`. Google Authorization Code OIDC, S256 PKCE, state, nonce, provider verification, secure cookie construction, and sliding application sessions come from the shared library.

Cook implements the consumer ports in SQLite:

- login attempts use insert-if-absent and one-statement `DELETE … RETURNING` consumption;
- identities map only by `(issuer, subject)`, while verified email and display name remain mutable profile data;
- sessions persist only keyed token digests and use version-based CAS renewal/revocation;
- login starts use a 10-attempt, 15-minute SQLite rate-limit window keyed by a digest of the trusted client address.

Cook owns its users, mappings, attempts, sessions, rate limits, database, keys, and `__Host-cook_session`/`__Host-cook_oauth_state` production cookies. It does not accept or share SOP-Node cookies or databases. Expired attempts are cleaned every 15 minutes. Request tracing records only method and path, never callback query strings or sensitive headers.

Development login is selectable only with `AUTH_MODE=dev` and a loopback bind. Production requires Google mode, the exact HTTPS origin, credentials, and independent random digest keys; incomplete configuration fails startup.

## Recommendation rules

The recommendation engine is deterministic:

1. Select candidates from enabled sources: favorites, owned recipes, and other users' public recipes.
2. Apply hard filters before scoring: excluded ingredients remove a recipe; every required ingredient must be present in the recipe.
3. Classify each ingredient from the current user's pantry:
   - recently confirmed `have`: available;
   - stale `have` or `unknown`: verify;
   - absent or `out`: missing.
4. Classify recipes as `READY`, `VERIFY`, `SHOP`, or `BLOCKED`. Pantry-only mode marks recipes with any missing or not-yet-confirmed ingredient as `BLOCKED`.
5. Sort by status, preferred-ingredient matches, confirmed ingredient count, then title.

Responses include available, verification, and missing lists plus human-readable reasons.

## Import boundary

`LicensedRecipeImporter` accepts only structured recipes with source URL, license, and attribution. The included fixture returns no copied content. HowToCook prose and images are not stored in this repository.

The AI-client import path is a separate, deliberately narrow boundary. The official skill uses browser or transcript capabilities already present in the user's client; Cook never fetches the submitted URL. A high-entropy Bearer token with only `recipes:import` scope authenticates `POST /api/import/drafts`, derives the Cook owner, and cannot authenticate ordinary session endpoints. Cook stores only its SHA-256 digest and non-secret prefix/metadata, supports expiry and revocation, and rate-limits each token.

All imported fields are untrusted and validated before persistence. Drafts are owner-scoped, private, quota-limited, and retain source attribution, extraction warnings, and confidence. Web-session routes allow only that owner to list, edit, discard, or accept a draft. Acceptance creates a private recipe and marks the draft consumed in one SQLite transaction, preventing duplicate acceptance. The owner must use the normal recipe workflow for any later publication.

## Public profile boundary

OIDC email, display name, issuer, and subject are private. Public recipe DTOs contain a nullable opt-in nickname and an `isOwner` flag instead of the recipe owner's UUID. Null is the stable anonymous state; localized anonymous labels exist only in the UI. Recipe queries join the current nickname dynamically so a profile change updates prior public recipes without rewriting recipe rows.
