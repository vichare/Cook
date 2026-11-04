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
- Per token, the fixed one-hour buckets allow 20 draft imports, 240 recipe reads, 600 recipe writes, 300 image writes, 60 publish-scope checks, and 120 capability checks. Keep batches comfortably below the relevant ceiling to leave retry headroom.
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

Prefer `schema/import-draft-v2.schema.json`. Use v1 only for a source that has no alternatives or optional ingredients; Cook maps every v1 ingredient to one required single-alternative group.

```http
POST $COOK_BASE_URL/api/import/drafts
Authorization: Bearer $COOK_IMPORT_TOKEN
Content-Type: application/json
```

```json
{
  "schemaVersion": "2.0",
  "sourceUrl": "https://example.com/recipes/tomato-soup",
  "sourceTitle": "Tomato soup",
  "sourceAuthor": "Example author",
  "sourceLicense": "CC-BY-4.0",
  "sourceAttribution": "Example author — Tomato soup",
  "title": "Tomato soup",
  "description": "A concise description.",
  "difficulty": 2,
  "estimatedCalories": 900,
  "ingredientModel": "groups-v1",
  "ingredientGroups": [
    {
      "id": "tomato",
      "optional": false,
      "note": "",
      "sortOrder": 0,
      "alternatives": [
        { "id": "fresh-tomato", "name": "tomatoes", "quantityNote": "800 g", "sortOrder": 0 },
        { "id": "canned-tomato", "name": "canned tomatoes", "quantityNote": "2 cans", "sortOrder": 1 }
      ]
    },
    {
      "id": "garnish",
      "optional": true,
      "note": "for serving",
      "sortOrder": 1,
      "alternatives": [
        { "id": "basil", "name": "basil", "quantityNote": "a few leaves", "sortOrder": 0 }
      ]
    }
  ],
  "steps": [
    "Chop the ingredients.",
    "Simmer until tender, then blend."
  ],
  "warnings": ["Cooking time was inferred from the source."],
  "confidence": 0.9
}
```

`schemaVersion`, `sourceUrl`, `title`, 1–100 groups, and at least one step are required. Groups are AND; 1–8 alternatives inside one group are OR; there may be at most 100 alternatives total. Preserve source alternatives and optionality, but never invent alternatives or mark an ingredient optional without source evidence. IDs must remain stable across retries/updates within that recipe or draft; the same source-local IDs may safely be reused in another resource. Within one group, do not repeat a canonical identity or unresolved normalized name. The source metadata, description, difficulty, whole-recipe estimated calories, warnings, and confidence are optional. Never include owner ID or visibility; Cook derives the owner and creates a private draft. Return the `reviewUrl` resolved against `COOK_BASE_URL` and tell the user that acceptance still requires browser review.

Ingredient identity is whole-term and strict. Never infer that a shorter contained CJK term is equivalent to a longer one (`酱油` is not `油`; `鸡腿` is not `鸡翅`), never manufacture aliases, and never reuse a canonical ID unless Cook returned it for that exact source identity or the user explicitly confirmed the mapping.

## Create or update an owned recipe

Prefer `schema/ai-recipe-v2.schema.json`. The v1 schema remains accepted for single required ingredients only. Never send both `ingredients` and `ingredientGroups`.

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

`title` and at least one ingredient group are required. Preserve the `ingredientModel`, group IDs, alternative IDs, ordering, optional flags, group notes, quantities, canonical IDs, and provenance returned by Cook. A legacy-only update of a recipe or import draft that already has OR or optional groups is rejected with `ingredient_groups_required`. `expectedVersion` is required for update and omitted for create. A stable idempotency key belongs to one source item/revision; do not generate a new key merely because a response was delayed. Without `recipes:publish`, changes to an owned public recipe create a pending owner-reviewed revision. There is no token recipe-delete endpoint.

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
