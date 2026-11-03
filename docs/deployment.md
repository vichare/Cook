# Deployment

## Prerequisites

- Docker Engine with Compose
- Host nginx and TLS certificates for `cook.vichare.net`
- DNS pointing to the host
- A Google OAuth client with exact callback `https://cook.vichare.net/api/auth/google/callback`

Host capacity, Google callback registration, and production secrets remain **unverified**. The shared auth dependency is pinned to revision `113d1f607f8e99efba5e888204facd53304d5b7f`.

## Procedure

1. Copy `.env.example` to `.env` outside version control.
2. Set `APP_ENV=production`, `AUTH_MODE=google`, `PUBLIC_ORIGIN=https://cook.vichare.net`, Google credentials, and independent random 64-hex-character `OAUTH_STATE_DIGEST_KEY` and `APP_SESSION_DIGEST_KEY` values.
3. Register the exact Google callback above; do not add wildcards or alternate query parameters.
4. Run `docker compose build` and `docker compose up -d`.
5. Install `deploy/nginx/cook.vichare.net.conf` in host nginx and validate with `nginx -t`.
6. Confirm that only `127.0.0.1:18080` is published by Compose, then reload nginx.
7. Smoke-test Google login, current/all-device logout, cross-account recipe isolation, recommendation explanations, and restore procedure.

Production releases are normally installed by the tag-driven GitHub Actions workflow rather than by building on the VM. See `release-and-deployment.md`.

Never put real credentials in Compose, documentation, Git, screenshots, or support logs.

## Operations

`GET /health` checks API process availability. Container logs are emitted to stdout/stderr. Treat repeated `401`, database lock, and startup configuration errors as actionable; there is no success-shaped fallback.
