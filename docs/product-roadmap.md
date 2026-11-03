# Product roadmap

This document records future product direction separately from the shipped MVP. Confirmed items are commitments; proposed items still require product and implementation decisions. Unknowns remain explicitly unverified.

## Confirmed

### Pantry expiration

Each pantry item has one optional date-only expiration value using ISO `YYYY-MM-DD`. The value can be set, replaced, or cleared without timezone conversion. The UI provides a native date field plus large shortcuts for no date, today, tomorrow, one week, and one month. Expired and expiring-today items remain pantry items rather than being changed to “used up,” but recommendation results require verification and explain the date concern; pantry-only results are blocked until the user confirms a suitable item.

The current scope intentionally models one pantry item with at most one expiration date. Multiple batches of the same ingredient with different quantities or expiration dates are future scope and remain **unverified**.

### Canonical ingredient matching

All ingredient entry points use a shared server-authoritative normalization pipeline: Unicode NFKC, Unicode lowercasing where applicable, full-width and other whitespace normalization, trimming, and whitespace collapse. Pasted lists support commas, full-width commas, Chinese enumeration commas, semicolons, full-width semicolons, and newlines, then remove empty and canonically duplicate values while retaining a readable display name.

Ingredient suggestions are ranked as exact normalized match, owner-confirmed alias, prefix, substring, then bounded typo similarity. Suggestions may use the current account's pantry, owned recipes, visible public recipes, and owner-scoped aliases. A similar spelling or possible synonym is advisory only: Cook never silently changes an ingredient or uses a fuzzy match for recommendation hard filters. The user must explicitly select a canonical candidate, and “keep as entered” always remains available. Cook does not ship an unreviewed large synonym table.

### Older-adult accessibility baseline

Cook targets WCAG 2.2 AA, supports 200% text zoom and reflow, uses at least 16px default body text and 44-by-44 CSS-pixel controls, provides visible keyboard focus and high contrast, and never communicates status only by color. Important forms retain entered values on recoverable errors, use plain-language recovery guidance, and do not impose short automatic timeouts. Destructive credential, token, and import-draft actions require confirmation.

Autocomplete follows the combobox/listbox ARIA pattern with keyboard navigation, Enter selection, Escape dismissal, result-count and selection announcements, explicit labels, and no hover-only behavior.

Automated tests cover representative normalization, delimiter, high-risk non-substitution, date-boundary, and keyboard cases. They do not yet justify a broad Chinese/English Top-3 typo-quality claim or production-catalog latency claim. The assistive-technology matrix, 400%/320px reflow check, measured contrast, production-sized p95 latency, timezone boundary smoke tests, and any future typo benchmark are explicit release gates in `accessibility-release-checklist.md`.

### Public recipe nickname

Google email, display name, issuer, and subject remain private authentication data and are never reused automatically as a public author identity. A user may opt into a non-unique Unicode nickname, clear it later, or remain in the language-independent anonymous state represented by a null nickname. The UI renders the anonymous label for the current locale.

First-login profile setup defaults to anonymous, offers the Google display name only as a private suggestion that must be selected explicitly, and supports a custom nickname. Skipping completes onboarding anonymously. Public recipe responses expose only the nullable nickname and whether the current viewer owns the recipe; they do not expose another account's internal owner ID or authentication profile. Nickname changes apply immediately to historical public recipes.

### AI-client recipe import

The official Cook skill performs extraction in the user's AI client and submits only a private import draft for review. The Cook server does not fetch arbitrary source URLs, which avoids creating an SSRF-capable fetch service. Web extraction prefers `schema.org/Recipe` JSON-LD and then visible content. Video extraction uses legitimately available captions or transcripts and asks the user for a transcript when tools cannot obtain one. The skill must not bypass login, paywalls, access controls, or DRM.

Imported output is untrusted and must pass server-side URL, length, count, quota, and rate-limit validation. The draft preserves source URL, source title/author, warnings, and confidence without copying source images or long copyrighted passages. A user must review and edit the draft before transactionally accepting it as a private recipe; publication is a later explicit action.

