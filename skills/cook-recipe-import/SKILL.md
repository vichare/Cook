# Cook recipe import and management

Use this skill to turn an authorized source into a private Cook review draft, manage a user's private recipes and cover images, or publish only when the user deliberately grants the high-risk publish preset.

## Required configuration

- Read the Cook import token from the client's secret store or the `COOK_IMPORT_TOKEN` environment variable.
- Read the Cook origin from trusted client configuration or `COOK_BASE_URL`.
- Never place the token in a URL, prompt, transcript, log, source document, or generated file.
- Do not copy the token into skill installation instructions or a general-AI prompt. Prompts must reference the secret/environment variable name `COOK_IMPORT_TOKEN` only.
- The recommended token has only `recipes:import`. The optional management preset has `recipes:read`, `recipes:write`, and `recipes:images:write`.
- The high-risk publish preset has those management scopes plus `recipes:publish`. Check the token's returned scope list; never infer or request public visibility without that exact scope.
- No preset grants recipe deletion, pantry, account, another user's recipes, or ordinary Cook session access.

## Extraction rules

1. Use browser capabilities already available in the user's AI client. Cook does not fetch the source URL.
2. For web pages, prefer a `schema.org/Recipe` JSON-LD object. If it is absent or incomplete, extract only from visible content the user can legitimately access.
3. For video, use captions or a transcript obtained through a legitimate client capability. If neither is available, ask the user to provide a transcript.
4. Do not bypass authentication, paywalls, robots restrictions, DRM, or access controls.
5. Do not copy source images or long passages. Convert the recipe into concise structured ingredients and rewritten cooking steps while preserving source attribution.
6. Treat every extracted field as untrusted. Record ambiguity, missing quantities, conflicting instructions, or low-confidence interpretation in `warnings`.

## Create the draft

Validate the payload against `schema/import-draft-v1.schema.json`, then send JSON to:

```text
POST {COOK_BASE_URL}/api/import/drafts
```

Authenticate with the standard HTTP Bearer scheme by reading the credential from `COOK_IMPORT_TOKEN` at request time. Do not echo the credential to the conversation or generated output. Cook Settings also provides a token-free bilingual prompt template for clients that do not support installing this skill directly.

Use `schemaVersion: "1.0"`. Do not include an owner identifier or visibility. Cook derives the owner from the token and creates only a private review draft.

On success, tell the user that the draft still requires review and provide the returned `reviewUrl` resolved against `COOK_BASE_URL`. Never claim the recipe was published or finalized. Only the authenticated user can edit, accept, or discard the draft in Cook; acceptance creates a private recipe, which the user may publish separately.

If Cook returns `401`, ask the user to create or replace the appropriate token preset in Cook Settings and store it in the client's secret store. A `403` means the valid token lacks the endpoint's scope; do not broaden access without the user's explicit choice. For `400`, correct the payload rather than weakening validation. For `429`, stop retrying and report the quota or rate limit.

## Manage owned recipes

Use management endpoints only when the token includes the required scopes. Validate create/update JSON against `schema/ai-recipe-v1.schema.json`.

- `GET {COOK_BASE_URL}/api/ai/recipes` lists only recipes owned by the token principal.
- `GET {COOK_BASE_URL}/api/ai/recipes/{id}` returns an owned recipe and its current `version`.
- `POST {COOK_BASE_URL}/api/ai/recipes` creates a private recipe by default. Send a stable, retry-safe `Idempotency-Key`; never generate a new key merely because a response was delayed. `visibility: "public"` requires `recipes:publish`.
- `PUT {COOK_BASE_URL}/api/ai/recipes/{id}` includes `expectedVersion`. Without `recipes:publish`, a public recipe returns a pending revision that the owner must review in Cook. With `recipes:publish`, an owned private recipe may become public and an owned public recipe updates live.
- There is no token recipe-delete endpoint. Do not attempt to simulate deletion.

On `409`, re-read the recipe and ask the user how to reconcile their newer edits; never use last-write-wins. On `404`, do not infer whether another account owns the ID.

For bulk work, call the per-recipe endpoints one item at a time rather than inventing a batch route. Use one stable idempotency key per creation, continue only according to the user's requested failure policy, and never silently skip an item. Finish with created, updated, skipped, and failed counts plus each recipe ID or URL and any error.

## Cover images

Never ask Cook to fetch an image URL. Before upload, ask the user to confirm that they own the local image or have permission to use it. Do not automatically copy a webpage or video thumbnail. Upload only local JPEG, PNG, or WebP bytes to `/api/ai/recipes/{id}/cover`, provide useful alt text, and include known source URL, license, and attribution. Send `If-Match: "recipe-v<version>"`.

Cook rejects SVG, GIF, malformed, oversized, and decompression-risk inputs; normalizes the longest edge to 1,600px; and stores at most one cover. A private cover changes directly. Without `recipes:publish`, a public cover replacement/removal creates a pending revision for owner review; with both `recipes:images:write` and `recipes:publish`, the owned public cover changes live. Do not retry a rejected image by weakening validation or converting it with an external command.
