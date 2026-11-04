# Product roadmap

This document records future product direction separately from the shipped MVP. Confirmed items are commitments; proposed items still require product and implementation decisions. Unknowns remain explicitly unverified.

## Confirmed

### Pantry expiration

Each pantry item has one optional date-only expiration value using ISO `YYYY-MM-DD`. The canonical API and database value can be set, replaced, or cleared without timezone conversion. The UI does not use a date picker: it asks how many local calendar days from today the item expires, offers large shortcuts for none, 0, 1, 3, 7, 14, and 30 days, and shows the calculated absolute date for confirmation. Limited negative values preserve and correct already-expired dates. Expired and expiring-today items remain pantry items rather than being changed to “used up,” but recommendation results require verification and explain the date concern; pantry-only results are blocked until the user confirms a suitable item.

The current scope intentionally models one pantry item with at most one expiration date. Multiple batches of the same ingredient with different quantities or expiration dates are future scope and remain **unverified**.

### Canonical ingredient matching

All ingredient entry points use a shared server-authoritative normalization pipeline: Unicode NFKC, Unicode lowercasing where applicable, full-width and other whitespace normalization, trimming, and whitespace collapse. Pasted lists support commas, full-width commas, Chinese enumeration commas, semicolons, full-width semicolons, and newlines, then remove empty and canonically duplicate values while retaining a readable display name.

Ingredient suggestions are ranked as exact normalized match, owner-confirmed alias, prefix, substring, then bounded typo similarity. Suggestions may use the current account's pantry, owned recipes, visible public recipes, and owner-scoped aliases. A similar spelling or possible synonym is advisory only: Cook never silently changes an ingredient or uses a fuzzy match for recommendation hard filters. The user must explicitly select a canonical candidate, and “keep as entered” always remains available. Cook does not ship an unreviewed large synonym table.

### Shopping list and automatic restock

Each pantry item has an optional, default-off automatic-restock setting. Only an explicit `out` status derives an automatic shopping item; `unknown` and expiration alone do not. Derived items are computed from current pantry state instead of synchronized into a second table, so changing the pantry item to `have` or disabling automatic restock removes the entry immediately. The shopping list represents only ingredients still needed; completed rows are retained solely for migration compatibility and are never rendered.

Manual, recipe-only, automatic, and merged items share one Bought workflow. A single owner-scoped transaction resolves or creates the pantry item, marks it available, refreshes confirmation time, and either consumes every persistent shopping source or keeps all sources for a partial purchase. Keeping a purely derived automatic item materializes the user's explicit keep intent as one persistent source before the pantry changes to `have`, so the item does not disappear. Exact canonical matches and exact unresolved normalized matches may be reused automatically; mismatched identities, aliases, and fuzzy candidates require an explicit reuse-or-create choice. Optimistic versions, bounded idempotency records, and reversible purchase snapshots prevent duplicate writes and stale undo. Recommendation results provide an explicit, confirmed action for adding individual missing ingredients. AI recipe tokens receive no pantry or shopping-list access.

Recipe ingredient lists decorate reliable matches with the signed-in viewer's private pantry status using canonical identity, owner-confirmed aliases, or exact normalized unresolved names only. Recommendation cards use the same bounded hydrated recipe data for a compact cover/description preview, grouped available/verify/missing ingredients, and shopping-list membership; detailed reason codes and ranking remain available under one collapsed explanation. Public recipe DTOs and anonymous views never contain another account's pantry state.

The primary navigation uses a non-underlined selected treatment on wider screens and an accessible menu disclosure on narrow screens. The mobile menu preserves all destinations without horizontal scrolling and restores trigger focus after dismissal.

Quantities and units, multiple pantry batches or expiration dates, barcode entry, bulk checkoff, and persistent purchase history remain future work.

### Optional difficulty and estimated total calories

Recipes, import drafts, AI management payloads, and pending revisions may carry a nullable difficulty integer from 1 to 5 and a nullable estimated total calorie value from 1 to 100,000 kcal. Calories always describe the whole recipe, not a serving, and the interface labels them as an estimate rather than medical or precise nutrition information. Null values render no label, chip, or empty placeholder in recipe lists, details, recommendations, or import review.

