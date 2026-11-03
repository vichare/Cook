# Release and production deployment

Cook follows an immutable tag-to-image deployment flow. A tag matching `vX.Y.Z` runs all release checks, verifies the tagged commit is contained in `main`, publishes `ghcr.io/vichare/cook:<tag>` and `:latest` for `linux/amd64`, then invokes a restricted production SSH hook.

## GitHub configuration

The repository is public so GitHub-hosted Actions can read the pinned public `rust-web-auth` revision. Configure:

| Kind | Name | Value |
|---|---|---|
| Actions secret | `COOK_DEPLOY_SSH_KEY` | Private half of the dedicated deploy key |
| Actions secret | `COOK_DEPLOY_KNOWN_HOSTS` | Pinned host key for the production SSH endpoint |
| Actions variable | `COOK_DEPLOY_HOST` | Production SSH host name or address |
| Actions variable | `COOK_DEPLOY_PORT` | Production SSH port |
| Actions variable | `COOK_DEPLOY_USER` | Restricted deployment account |

Workflows use only the job-scoped `GITHUB_TOKEN` for GHCR. No registry token is stored on the VM: the forced hook writes the token to a temporary Docker config and deletes it after the upgrade.

Confirm the GHCR package is public or grants the repository workflow read access. Protect `main` and restrict release-tag creation to trusted maintainers.

## Production host bootstrap

The host requires Docker Engine/Compose, nginx, `curl`, `sqlite3`, `flock`, and OpenSSH. Copy the files under `deploy/` into a Cook-specific deployment directory, create a mode-`600` production `.env`, install the nginx configuration, and provision TLS.

Use a Cook-specific SSH key whose server-side record is restricted to `deploy-hook.sh`. Disable PTY, agent forwarding, port forwarding, and X11 forwarding for that key. Do not share deployment private keys or allow the Cook key to invoke another application's hook.

Concrete server account names, filesystem paths, key records, bootstrap commands, and co-hosting permissions are intentionally excluded from this public repository. Follow the private operations manual for the production host layout and exact commands.

## Production environment

Set these in the production deployment directory's `.env`:

- `COOK_DOMAIN=cook.vichare.net`
- `COOK_HOST_HTTP_ADDRESS=127.0.0.1:18080`
- Google client ID and secret
- two independent `openssl rand -hex 32` digest keys
- backup retention

Register exactly `https://cook.vichare.net/api/auth/google/callback` in Google. Never place secrets in GitHub variables, workflow logs, images, or the repository.

## Release

Create a tag only on a tested `main` commit:

```sh
git switch main
git pull --ff-only
git tag v1.0.0
git push origin v1.0.0
```

The Release workflow rejects malformed tags and tags whose commit is not an ancestor of `origin/main`. CI and release concurrency groups are Cook-specific and do not block SOP-Node deployments.

## Upgrade and rollback

The forced hook accepts only `deploy vX.Y.Z`. It logs into GHCR with the workflow's short-lived token and runs `upgrade.sh` under a maintenance lock. Upgrade:

1. validates the exact Cook image reference;
2. pulls the immutable tag;
3. creates and integrity-checks an online SQLite backup;
4. starts the new release;
5. waits up to 60 seconds for `http://127.0.0.1:18080/health`.

If startup or health fails, the script restores the previous `.env`, stops the failed container, restores the pre-upgrade database, and starts the previous image. Review the reported deployment log and verify health after any automatic rollback.

To redeploy or roll back manually, run the Deploy workflow with an earlier published tag. Do not retag an existing release.

## Backup and restore

Run `./backup.sh` manually or from a scheduler. It uses SQLite's online backup API, verifies `PRAGMA integrity_check`, and enforces `COOK_BACKUP_KEEP`.

For a manual restore, change to the configured production deployment directory and run:

```sh
flock .maintenance.lock docker compose -f compose.yaml stop cook
./restore.sh backups/cook-YYYYMMDDTHHMMSSZ.db
docker compose -f compose.yaml up -d
curl -fsS http://127.0.0.1:18080/health
```

Store off-host encrypted copies and regularly test restoration. Deployment automation does not configure off-host transport.

## Post-deploy checks

- `curl -fsS https://cook.vichare.net/health`
- complete real Google login and current/all-device logout
- verify private recipe and pantry isolation with two accounts
- verify READY/VERIFY/SHOP/BLOCKED recommendations
- inspect `docker compose -f compose.yaml ps` and bounded container logs
- verify the newest backup and perform a scheduled restore drill