Settings provides revocable, expiring tokens with the fixed `recipes:import` scope. Secrets are generated with a CSPRNG, displayed once, and stored only as a digest plus non-secret metadata. The token derives the owner, works only on the import-draft endpoint, and must remain in an AI client's secret store or environment rather than URLs, prompts, or logs.

After the one-time reveal, Settings provides a stable link to the official public skill, copyable token-free installation guidance, and a bilingual prompt template for general AI clients. The skill and prompt refer only to `COOK_IMPORT_TOKEN` and `COOK_BASE_URL`; the real secret is copied separately and is never interpolated into those outputs or stored in browser persistence.

Agent Finder did not yield an adopted third-party dedicated recipe skill. Cook therefore ships its own small contract-focused skill and does not add Firecrawl or Apify as dependencies.

### User-configurable interface language

**Shipped on `main`.** Cook provides complete English (`en`) and Simplified Chinese (`zh-CN`) interface catalogs, including navigation, authentication, onboarding, recipes, pantry and expiration states, recommendations, import drafts, token setup, form errors, empty states, ARIA labels, and live announcements.

Before sign-in, locale selection uses a saved local preference first, then `navigator.languages`; Chinese browser languages map to `zh-CN` and all other languages fall back to English. After sign-in, the account locale is authoritative and persists in SQLite. An existing account with no locale is initialized once from the current local/browser preference and is not repeatedly overwritten. Settings changes update the account and local preference together so the selected language remains active after sign-out.

Anonymous authors remain a semantic null nickname rather than a translated database string. Dates and numbers use locale-aware `Intl` formatting, while API enums, recommendation reason codes, error codes, and persisted domain values remain language-independent.

### Authentication availability beyond Google

Google sign-in is unavailable or impractical for many users in mainland China, so Google cannot remain Cook's only sign-in path. Phase 1 now provides provider-neutral email one-time codes (OTP) and WebAuthn/passkeys while preserving explicit account linking. Cook never merges accounts automatically only because provider profiles report the same email address.

The email OTP flow returns an opaque `challengeId` from a generic start response and verifies that challenge with the emailed code. Responses and errors do not reveal whether an account exists. The UI uses `autocomplete="one-time-code"`, supports pasting the complete code, shows a resend countdown backed by server-enforced cooldowns, and works across devices.

Microsoft OIDC is the recommended Phase 2 provider based on current research into its global personal-account/Entra endpoints and free application registration. Apple sign-in remains optional because its developer program is paid. Direct WeChat sign-in is deferred because website-application review and eligible-entity requirements are still **unverified**. Authing and SMS login are deferred because they add third-party dependency, personal-data handling, and cost.

These are product and engineering decisions, not legal advice. Any mainland-China hosting, licensing, or regulatory requirements remain **unverified** and require qualified review before deployment decisions rely on them.

## Proposed

### Recipe image uploads

Recipe image upload is a post-MVP feature. A reasonable initial scope is one cover image per recipe, without galleries or inline instruction images.

The implementation complexity is moderate when images are stored on a local persistent volume, but it adds backup size, restore time, capacity monitoring, and orphan-file cleanup to operations. Object storage is likely easier to scale later. The final storage backend is **unverified** and remains undecided.

Any image-upload design must include these security and privacy controls:

- accept only JPEG, PNG, and WebP; reject SVG and other active or unsupported formats;
- validate by actually decoding the file server-side, then re-encode it into an approved format;
- remove EXIF and other metadata during re-encoding;
- enforce limits on uploaded bytes, decoded pixel dimensions, and images per recipe;
- generate random, non-guessable storage identifiers instead of trusting original file names;
- require authentication, recipe-owner authorization, storage quotas, and upload rate limits;
- never trust the client-provided file name, extension, or MIME type as validation;
- ensure images attached to private recipes cannot be fetched through a public or predictable URL;
- include image data in backup and restore procedures, and periodically remove orphaned files safely.

Before implementation, the storage choice, serving authorization model, size limits, image transformations, retention rules, and migration path must be documented and tested.