These fields do not affect recommendation scores. Recipe-specific scaling by servings or primary-ingredient weight is future scope and remains **unverified**; Cook does not assume calories scale linearly.

### Safe Markdown recipe editing

Recipe descriptions and instructions remain Markdown source strings; Cook never stores rendered HTML. The requested MDXEditor spike confirmed React 19 peer support, but the editor was not adopted: its 659 KB unpacked package brings a broad Lexical, CodeMirror, MDX/JSX, directive, and code-oriented dependency surface that Cook would then need to disable for this constrained recipe editor. That cost and security surface were disproportionate to the required standard-Markdown subset.

Cook instead uses an accessible source editor with text-labeled 44px formatting controls and a live safe preview. Rendering uses `react-markdown` with GFM tables, a strict element allowlist, raw HTML disabled, embedded images excluded, and HTTP/HTTPS-only links rendered with `noopener noreferrer`. The fallback passed React 19 StrictMode tests for Chinese, emoji, nested lists, tables, links, legacy plain text, malformed input, and source/preview synchronization. Its measured production bundle changed from 539.41 KB / 168.74 KB gzip before Markdown support to 708.91 KB / 220.40 KB gzip; route-level code splitting remains a future performance improvement rather than weakening the renderer.

### Older-adult accessibility baseline

Cook targets WCAG 2.2 AA, supports 200% text zoom and reflow, uses at least 16px default body text and 44-by-44 CSS-pixel controls, provides visible keyboard focus and high contrast, and never communicates status only by color. Important forms retain entered values on recoverable errors, use plain-language recovery guidance, and do not impose short automatic timeouts. Destructive credential, token, and import-draft actions require confirmation.

Autocomplete follows the combobox/listbox ARIA pattern with keyboard navigation, Enter selection, Escape dismissal, result-count and selection announcements, explicit labels, and no hover-only behavior.

Automated tests cover representative normalization, delimiter, high-risk non-substitution, date-boundary, and keyboard cases. They do not yet justify a broad Chinese/English Top-3 typo-quality claim or production-catalog latency claim. The assistive-technology matrix, 400%/320px reflow check, measured contrast, production-sized p95 latency, timezone boundary smoke tests, and any future typo benchmark are explicit release gates in `accessibility-release-checklist.md`.

### Public recipe nickname

Google email, display name, issuer, and subject remain private authentication data and are never reused automatically as a public Cook-user identity. A user may opt into a non-unique Unicode nickname, clear it later, or remain in the language-independent anonymous state represented by a null nickname. The UI renders the anonymous label for the current locale.

First-login profile setup defaults to anonymous, offers the Google display name only as a private suggestion that must be selected explicitly, and supports a custom nickname. Skipping completes onboarding anonymously. Public recipe responses expose only the nullable nickname and whether the current viewer owns the recipe; they do not expose another account's internal owner ID or authentication profile. Nickname changes apply immediately to historical public recipes.

### Public Cook users, favorite accounts, and deterministic recommendation ranking

**Shipped on `main`.** Every account has an immutable random 128-bit URL-safe public handle. `/authors/{handle}` exposes only the nullable opt-in nickname and stable keyset-paginated public recipes; it never returns email, Google identity fields, internal user IDs, pantry data, private recipes, account age, or private counts.

Favorite-account relationships are private, reject self-favorites, and dynamically include that Cook user's current and future public recipes. Unpublished recipes disappear immediately. Saved-recipe and favorite-user overlap appears once while preserving both matched source labels and awarding the shared 75-point favorite source contribution only once. Internal `/authors/{handle}` routes and API identifiers remain unchanged for compatibility.

Recommendation source controls separately cover owned recipes, saved recipes, favorite accounts, and other public recipes. Source/visibility, excluded-ingredient, required-ingredient, and ingredient-scope hard filters run before ranking. Responses still populate the documented 0–999 score and score breakdown for compatibility only; `intent-v2` never sorts by them. Production-sized 10,000-public-recipe / 500-favorite-author warm p95 under 250 ms remains an explicit release gate because this development environment does not provide representative production data.

### AI-client recipe import and management

