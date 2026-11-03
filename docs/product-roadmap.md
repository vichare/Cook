# Product roadmap

This document records future product direction separately from the shipped MVP. Confirmed items are commitments; proposed items still require product and implementation decisions. Unknowns remain explicitly unverified.

## Confirmed

### Public recipe nickname

Google email, display name, issuer, and subject remain private authentication data and are never reused automatically as a public author identity. A user may opt into a non-unique Unicode nickname, clear it later, or remain in the language-independent anonymous state represented by a null nickname. The UI renders the anonymous label for the current locale.

First-login profile setup defaults to anonymous, offers the Google display name only as a private suggestion that must be selected explicitly, and supports a custom nickname. Skipping completes onboarding anonymously. Public recipe responses expose only the nullable nickname and whether the current viewer owns the recipe; they do not expose another account's internal owner ID or authentication profile. Nickname changes apply immediately to historical public recipes.

### AI-client recipe import

The official Cook skill performs extraction in the user's AI client and submits only a private import draft for review. The Cook server does not fetch arbitrary source URLs, which avoids creating an SSRF-capable fetch service. Web extraction prefers `schema.org/Recipe` JSON-LD and then visible content. Video extraction uses legitimately available captions or transcripts and asks the user for a transcript when tools cannot obtain one. The skill must not bypass login, paywalls, access controls, or DRM.

Imported output is untrusted and must pass server-side URL, length, count, quota, and rate-limit validation. The draft preserves source URL, source title/author, warnings, and confidence without copying source images or long copyrighted passages. A user must review and edit the draft before transactionally accepting it as a private recipe; publication is a later explicit action.

Settings provides revocable, expiring tokens with the fixed `recipes:import` scope. Secrets are generated with a CSPRNG, displayed once, and stored only as a digest plus non-secret metadata. The token derives the owner, works only on the import-draft endpoint, and must remain in an AI client's secret store or environment rather than URLs, prompts, or logs.

Agent Finder did not yield an adopted third-party dedicated recipe skill. Cook therefore ships its own small contract-focused skill and does not add Firecrawl or Apify as dependencies.

### User-configurable interface language

Cook will provide an account-level interface language setting. The initial supported locales and fallback order are **unverified** and have not been selected yet. Implementation should avoid hard-coding a first locale set before that decision is made.

### Authentication availability beyond Google

Google sign-in is unavailable or impractical for many users in mainland China, so Google cannot remain Cook's only long-term sign-in path. Phase 1 will add provider-neutral email magic links and WebAuthn/passkeys while preserving explicit account linking. Cook must never merge accounts automatically only because provider profiles report the same email address.

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
