---
name: cook-recipe-import
description: Import authorized recipes into Cook review drafts, manage owned recipes and covers, and publish only with an explicitly granted publish token.
---

# Cook recipe import and management

Use this skill only for content the user is authorized to access. Prefer the safest task: create a private review draft. Manage owned recipes only with the matching scopes, and publish only when the user explicitly selected the high-risk publish preset.

## Configuration and capability check

- Read the trusted Cook origin from `COOK_BASE_URL`. Cook Settings generates its exact value, such as `https://cook.vichare.net`; never ask the user to replace a base-URL placeholder.
- Read the bearer token from the `COOK_IMPORT_TOKEN` environment variable at request time. Never print, quote, log, download, or write it into a URL, prompt, transcript, skill file, or generated artifact.
- GitHub Copilot does not document an arbitrary custom-secret UI for skills. The user should set both environment variables before launching the app and restart it after changes so the app inherits them.
- At task start, call `GET $COOK_BASE_URL/api/ai/capabilities` once. It returns only the token prefix, scopes, expiry, and optional preset. Check again only before a newly requested capability-sensitive phase when needed, not before every recipe.
- `recipes:import` creates private review drafts. Management uses `recipes:read`, `recipes:write`, and `recipes:images:write`. Direct public creation/update additionally requires `recipes:publish`.
- No token preset grants recipe deletion, pantry, shopping-list, account/profile, favorite-account, another user's recipes, or ordinary browser-session access.

## Source and provenance policy

1. Use browser/transcript capabilities already available in the user's client. Cook never fetches a recipe page or image URL.
2. Prefer `schema.org/Recipe` JSON-LD. Otherwise use only visible content the user can legitimately access. Do not bypass login, paywalls, robots restrictions, DRM, or access controls.
3. Use only legitimately available captions/transcripts for video. Ask the user for a transcript if none is available.
4. Rewrite concise ingredients and steps; do not copy long passages or source images.
5. Preserve `sourceUrl`, `sourceTitle`, `sourceAuthor`, `sourceLicense`, and `sourceAttribution` when available. A stated license is metadata, not verified proof.
6. If an external recipe lacks reliable source/license/attribution, do not directly publish it. Create a private review draft or private recipe and include a warning.
7. Record ambiguity, missing quantities, conflicting instructions, or low-confidence interpretation in `warnings`. Do not invent precision.
8. Use standard Markdown only. Do not emit MDX, JSX, raw HTML, scripts, directives, remote images, or executable content.

## Import a private review draft

Validate against `schema/import-draft-v1.schema.json`, then send:

```http
POST $COOK_BASE_URL/api/import/drafts
Authorization: Bearer $COOK_IMPORT_TOKEN
Content-Type: application/json
```

```json
{
  "schemaVersion": "1.0",
  "sourceUrl": "https://example.com/recipes/tomato-soup",
  "sourceTitle": "Tomato soup",
  "sourceAuthor": "Example author",
  "sourceLicense": "CC-BY-4.0",
  "sourceAttribution": "Example author — Tomato soup",
  "title": "Tomato soup",
  "description": "A concise description.",
  "difficulty": 2,
  "estimatedCalories": 900,
  "ingredients": [
    { "name": "tomatoes", "quantityNote": "800 g" },
    { "name": "onion", "quantityNote": "1" }
  ],
  "steps": [
    "Chop the ingredients.",
    "Simmer until tender, then blend."
  ],
  "warnings": ["Cooking time was inferred from the source."],
  "confidence": 0.9
}
```

`schemaVersion`, `sourceUrl`, `title`, at least one ingredient, and at least one step are required. The source metadata, description, difficulty, whole-recipe estimated calories, warnings, and confidence are optional and may be omitted or set to `null` where the schema permits. If present, `difficulty` is an integer from 1 to 5 and `estimatedCalories` is an integer from 1 to 100000 representing total kcal for the whole recipe, never per serving. Never include owner ID or visibility; Cook derives the owner and creates a private draft. Return the `reviewUrl` resolved against `COOK_BASE_URL` and tell the user that acceptance still requires browser review.

## Create or update an owned recipe