The official Cook skill performs extraction in the user's AI client and submits only a private import draft for review. The Cook server does not fetch arbitrary source URLs, which avoids creating an SSRF-capable fetch service. Web extraction prefers `schema.org/Recipe` JSON-LD and then visible content. Video extraction uses legitimately available captions or transcripts and asks the user for a transcript when tools cannot obtain one. The skill must not bypass login, paywalls, access controls, or DRM.

Imported output is untrusted and must pass server-side URL, length, count, quota, and rate-limit validation. The draft preserves source URL, source title/author, warnings, and confidence without copying source images or long copyrighted passages. A user must review and edit the draft before transactionally accepting it as a private recipe; publication is a later explicit action.

Settings provides revocable, expiring scoped tokens. The recommended preset grants only `recipes:import`; the private-management preset grants only `recipes:read`, `recipes:write`, and `recipes:images:write`. A separately confirmed high-risk preset also grants `recipes:publish`, allowing immediate public creation, owned private-to-public transitions, and direct optimistic updates to owned public recipes. Existing tokens are never upgraded, and no token receives recipe-delete, pantry, account, or cross-account access. The UI recommends a 30-day expiry for publish tokens and adds a second warning when the user chooses no automatic expiry. Secrets are generated with a CSPRNG, displayed once, and stored only as a digest plus non-secret metadata. `GET /api/ai/capabilities` lets any valid token inspect only its prefix, scopes, expiry, and standard preset; business endpoints still enforce their own scopes. The one-time secret belongs in the `COOK_IMPORT_TOKEN` environment variable, never in URLs, prompts, chats, skill files, reusable commands, or logs.

Private-management tokens can list/read owned recipes, idempotently create private recipes, optimistically update owned private recipes, and propose owner-reviewed revisions for public recipes. Publish tokens may additionally create public recipes, publish owned private recipes, and directly update owned public recipes. Both paths return a conflict for stale versions instead of overwriting user edits. Audit metadata records the token ID/prefix, action, recipe ID, and time without authorization secrets or recipe content.

**Shipped on `main`.** Each recipe supports one optional normalized cover image with editable alt text and optional source/license/attribution. Cook uses SQLite BLOB storage for the current single-VM, low-traffic scope so the existing consistent backup includes image bytes atomically and no external storage secret is required. Images are served through random IDs with owner authorization for private recipes and public caching only for public recipes. A future object-storage migration is expected if scale requires it.

The image pipeline accepts only JPEG, PNG, and WebP bytes, rejects SVG/GIF/unsupported formats, enforces request/decoder/pixel/stored-byte limits, applies orientation, resizes to a 1,600px longest edge, flattens transparency, and re-encodes to JPEG to remove metadata. Cook never fetches remote images. The skill must ask the user to confirm image rights and must not automatically copy source-site or video images.

Before and after the one-time reveal, Settings provides persistent non-secret AI client setup with a human-review link to the official public Skill source, verified GitHub CLI repository+skill installation commands pinned to `v0.5.0`, manual full-folder fallback instructions, the exact trusted frontend origin as a copyable `COOK_BASE_URL=<origin>` assignment, and token-free bilingual Prompt output. The real `COOK_IMPORT_TOKEN` secret is copied separately during its one-time reveal and is never interpolated into reusable output or stored in browser persistence. Progressive guidance distinguishes client Secret Stores, secure-prompted current-process environment variables, and persistent user-level configuration with its larger exposure surface. Guided setup includes task cards, a numbered flow, connection testing, copyable request examples that match the bundled JSON schemas, optional/null difficulty and whole-recipe calorie guidance, multipart cover guidance, first-use requests, and progressive disclosure for deterministic bulk/error rules.

**Planned for `v0.5.0`.** Recipe ingredients use one-level requirement groups: groups are combined with AND, alternatives inside a group use OR, and a group may be optional. Each alternative retains its own quantity note and canonical or unresolved identity; alternatives never create aliases and Cook never infers substitutions or converts quantities. Legacy ingredient arrays map deterministically to required single-alternative groups, while old clients cannot flatten an existing OR or optional recipe.

