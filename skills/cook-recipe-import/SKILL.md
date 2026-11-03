# Cook recipe import

Use this skill to turn a recipe source that the user is authorized to access into a private Cook import draft for human review.

## Required configuration

- Read the Cook import token from the client's secret store or the `COOK_IMPORT_TOKEN` environment variable.
- Read the Cook origin from trusted client configuration or `COOK_BASE_URL`.
- Never place the token in a URL, prompt, transcript, log, source document, or generated file.
- The token is scoped only to `recipes:import`; do not attempt to use it with Cook session endpoints.

## Extraction rules

1. Use browser capabilities already available in the user's AI client. Cook does not fetch the source URL.
2. For web pages, prefer a `schema.org/Recipe` JSON-LD object. If it is absent or incomplete, extract only from visible content the user can legitimately access.
3. For video, use captions or a transcript obtained through a legitimate client capability. If neither is available, ask the user to provide a transcript.
4. Do not bypass authentication, paywalls, robots restrictions, DRM, or access controls.
5. Do not copy source images or long passages. Convert the recipe into concise structured ingredients and rewritten cooking steps while preserving source attribution.
6. Treat every extracted field as untrusted. Record ambiguity, missing quantities, conflicting instructions, or low-confidence interpretation in `warnings`.

## Create the draft

Validate the payload against `schema/import-draft-v1.schema.json`, then send:

```http
POST {COOK_BASE_URL}/api/import/drafts
Authorization: Bearer {COOK_IMPORT_TOKEN}
Content-Type: application/json
```

Use `schemaVersion: "1.0"`. Do not include an owner identifier or visibility. Cook derives the owner from the token and creates only a private review draft.

On success, tell the user that the draft still requires review and provide the returned `reviewUrl` resolved against `COOK_BASE_URL`. Never claim the recipe was published or finalized. Only the authenticated user can edit, accept, or discard the draft in Cook; acceptance creates a private recipe, which the user may publish separately.

If Cook returns `401`, ask the user to create or replace a `recipes:import` token in Cook Settings and store it in the client's secret store. For `400`, correct the payload rather than weakening validation. For `429`, stop retrying and report the quota or rate limit.