Validate against `schema/ai-recipe-v1.schema.json`.

Create:

```http
POST $COOK_BASE_URL/api/ai/recipes
Authorization: Bearer $COOK_IMPORT_TOKEN
Idempotency-Key: stable-source-item-revision-key
Content-Type: application/json
```

```json
{
  "title": "Tomato soup",
  "description": "A concise description.",
  "instructions": "1. Chop the ingredients.\n2. Simmer, then blend.",
  "visibility": "private",
  "difficulty": 2,
  "estimatedCalories": 900,
  "sourceUrl": "https://example.com/recipes/tomato-soup",
  "sourceTitle": "Tomato soup",
  "sourceAuthor": "Example author",
  "sourceLicense": "CC-BY-4.0",
  "sourceAttribution": "Example author — Tomato soup",
  "ingredients": [
    { "name": "tomatoes", "quantityNote": "800 g" },
    { "name": "onion", "quantityNote": "1" }
  ]
}
```

Update:

```http
PUT $COOK_BASE_URL/api/ai/recipes/{recipeId}
Authorization: Bearer $COOK_IMPORT_TOKEN
Content-Type: application/json
```

```json
{
  "title": "Tomato soup, revised",
  "description": "A concise description.",
  "instructions": "1. Chop the ingredients.\n2. Simmer, then blend.",
  "visibility": "private",
  "difficulty": 2,
  "estimatedCalories": 900,
  "sourceUrl": "https://example.com/recipes/tomato-soup",
  "sourceTitle": "Tomato soup",
  "sourceAuthor": "Example author",
  "sourceLicense": "CC-BY-4.0",
  "sourceAttribution": "Example author — Tomato soup",
  "ingredients": [
    { "name": "tomatoes", "quantityNote": "800 g" },
    { "name": "onion", "quantityNote": "1" }
  ],
  "expectedVersion": 3
}
```

`title` and at least one ingredient are required. `expectedVersion` is required for update and omitted for create. A stable idempotency key belongs to one source item/revision; do not generate a new key merely because a response was delayed. Without `recipes:publish`, changes to an owned public recipe create a pending owner-reviewed revision. There is no token recipe-delete endpoint.

## Cover images

Upload only a local image the user owns or has explicit permission to use. A repository-level license may not cover every image. When uncertain, skip the image and record why. Never remotely fetch an image URL or automatically copy a webpage/video thumbnail.

Cover upload is multipart, not JSON:

```http
POST $COOK_BASE_URL/api/ai/recipes/{recipeId}/cover
Authorization: Bearer $COOK_IMPORT_TOKEN
If-Match: "recipe-v3"
Idempotency-Key: stable-cover-operation-key
Content-Type: multipart/form-data
```

Fields: `image` (local JPEG/PNG/WebP bytes), `altText`, and optional `sourceUrl`, `sourceLicense`, `sourceAttribution`. Cook rejects unsupported, malformed, oversized, and decompression-risk files and normalizes accepted images. Do not retry a rejected image by weakening validation.

## Deterministic bulk and error behavior

- Process one recipe at a time.
- Stop the batch on `401`, `403`, capability mismatch, `429` (report `Retry-After` when provided), or bounded repeated network/`5xx` failures.
- For an isolated item `400`, provenance, or image failure, record it and continue. Never silently skip.
- On `409`, reread. Never auto-overwrite, guess a merge, or use last-write-wins. Mark `needs_review` unless a true idempotent replay is positively identified.
- On `404`, do not infer whether another account owns the ID.
- Final report: `created`, `updated`, `skipped`, `failed`, and `needs_review` counts, plus every source item and its recipe ID/URL or non-secret error.

## First-use requests

- Draft preset: “Use the Cook skill to inspect token capabilities, then import this authorized recipe URL as a private review draft.”
- Manage preset: “Use the Cook skill to inspect token capabilities, then save this authorized recipe as private; preserve provenance and skip any image whose rights are uncertain.”
- Publish preset: “Use the Cook skill to inspect token capabilities, then save and publish this authorized recipe; if reliable source/license/attribution is missing, keep it private and flag it for review.”
