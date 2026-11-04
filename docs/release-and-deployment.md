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

The host requires Docker Engine/Compose, nginx, `curl`, `sqlite3`, `flock`, and OpenSSH. Copy the files under `deploy/` into a Cook-specific deployment directory, create a mode-`600` production `.env`, install the nginx configuration, and provision TLS. Before the first release, create the bind-mounted `data` directory with owner UID `10001`, an appropriate `<deploy-group>`, and mode `2770`. This lets the container owner write while the deployment group performs host-side backup and restore, with setgid inheritance for new files. `upgrade.sh` rejects a missing or incorrectly owned directory, or one the deployment user cannot read/write, rather than attempting a privileged ownership change.

Use a Cook-specific SSH key whose server-side record is restricted to `deploy-hook.sh`. Disable PTY, agent forwarding, port forwarding, and X11 forwarding for that key. Do not share deployment private keys or allow the Cook key to invoke another application's hook.

Concrete server account names, filesystem paths, key records, bootstrap commands, and co-hosting permissions are intentionally excluded from this public repository. Follow the private operations manual for the production host layout and exact commands.

## Production environment

Set these in the production deployment directory's `.env`:

- `COOK_DOMAIN=cook.vichare.net`
- `COOK_HOST_HTTP_ADDRESS=127.0.0.1:18080`
- Google client ID and secret
- TLS SMTP host, port, mode, username, password, and verified sender
- exact passkey RP ID, HTTPS origin, and display name
- four independent `openssl rand -hex 32` digest keys
- backup retention

Register exactly `https://cook.vichare.net/api/auth/google/callback` in Google. Keep the SMTP password and every digest key only in the private production environment; never place them in GitHub variables, workflow logs, images, or the repository.

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
5. waits up to 60 seconds for `http://127.0.0.1:18080/health` to return the exact API body `ok`.

The image nginx configuration proxies exact `/health` requests to the API rather than the SPA fallback. The entrypoint supervises both the API and nginx; if either exits, it terminates and reaps the other process so the container exits and its restart policy can apply.

If startup or health fails, the script restores the previous `.env`, stops the failed container, and restores the pre-upgrade database. It restarts the previous image only when `.env` contained a previous `COOK_IMAGE`. On a failed first deployment there is no prior release to start, so the failed container remains stopped and the hook reports that host bootstrap or configuration must be corrected before retrying. Review the reported deployment log and verify health after any automatic rollback.

To redeploy or roll back manually, run the Deploy workflow with an earlier published tag. Do not retag an existing release.

## Backup and restore

Run `./backup.sh` manually or from a scheduler. It uses SQLite's online backup API, verifies `PRAGMA integrity_check`, and enforces `COOK_BACKUP_KEEP`.

For a manual restore, change to the configured production deployment directory and run:

```sh
flock .maintenance.lock docker compose -f compose.yaml stop cook
./restore.sh backups/cook-YYYYMMDDTHHMMSSZ.db
docker compose -f compose.yaml up -d
test "$(curl -fsS http://127.0.0.1:18080/health)" = ok
```

Store off-host encrypted copies and regularly test restoration. Normalized recipe cover images are stored as SQLite BLOBs and are included automatically, increasing backup size and restore time. Deployment automation does not configure off-host transport.

## Post-deploy checks

- `test "$(curl -fsS https://cook.vichare.net/health)" = ok`
- complete real Google and Email OTP login, passkey registration/login/revocation, and current/all-device logout
- verify Google-first exact-email OTP linking, Gmail email-first Google convergence, non-Gmail rejection, and pre-existing split-account recovery without account merging
- verify private recipe and pantry isolation with two accounts
- open, refresh, copy, and reopen a versioned recommendation URL; verify identical filter/source request semantics, Back/Forward restoration, automatic debounce, retained results on failure, Retry, and no pantry/shopping/account/token/private-recipe data in the URL
- move canonical and unresolved recommendation ingredients among desired, must-use, and do-not-use with keyboard and touch; verify one URL replacement/request per committed move or confirmed batch, target-chip focus, localized live announcements, explicit cross-bucket add confirmation, full-bucket and oversized-URL rollback, Back/Forward cancellation of stale dialogs, visual-viewport collision handling and reference-hidden closure at 320px/zoom, and no duplicate identity or intermediate conflicting URL
- share a URL whose canonical ingredient is visible only through the sender's private pantry/recipe/alias; verify the receiver gets an explained name-only downgrade, while a directly selected unauthorized canonical ID remains rejected
- verify private cover denial, public cover delivery, scoped AI token denial, and owner revision acceptance without sending a real token in logs
- verify READY/VERIFY/SHOP ordering and that ingredient-scope hard filters omit ineligible recipes
- inspect `docker compose -f compose.yaml ps` and bounded container logs
- verify the newest backup and perform a scheduled restore drill
