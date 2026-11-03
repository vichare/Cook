# Deployment

## Prerequisites

- Docker Engine with Compose
- Host nginx and TLS certificates for `cook.vichare.net`
- DNS pointing to the host
- A Google OAuth client with exact callback `https://cook.vichare.net/api/auth/google/callback`
- A TLS-capable SMTP account and verified sender for Email OTP delivery
- A passkey relying party configured with RP ID `cook.vichare.net` and origin `https://cook.vichare.net`

Host capacity, Google callback registration, SMTP deliverability, and production secrets remain **unverified**. The shared auth dependency is pinned to revision `b087b47a3ba6fdcc04d67931e97c3214b82e5e6d`.

## Procedure

1. Copy `.env.example` to `.env` outside version control.
2. Set `APP_ENV=production`, `AUTH_MODE=google`, `PUBLIC_ORIGIN=https://cook.vichare.net`, Google credentials, complete TLS SMTP settings, exact passkey RP/origin/name values, and four independent random 64-hex-character authentication keys.
3. Register the exact Google callback above; do not add wildcards or alternate query parameters.
4. Verify the SMTP sender and delivery path without placing credentials or OTP codes in logs.
5. Run `docker compose build` and `docker compose up -d`.
6. Install `deploy/nginx/cook.vichare.net.conf` in host nginx and validate with `nginx -t`.
7. Confirm that only `127.0.0.1:18080` is published by Compose, then reload nginx.
8. Smoke-test Google, Email OTP, passkey registration/login/revocation, Google-first exact-email OTP linking, Gmail email-first Google convergence, non-Gmail rejection, split-account recovery, current/all-device logout, cross-account recipe/image isolation, scoped AI token denial, recommendation explanations, and restore procedure. Never create production test identities that intentionally collide with a real user.

Production releases are normally installed by the tag-driven GitHub Actions workflow rather than by building on the VM. See `release-and-deployment.md`.

Never put real credentials in Compose, documentation, Git, screenshots, or support logs.

## Operations

`GET /health` checks API process availability. Container logs are emitted to stdout/stderr. Treat repeated `401`, SMTP delivery failures, WebAuthn origin/RP mismatches, database locks, and startup configuration errors as actionable; there is no success-shaped fallback.

All nginx layers set `client_max_body_size 9m`, slightly above Cook's 8 MiB image-file limit to allow multipart metadata and framing. Keep those values aligned when changing upload limits. Cover bytes are normalized and stored inside SQLite, so no external storage secret or orphan-file backup job is required. Image growth does increase database, backup, restore, integrity-check, and off-host storage sizes; monitor those sizes before raising limits.
