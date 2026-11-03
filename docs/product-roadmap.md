# Product roadmap

This document records future product direction separately from the shipped MVP. Confirmed items are commitments; proposed items still require product and implementation decisions. Unknowns remain explicitly unverified.

## Confirmed

### Pantry expiration

Each pantry item has one optional date-only expiration value using ISO `YYYY-MM-DD`. The canonical API and database value can be set, replaced, or cleared without timezone conversion. The UI does not use a date picker: it asks how many local calendar days from today the item expires, offers large shortcuts for none, 0, 1, 3, 7, 14, and 30 days, and shows the calculated absolute date for confirmation. Limited negative values preserve and correct already-expired dates. Expired and expiring-today items remain pantry items rather than being changed to “used up,” but recommendation results require verification and explain the date concern; pantry-only results are blocked until the user confirms a suitable item.

The current scope intentionally models one pantry item with at most one expiration date. Multiple batches of the same ingredient with different quantities or expiration dates are future scope and remain **unverified**.

### Canonical ingredient matching

All ingredient entry points use a shared server-authoritative normalization pipeline: Unicode NFKC, Unicode lowercasing where applicable, full-width and other whitespace normalization, trimming, and whitespace collapse. Pasted lists support commas, full-width commas, Chinese enumeration commas, semicolons, full-width semicolons, and newlines, then remove empty and canonically duplicate values while retaining a readable display name.

Ingredient suggestions are ranked as exact normalized match, owner-confirmed alias, prefix, substring, then bounded typo similarity. Suggestions may use the current account's pantry, owned recipes, visible public recipes, and owner-scoped aliases. A similar spelling or possible synonym is advisory only: Cook never silently changes an ingredient or uses a fuzzy match for recommendation hard filters. The user must explicitly select a canonical candidate, and “keep as entered” always remains available. Cook does not ship an unreviewed large synonym table.

### Private shopping list and automatic restock

Each pantry item has an optional, default-off automatic-restock setting. Only an explicit `out` status derives an automatic shopping item; `unknown` and expiration alone do not. Derived items are computed from current pantry state instead of synchronized into a second table, so changing the pantry item to `have` or disabling automatic restock removes the entry immediately. Marking an automatic item purchased transactionally changes the pantry status to `have`, refreshes confirmation time, and removes the derived entry, with a web undo action.

Manual and recipe-added shopping sources persist privately per account. Canonical identities deduplicate by canonical ID, unresolved input deduplicates by the account's normalized key, and overlapping automatic, manual, and recipe sources appear as one item with all provenance shown. Automatic items cannot be deleted or completed like ordinary manual entries; the user must mark them purchased or disable automatic restock. Recommendation results provide an explicit, confirmed action for adding individual missing ingredients. AI recipe tokens receive no pantry or shopping-list access.

### Older-adult accessibility baseline

Cook targets WCAG 2.2 AA, supports 200% text zoom and reflow, uses at least 16px default body text and 44-by-44 CSS-pixel controls, provides visible keyboard focus and high contrast, and never communicates status only by color. Important forms retain entered values on recoverable errors, use plain-language recovery guidance, and do not impose short automatic timeouts. Destructive credential, token, and import-draft actions require confirmation.

Autocomplete follows the combobox/listbox ARIA pattern with keyboard navigation, Enter selection, Escape dismissal, result-count and selection announcements, explicit labels, and no hover-only behavior.

Automated tests cover representative normalization, delimiter, high-risk non-substitution, date-boundary, and keyboard cases. They do not yet justify a broad Chinese/English Top-3 typo-quality claim or production-catalog latency claim. The assistive-technology matrix, 400%/320px reflow check, measured contrast, production-sized p95 latency, timezone boundary smoke tests, and any future typo benchmark are explicit release gates in `accessibility-release-checklist.md`.

### Public recipe nickname

Google email, display name, issuer, and subject remain private authentication data and are never reused automatically as a public author identity. A user may opt into a non-unique Unicode nickname, clear it later, or remain in the language-independent anonymous state represented by a null nickname. The UI renders the anonymous label for the current locale.

First-login profile setup defaults to anonymous, offers the Google display name only as a private suggestion that must be selected explicitly, and supports a custom nickname. Skipping completes onboarding anonymously. Public recipe responses expose only the nullable nickname and whether the current viewer owns the recipe; they do not expose another account's internal owner ID or authentication profile. Nickname changes apply immediately to historical public recipes.

### AI-client recipe import and management

The official Cook skill performs extraction in the user's AI client and submits only a private import draft for review. The Cook server does not fetch arbitrary source URLs, which avoids creating an SSRF-capable fetch service. Web extraction prefers `schema.org/Recipe` JSON-LD and then visible content. Video extraction uses legitimately available captions or transcripts and asks the user for a transcript when tools cannot obtain one. The skill must not bypass login, paywalls, access controls, or DRM.

Imported output is untrusted and must pass server-side URL, length, count, quota, and rate-limit validation. The draft preserves source URL, source title/author, warnings, and confidence without copying source images or long copyrighted passages. A user must review and edit the draft before transactionally accepting it as a private recipe; publication is a later explicit action.