Recommendations use deterministic `intent-v2` ranking inferred from the named controls rather than user-adjustable weights. Ingredient scope, must-use, and avoid are hard filters. If desired ingredients are set, every eligible recipe matching at least one desired identity is in the first partition and every eligible non-match is in the second, so a ready non-match cannot overtake a desired match. Desired identities are OR intent with distinct group assignment: multiple distinct matches rank higher, while one OR group contributes at most one match. Within each partition, owned recipes, saved recipes, and favorite-account recipes share one source tier above other public recipes; matching results then compare desired count/action and other missing ingredients before ordinary fulfillment tie-breakers. Strict canonical/unresolved identity, CJK non-fuzzy behavior, owner isolation, and bounded-query rules remain authoritative.

Shopping supports concrete ingredients and exact-set choice items for OR groups. Choice purchase records which alternative was actually bought before updating pantry. Persisted shopping requirements can be removed transactionally without mutating pantry; automatic-restock-derived requirements remain until automatic restocking is disabled. Homepage statistics are whole-card links to owner recipes and filtered pantry views.

### v0.6.0 content moderation

**Implemented for v0.6.0.** Content moderation is a post-publication moderation overlay rather than preapproval or direct moderator editing. A least-privilege `content_moderator` capability accesses only public-content queues, cases, reports, appeals, restrict/reinstate actions, and a PII-free audit view. It does not expose private recipes, pantry, shopping, authentication identities, sessions, tokens, impersonation, user suspension, deployment/database access, role management, or self-moderation.

Author publication intent remains separate from platform moderation state, and every public surface must use one shared eligibility predicate. Restricted content is excluded or returns 404 to other viewers while its owner may view, edit, and appeal; AI and legacy clients cannot clear a restriction. The first dedicated moderator is bootstrapped with a host-generated one-time 15-minute claim code, an existing Cook user, Passkey enrollment, and a fresh Passkey assertion. There is no hard-coded moderator email or first-user auto-promotion, and emergency revocation remains host-only.

The implementation includes logged-in reporting, aggregated cases and bounded evidence snapshots, owner notifications, one active appeal per restriction epoch, stable reason codes, no reporter identity in moderator DTOs, Passkey step-up for sensitive actions, atomic idempotent optimistic mutations, public image invalidation, an immutable PII-free audit ledger, rate limits, and explicit IDOR/public-query/inventory release gates.

### v0.6.0 recommendation URL state

**Implemented for v0.6.0.** Recommendation filters are shareable, deep-linkable URL state and refresh recommendations automatically. The versioned `rv=1` encoding covers source selection, desired/must/avoid ingredient identities, and `ingredientScope` with deterministic ordering and bounded parsing. Canonical ingredients retain stable IDs plus display text; unresolved text remains explicitly unresolved with length limits. Invalid or unknown parameters fail safely with visible guidance rather than silently binding identities or issuing an unbounded request. Locale and UI-only disclosure state do not belong in the URL.

Initial navigation hydrates controls and performs one request; browser back/forward restores controls and requests the corresponding results without a URL/state loop. Committed controls use a 250 ms debounce, `replaceState`, AbortController cancellation, and a request-generation guard. Uncommitted combobox typing never changes the URL or requests recommendations. Because there is no explicit apply transaction, automatic edits do not create synthetic history entries. Loading and errors preserve prior results, and Retry remains only for failed requests rather than being the primary apply mechanism.

When a shared URL carries a canonical ID that is private to the sender's pantry, recipe, or alias visibility, the receiver's server request still fails closed with a specific list of unavailable submitted IDs. The workbench may then downgrade only those tracked URL-origin IDs to the display names already carried in the link, merge that change into the latest controls, rewrite the URL, explain the downgrade, and retry. Committed edits and Back/Forward invalidate older request generations immediately, before debounce. Returning to the active semantic key inside the debounce restores a cached success or restarts an aborted request, so the page cannot remain stuck loading and no extra history entry is created. If downgrade would merge identities across preferred/required/excluded buckets or collapse multiple canonical IDs to one unresolved name, it fails closed without changing controls or URL. Ordinary UI-selected invalid or unauthorized canonical IDs are never silently downgraded. Stable URL ordering is locale-independent.

