# Cook

Cook is a deployable recipe and private-pantry recommendation MVP. It combines user-authored recipes, per-account pantry state, configurable recommendation sources, and deterministic explanations for what can be cooked now.

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

Production uses Google Authorization Code OIDC with S256 PKCE through `vichare/rust-web-auth`, pinned to Git revision `113d1f607f8e99efba5e888204facd53304d5b7f`. Set `APP_ENV=production`, `AUTH_MODE=google`, `PUBLIC_ORIGIN=https://cook.vichare.net`, Google credentials, and two independent 32-byte digest keys. Missing or placeholder authentication configuration fails startup rather than falling back to development authentication.

## Repository layout

- `apps/api`: Axum API, SQLite schema, auth port, and domain tests
- `apps/web`: responsive React application and frontend tests
- `openapi/openapi.yaml`: API contract
- `docs`: architecture and operations
- `deploy/nginx`: host reverse-proxy example

See `docs/architecture.md` and `docs/deployment.md` for design and deployment details.

Release tags matching `vX.Y.Z` publish `ghcr.io/vichare/cook:<tag>` and deploy through a restricted SSH hook. See `docs/release-and-deployment.md` for the public mechanism; server-specific bootstrap and account details are kept in the private operations manual.