Settings provides revocable, expiring scoped tokens. The recommended preset grants only `recipes:import`; the private-management preset grants only `recipes:read`, `recipes:write`, and `recipes:images:write`. A separately confirmed high-risk preset also grants `recipes:publish`, allowing immediate public creation, owned private-to-public transitions, and direct optimistic updates to owned public recipes. Existing tokens are never upgraded, and no token receives recipe-delete, pantry, account, or cross-account access. The UI recommends a 30-day expiry for publish tokens and adds a second warning when the user chooses no automatic expiry. Secrets are generated with a CSPRNG, displayed once, and stored only as a digest plus non-secret metadata. Every token operation derives the owner and must remain in an AI client's secret store or environment rather than URLs, prompts, or logs.

Private-management tokens can list/read owned recipes, idempotently create private recipes, optimistically update owned private recipes, and propose owner-reviewed revisions for public recipes. Publish tokens may additionally create public recipes, publish owned private recipes, and directly update owned public recipes. Both paths return a conflict for stale versions instead of overwriting user edits. Audit metadata records the token ID/prefix, action, recipe ID, and time without authorization secrets or recipe content.

**Shipped on `main`.** Each recipe supports one optional normalized cover image with editable alt text and optional source/license/attribution. Cook uses SQLite BLOB storage for the current single-VM, low-traffic scope so the existing consistent backup includes image bytes atomically and no external storage secret is required. Images are served through random IDs with owner authorization for private recipes and public caching only for public recipes. A future object-storage migration is expected if scale requires it.

The image pipeline accepts only JPEG, PNG, and WebP bytes, rejects SVG/GIF/unsupported formats, enforces request/decoder/pixel/stored-byte limits, applies orientation, resizes to a 1,600px longest edge, flattens transparency, and re-encodes to JPEG to remove metadata. Cook never fetches remote images. The skill must ask the user to confirm image rights and must not automatically copy source-site or video images.

After the one-time reveal, Settings provides a stable link to the official public skill, copyable token-free installation guidance, and a bilingual prompt template for general AI clients. The skill and prompt refer only to `COOK_IMPORT_TOKEN` and `COOK_BASE_URL`; the real secret is copied separately and is never interpolated into those outputs or stored in browser persistence.

Agent Finder did not yield an adopted third-party dedicated recipe skill. Cook therefore ships its own small contract-focused skill and does not add Firecrawl or Apify as dependencies.

### User-configurable interface language

**Shipped on `main`.** Cook provides complete English (`en`) and Simplified Chinese (`zh-CN`) interface catalogs, including navigation, authentication, onboarding, recipes, pantry and expiration states, recommendations, import drafts, token setup, form errors, empty states, ARIA labels, and live announcements.

Before sign-in, locale selection uses a saved local preference first, then `navigator.languages`; Chinese browser languages map to `zh-CN` and all other languages fall back to English. The sign-in screen has a visible English/简体中文 selector; switching saves the preference and retains entered Email OTP values. After sign-in, the account locale is authoritative and persists in SQLite. An existing account with no locale is initialized once from the current local/browser preference and is not repeatedly overwritten. Settings changes update the account and local preference together so the selected language remains active after sign-out.

Anonymous authors remain a semantic null nickname rather than a translated database string. Dates and numbers use locale-aware `Intl` formatting, while API enums, recommendation reason codes, error codes, and persisted domain values remain language-independent.

### Authentication availability beyond Google

Google sign-in is unavailable or impractical for many users in mainland China, so Google cannot remain Cook's only sign-in path. Phase 1 now provides provider-neutral email one-time codes (OTP) and WebAuthn/passkeys.

The earlier blanket decision that matching provider email must never cause automatic linking is **superseded** by one narrow, directional exception. A Google-first account may explicitly link Email OTP only after verifying the exact Google email. An email-first account may attach a previously unseen Google identity on first Google login only when the verified addresses match exactly after trim and conservative lowercase normalization and the domain is exactly `gmail.com`. Email-first non-Gmail accounts cannot add Google sign-in. Cook does not fold Gmail dots, remove `+tags`, equate `googlemail.com` with `gmail.com`, merge pre-existing split users, or move passkeys. Existing `(issuer, subject)` ownership remains authoritative unless it conflicts with an Email OTP identity already owned by another user, in which case login fails safely and requires manual recovery.

The email OTP flow returns an opaque `challengeId` from a generic start response and verifies that challenge with the emailed code. Responses and errors do not reveal whether an account exists. The UI uses `autocomplete="one-time-code"`, supports pasting the complete code, shows a resend countdown backed by server-enforced cooldowns, and works across devices.

Microsoft OIDC is the recommended Phase 2 provider based on current research into its global personal-account/Entra endpoints and free application registration. Apple sign-in remains optional because its developer program is paid. Direct WeChat sign-in is deferred because website-application review and eligible-entity requirements are still **unverified**. Authing and SMS login are deferred because they add third-party dependency, personal-data handling, and cost.

These are product and engineering decisions, not legal advice. Any mainland-China hosting, licensing, or regulatory requirements remain **unverified** and require qualified review before deployment decisions rely on them.

## Proposed

### Recipe image expansion

Image galleries, inline step images, and object-storage migration remain future work and are **unverified**. The current confirmed scope is exactly one normalized cover per recipe.
