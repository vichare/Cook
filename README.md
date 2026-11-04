# Cook

Cook is a deployable recipe and private-pantry recommendation MVP. It combines user-authored recipes, per-account pantry state, configurable recommendation sources, and deterministic explanations for what can be cooked now.

Pantry items can carry one optional date-only expiration and a default-off automatic-restock setting. Only an explicit `out` status derives a private shopping-list item; unknown or expired items are not silently changed to out. Manual, recipe, and automatic sources deduplicate by confirmed canonical identity or owner-normalized input and remain isolated per account. Every active source combination uses the same Bought workflow to update pantry state and either remove all shopping sources or keep them for a partial purchase, with optimistic concurrency, idempotent retries, and safe atomic undo.

Ingredient entry uses shared Unicode normalization, multilingual pasted-list splitting, explicit canonical selections, and accessible suggestions; fuzzy matches are advisory and never silently alter recommendation hard filters. Expired items are explained as verification concerns rather than being changed to “used up.”

Recipes may optionally include a 1–5 difficulty and an estimated total kcal value for the whole recipe. Both remain hidden when unset, flow through import drafts and scoped AI management, and do not affect recommendation scoring.

Recipe descriptions and instructions remain Markdown source. The web editor provides text-labeled formatting controls and a live preview; rendering disables raw HTML and embedded images, allows only a small standard/GFM element set, and accepts only HTTP/HTTPS links.

Public recipes use immutable random author handles and optional public nicknames without exposing authentication identities or internal account IDs. Users can privately favorite authors, and recommendations deterministically combine owned recipes, saved recipes, favorite authors' current public recipes, and other public recipes with structured explanations and a bounded 0–999 score.

Recommendation responses are bounded to 50 by default (hard maximum 100) while still scanning every eligible candidate with constant query count and a top-N heap. Browser and AI recipe/image edits share optimistic versions, so a stale tab receives a conflict instead of overwriting newer work.

## Stack

- React 19, TypeScript strict, Vite, TanStack Router and Query
- React Hook Form, Zod, and Radix Dialog
- Rust, Axum, SQLx, SQLite WAL
- OpenAPI, Docker Compose, and host nginx TLS

## Quick start

1. Copy `.env.example` to `.env` and keep `APP_ENV=development`.
2. Start the API with `cargo run -p cook-api`.
3. In another terminal, run `npm install` and `npm run dev`.
4. Open `http://localhost:5173`. Development login is explicitly enabled only when `AUTH_MODE=dev`.

The API listens on `127.0.0.1:8080` by default. Vite proxies `/api` to it.

## Production safety

Production supports Google Authorization Code OIDC with S256 PKCE, Email OTP, and WebAuthn/passkeys through `vichare/rust-web-auth`, pinned to Git revision `b087b47a3ba6fdcc04d67931e97c3214b82e5e6d`. Google-first accounts may explicitly link Email OTP only after verifying the exact normalized Google email. An email-first account is automatically linked to a first-time Google identity only when both providers report the exact same normalized address and its domain is exactly `gmail.com`. Cook does not fold Gmail dots, remove `+tags`, equate `googlemail.com`, merge two existing users, or move passkeys between users. Set `APP_ENV=production`, `AUTH_MODE=google`, the exact public origin, Google credentials, TLS-protected SMTP settings, exact passkey RP/origin values, and four independent 32-byte digest keys. Missing, placeholder, or incomplete authentication configuration fails startup rather than falling back to development adapters.

Development uses the explicit capture Email OTP sender and loopback WebAuthn settings only with `APP_ENV=development`; it does not send mail or make those adapters available in production.

## Repository layout

- `apps/api`: Axum API, SQLite schema, auth port, and domain tests
- `apps/web`: responsive React application and frontend tests
- `openapi/openapi.yaml`: API contract
- `skills/cook-recipe-import`: official AI-client skill and versioned import schema
- `docs`: architecture and operations
- `deploy/nginx`: host reverse-proxy example

See `docs/architecture.md` and `docs/deployment.md` for design and deployment details. Future product scope and unresolved decisions are tracked in `docs/product-roadmap.md`. The manual assistive-technology, zoom, contrast, performance, and typo-benchmark release gates are in `docs/accessibility-release-checklist.md`.

## AI-client recipe tools

Cook's official [recipe skill](skills/cook-recipe-import/SKILL.md) uses browser or transcript tools already available in the user's AI client. The recommended `Import drafts only` preset grants only `recipes:import` and submits untrusted structured payloads as private review drafts. The private-management preset grants `recipes:read`, `recipes:write`, and `recipes:images:write`. A separately confirmed high-risk publish preset also grants `recipes:publish`, allowing immediate public creation, private-to-public transitions, and direct optimistic updates to owned public recipes. No preset grants recipe deletion, pantry/account access, cross-account access, or validation bypass.

Store the one-time token as the `COOK_IMPORT_TOKEN` environment variable, never in a URL, prompt, chat, skill file, reusable command, or log. Set `COOK_BASE_URL` to the exact trusted origin generated by Cook Settings; the GitHub Copilot desktop app may need a restart to inherit newly set environment variables. Cook does not fetch arbitrary source or image URLs. Cover uploads accept local JPEG, PNG, or WebP bytes only after rights confirmation, require alt text, and are safely re-encoded.

GitHub CLI 2.90+ can preview and install the full Skill, including schema assets:

```powershell
gh skill preview vichare/Cook cook-recipe-import@v0.4.0
gh skill install vichare/Cook cook-recipe-import --agent github-copilot --scope user --pin v0.4.0
```

Manual fallback: copy the entire `skills/cook-recipe-import` directory to `~/.copilot/skills/cook-recipe-import` or `~/.agents/skills/cook-recipe-import`; copying only `SKILL.md` omits required schemas. Customize > Skills manages skills after they exist in a supported location; the GitHub source link is for review, not a paste-to-install URL.

Settings presents plain-language task presets, a numbered setup flow, a one-time-token connection test against `GET /api/ai/capabilities`, copyable self-contained draft/create/update examples, multipart cover guidance, authoritative schema links, first-use tasks, and progressive security/bulk rules. Reusable output includes the resolved base URL but never the secret. Recipe-level source URL/title/author/license/attribution are optional and remain distinct from cover provenance; Cook displays supplied license metadata without claiming independent verification.

## Interface languages

Cook ships complete English (`en`) and Simplified Chinese (`zh-CN`) interface catalogs. Before sign-in, Cook uses the saved local preference, then `navigator.languages`, with Chinese browser languages mapped to `zh-CN` and all other languages falling back to English. The sign-in screen also provides a visible English/简体中文 selector that saves the local preference without clearing entered Email OTP values. After sign-in, the account locale returned by `/api/me` is authoritative and can be changed immediately in Settings. The initial browser choice is persisted to an account only while its locale is still unset, so later browser changes do not overwrite an explicit selection.

Release tags matching `vX.Y.Z` publish `ghcr.io/vichare/cook:<tag>` and deploy through a restricted SSH hook. See `docs/release-and-deployment.md` for the public mechanism; server-specific bootstrap and account details are kept in the private operations manual.
