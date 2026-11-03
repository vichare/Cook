# Cook

Cook is a deployable recipe and private-pantry recommendation MVP. It combines user-authored recipes, per-account pantry state, configurable recommendation sources, and deterministic explanations for what can be cooked now.

Pantry items can carry one optional date-only expiration. Ingredient entry uses shared Unicode normalization, multilingual pasted-list splitting, explicit canonical selections, and accessible suggestions; fuzzy matches are advisory and never silently alter recommendation hard filters. Expired items are explained as verification concerns rather than being changed to “used up.”

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

## AI-client recipe imports

Cook's official [recipe import skill](skills/cook-recipe-import/SKILL.md) uses browser or transcript tools already available in the user's AI client, then submits an untrusted structured payload to Cook as a private review draft. Create a scoped `recipes:import` token in Settings, store it in the client's secret store or `COOK_IMPORT_TOKEN`, and never place it in a URL or prompt. Cook does not fetch arbitrary source URLs, and accepting a reviewed draft creates a private recipe rather than publishing it.

After creating a token, Settings provides the stable public skill path on `main`, token-free installation instructions, and a bilingual prompt template for general AI clients. The `main` URL follows the latest reviewed skill; clients that require immutable instructions should replace `main` with a Cook release tag. These outputs reference `COOK_IMPORT_TOKEN` and `COOK_BASE_URL` only; the one-time token secret is copied separately and is never interpolated into a prompt, URL, download, log, analytics event, or local storage.

## Interface languages

Cook ships complete English (`en`) and Simplified Chinese (`zh-CN`) interface catalogs. Before sign-in, Cook uses the saved local preference, then `navigator.languages`, with Chinese browser languages mapped to `zh-CN` and all other languages falling back to English. The sign-in screen also provides a visible English/简体中文 selector that saves the local preference without clearing entered Email OTP values. After sign-in, the account locale returned by `/api/me` is authoritative and can be changed immediately in Settings. The initial browser choice is persisted to an account only while its locale is still unset, so later browser changes do not overwrite an explicit selection.

Release tags matching `vX.Y.Z` publish `ghcr.io/vichare/cook:<tag>` and deploy through a restricted SSH hook. See `docs/release-and-deployment.md` for the public mechanism; server-specific bootstrap and account details are kept in the private operations manual.