The interface warns that shared URLs include ingredient filter words. Tokens, pantry and shopping-list contents, account/private recipe identifiers, email, PII, locale, local-date metadata, and result-card state never enter the URL. The implementation preserves bounded/rate-limited recommendation APIs, deterministic ranking, polite accessibility announcements, mobile and 200% reflow, and automated coverage for deep links, malformed URLs, popstate, debounce, races, failure/retry, both locales, stable identity ordering, and privacy.

Recipe and recommendation cards use the visible recipe title as the detail link rather than repeating a separate open/details action. Recipe-level source provenance appears in a compact Source section after description, ingredients, and method; cover-image provenance remains attached to the cover.

Recipe provenance is optional for original/home recipes and structured independently from cover-image provenance. Browser and AI create/update flows, pending revisions, import-draft acceptance, public/private DTOs, OpenAPI, and JSON schemas preserve source URL, title, author, license, and attribution. Cook treats submitted license text as metadata rather than verified proof; the Skill keeps externally sourced content private when reliable provenance is missing.

Agent Finder did not yield an adopted third-party dedicated recipe skill. Cook therefore ships its own small contract-focused skill and does not add Firecrawl or Apify as dependencies.

### User-configurable interface language

**Shipped on `main`.** Cook provides complete English (`en`) and Simplified Chinese (`zh-CN`) interface catalogs, including navigation, authentication, onboarding, recipes, pantry and expiration states, recommendations, import drafts, token setup, form errors, empty states, ARIA labels, and live announcements.

Before sign-in, locale selection uses a saved local preference first, then `navigator.languages`; Chinese browser languages map to `zh-CN` and all other languages fall back to English. The sign-in screen has a visible English/简体中文 selector; switching saves the preference and retains entered Email OTP values. After sign-in, the account locale is authoritative and persists in SQLite. An existing account with no locale is initialized once from the current local/browser preference and is not repeatedly overwritten. Settings changes update the account and local preference together so the selected language remains active after sign-out.

Anonymous Cook users remain a semantic null nickname rather than a translated database string. Dates and numbers use locale-aware `Intl` formatting, while API enums, recommendation reason codes, error codes, and persisted domain values remain language-independent.

### Authentication availability beyond Google

Google sign-in is unavailable or impractical for many users in mainland China, so Google cannot remain Cook's only sign-in path. Phase 1 now provides provider-neutral email one-time codes (OTP) and WebAuthn/passkeys.

The earlier blanket decision that matching provider email must never cause automatic linking is **superseded** by one narrow, directional exception. A Google-first account may explicitly link Email OTP only after verifying the exact Google email. An email-first account may attach a previously unseen Google identity on first Google login only when the verified addresses match exactly after trim and conservative lowercase normalization and the domain is exactly `gmail.com`. Email-first non-Gmail accounts cannot add Google sign-in. Cook does not fold Gmail dots, remove `+tags`, equate `googlemail.com` with `gmail.com`, merge pre-existing split users, or move passkeys. Existing `(issuer, subject)` ownership remains authoritative unless it conflicts with an Email OTP identity already owned by another user, in which case login fails safely and requires manual recovery.

The email OTP flow returns an opaque `challengeId` from a generic start response and verifies that challenge with the emailed code. Responses and errors do not reveal whether an account exists. The UI uses `autocomplete="one-time-code"`, supports pasting the complete code, shows a resend countdown backed by server-enforced cooldowns, and works across devices.

Microsoft OIDC is the recommended Phase 2 provider based on current research into its global personal-account/Entra endpoints and free application registration. Apple sign-in remains optional because its developer program is paid. Direct WeChat sign-in is deferred because website-application review and eligible-entity requirements are still **unverified**. Authing and SMS login are deferred because they add third-party dependency, personal-data handling, and cost.

These are product and engineering decisions, not legal advice. Any mainland-China hosting, licensing, or regulatory requirements remain **unverified** and require qualified review before deployment decisions rely on them.

## Proposed

### Recipe image expansion

Image galleries, inline step images, and object-storage migration remain future work and are **unverified**. The current confirmed scope is exactly one normalized cover per recipe.
