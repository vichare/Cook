use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG},
        HeaderMap, HeaderValue, Response, StatusCode,
    },
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    images::{delete_image, parse_image_multipart, upsert_image, ProcessedImage},
    imports::{
        authenticate_token, record_token_audit, TokenPrincipal, SCOPE_IMAGES_WRITE,
        SCOPE_RECIPES_PUBLISH, SCOPE_RECIPES_READ, SCOPE_RECIPES_WRITE,
    },
    models::{IngredientInput, Recipe, RecipeInput},
    routes::{
        expand_ingredient_inputs, load_visible_recipes, replace_ingredients,
        validate_ingredient_selections, validate_recipe, CurrentUser,
    },
    AppState,
};

const IDEMPOTENCY_HEADER: &str = "idempotency-key";
const VERSION_HEADER: &str = "if-match";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiRecipeInput {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    instructions: String,
    #[serde(default)]
    difficulty: Option<i64>,
    #[serde(default)]
    estimated_calories: Option<i64>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    source_title: Option<String>,
    #[serde(default)]
    source_author: Option<String>,
    #[serde(default)]
    source_license: Option<String>,
    #[serde(default)]
    source_attribution: Option<String>,
    ingredients: Vec<IngredientInput>,
    expected_version: Option<i64>,
    visibility: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeRevision {
    id: String,
    recipe_id: String,
    expected_version: i64,
    title: String,
    description: String,
    instructions: String,
    difficulty: Option<i64>,
    estimated_calories: Option<i64>,
    source_url: Option<String>,
    source_title: Option<String>,
    source_author: Option<String>,
    source_license: Option<String>,
    source_attribution: Option<String>,
    ingredients: Vec<IngredientInput>,
    image_action: String,
    image_alt_text: Option<String>,
    image_url: Option<String>,
    image_source_url: Option<String>,
    image_source_license: Option<String>,
    image_source_attribution: Option<String>,
    status: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum AiRecipeMutation {
    Updated { recipe: Recipe },
    PendingReview { revision: RecipeRevision },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionQuery {
    recipe_id: Option<String>,
}

pub async fn list_ai_recipes(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Vec<Recipe>>> {
    let principal = authenticate_token(&state.pool, &headers, SCOPE_RECIPES_READ).await?;
    let recipes = load_visible_recipes(&state.pool, &principal.owner_id, None)
        .await?
        .into_iter()
        .filter(|recipe| recipe.is_owner)
        .collect();
    audit(&state.pool, &principal, "recipe.list", None).await?;
    Ok(Json(recipes))
}

pub async fn get_ai_recipe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(recipe_id): Path<String>,
) -> AppResult<Json<Recipe>> {
    let principal = authenticate_token(&state.pool, &headers, SCOPE_RECIPES_READ).await?;
    let recipe = load_owned_recipe(&state.pool, &principal.owner_id, &recipe_id).await?;
    audit(&state.pool, &principal, "recipe.read", Some(&recipe_id)).await?;
    Ok(Json(recipe))
}

pub async fn create_ai_recipe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut input): Json<AiRecipeInput>,
) -> AppResult<(StatusCode, Json<Recipe>)> {
    let principal = authenticate_token(&state.pool, &headers, SCOPE_RECIPES_WRITE).await?;
    let request_id = idempotency_key(&headers)?;
    input.ingredients = expand_ingredient_inputs(input.ingredients);
    let visibility = validate_ai_visibility(input.visibility.as_deref().unwrap_or("private"))?;
    if visibility == "public" && !principal_can_publish(&principal) {
        return Err(AppError::Forbidden);
    }
    let recipe_input = recipe_input(&input, visibility);
    validate_recipe(&recipe_input)?;
    let now = Utc::now().timestamp();
    let mut transaction = state.pool.begin().await?;
    if let Some(existing) = sqlx::query_scalar::<_, Option<String>>(
        "SELECT recipe_id FROM ai_recipe_requests
         WHERE token_id = ? AND action = 'recipe.create' AND client_request_id = ?",
    )
    .bind(&principal.token_id)
    .bind(&request_id)
    .fetch_optional(&mut *transaction)
    .await?
    {
        let existing = existing.ok_or_else(|| {
            AppError::Conflict("the recipe created by this idempotency key was deleted".into())
        })?;
        record_token_audit(
            &mut transaction,
            &principal,
            "recipe.create.replay",
            Some(&existing),
            now,
        )
        .await?;
        transaction.commit().await?;
        return Ok((
            StatusCode::OK,
            Json(load_owned_recipe(&state.pool, &principal.owner_id, &existing).await?),
        ));
    }
    let recipe_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO recipes
         (id, owner_id, title, description, instructions, visibility, difficulty,
          estimated_calories, source_url, source_title, source_author, source_license,
          source_attribution, version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
    )
    .bind(&recipe_id)
    .bind(&principal.owner_id)
    .bind(recipe_input.title.trim())
    .bind(recipe_input.description.trim())
    .bind(recipe_input.instructions.trim())
    .bind(&recipe_input.visibility)
    .bind(recipe_input.difficulty)
    .bind(recipe_input.estimated_calories)
    .bind(crate::routes::optional_text(&recipe_input.source_url))
    .bind(crate::routes::optional_text(&recipe_input.source_title))
    .bind(crate::routes::optional_text(&recipe_input.source_author))
    .bind(crate::routes::optional_text(&recipe_input.source_license))
    .bind(crate::routes::optional_text(
        &recipe_input.source_attribution,
    ))
    .execute(&mut *transaction)
    .await?;
    crate::routes::insert_ingredients(
        &mut transaction,
        &recipe_id,
        &principal.owner_id,
        &recipe_input.ingredients,
    )
    .await?;
    sqlx::query(
        "INSERT INTO ai_recipe_requests
         (token_id, action, client_request_id, recipe_id, created_at)
         VALUES (?, 'recipe.create', ?, ?, ?)",
    )
    .bind(&principal.token_id)
    .bind(&request_id)
    .bind(&recipe_id)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    record_token_audit(
        &mut transaction,
        &principal,
        if recipe_input.visibility == "public" {
            "recipe.publish.create"
        } else {
            "recipe.create"
        },
        Some(&recipe_id),
        now,
    )
    .await?;
    transaction.commit().await?;
    Ok((
        StatusCode::CREATED,
        Json(load_owned_recipe(&state.pool, &principal.owner_id, &recipe_id).await?),
    ))
}

pub async fn update_ai_recipe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(recipe_id): Path<String>,
    Json(mut input): Json<AiRecipeInput>,
) -> AppResult<Json<AiRecipeMutation>> {
    let principal = authenticate_token(&state.pool, &headers, SCOPE_RECIPES_WRITE).await?;
    input.ingredients = expand_ingredient_inputs(input.ingredients);
    let expected_version = input
        .expected_version
        .ok_or_else(|| AppError::Validation("expectedVersion is required".into()))?;
    let mut transaction = state.pool.begin().await?;
    let current_visibility = owned_recipe_visibility(
        &mut transaction,
        &principal.owner_id,
        &recipe_id,
        expected_version,
    )
    .await?;
    let target_visibility = validate_ai_visibility(
        input
            .visibility
            .as_deref()
            .unwrap_or(current_visibility.as_str()),
    )?;
    if current_visibility == "public" && target_visibility == "private" {
        return Err(AppError::Validation(
            "AI tokens cannot make a public recipe private".into(),
        ));
    }
    let recipe_input = recipe_input(&input, target_visibility);
    validate_recipe(&recipe_input)?;
    let can_publish = principal_can_publish(&principal);
    if current_visibility == "private" {
        if target_visibility == "public" && !can_publish {
            return Err(AppError::Forbidden);
        }
        update_recipe_direct(
            &mut transaction,
            &principal,
            &recipe_id,
            expected_version,
            &current_visibility,
            &recipe_input,
        )
        .await?;
        transaction.commit().await?;
        return Ok(Json(AiRecipeMutation::Updated {
            recipe: load_owned_recipe(&state.pool, &principal.owner_id, &recipe_id).await?,
        }));
    }
    if can_publish {
        update_recipe_direct(
            &mut transaction,
            &principal,
            &recipe_id,
            expected_version,
            &current_visibility,
            &recipe_input,
        )
        .await?;
        transaction.commit().await?;
        return Ok(Json(AiRecipeMutation::Updated {
            recipe: load_owned_recipe(&state.pool, &principal.owner_id, &recipe_id).await?,
        }));
    }
    let revision_id = create_revision(
        &mut transaction,
        &principal,
        &recipe_id,
        expected_version,
        &recipe_input,
        "keep",
        None,
    )
    .await?;
    transaction.commit().await?;
    Ok(Json(AiRecipeMutation::PendingReview {
        revision: load_revision(&state.pool, &principal.owner_id, &revision_id).await?,
    }))
}

pub async fn upload_ai_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(recipe_id): Path<String>,
    multipart: Multipart,
) -> AppResult<Json<AiRecipeMutation>> {
    let principal = authenticate_token(&state.pool, &headers, SCOPE_IMAGES_WRITE).await?;
    let expected_version = expected_version_header(&headers)?;
    let image = parse_image_multipart(multipart).await?;
    mutate_ai_image(
        &state,
        principal,
        recipe_id,
        expected_version,
        "replace",
        Some(image),
    )
    .await
    .map(Json)
}

pub async fn delete_ai_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(recipe_id): Path<String>,
) -> AppResult<Json<AiRecipeMutation>> {
    let principal = authenticate_token(&state.pool, &headers, SCOPE_IMAGES_WRITE).await?;
    let expected_version = expected_version_header(&headers)?;
    mutate_ai_image(
        &state,
        principal,
        recipe_id,
        expected_version,
        "delete",
        None,
    )
    .await
    .map(Json)
}

async fn mutate_ai_image(
    state: &AppState,
    principal: TokenPrincipal,
    recipe_id: String,
    expected_version: i64,
    image_action: &str,
    image: Option<ProcessedImage>,
) -> AppResult<AiRecipeMutation> {
    let mut transaction = state.pool.begin().await?;
    let visibility = owned_recipe_visibility(
        &mut transaction,
        &principal.owner_id,
        &recipe_id,
        expected_version,
    )
    .await?;
    let direct_public_update = visibility == "public" && principal_can_publish(&principal);
    if visibility == "private" || direct_public_update {
        match image_action {
            "replace" => {
                upsert_image(
                    &mut transaction,
                    &recipe_id,
                    image
                        .as_ref()
                        .ok_or_else(|| AppError::Validation("image is required".into()))?,
                )
                .await?;
            }
            "delete" => delete_image(&mut transaction, &recipe_id).await?,
            _ => return Err(AppError::Validation("invalid image action".into())),
        }
        let updated = sqlx::query(
            "UPDATE recipes SET version = version + 1, updated_at = CURRENT_TIMESTAMP
             WHERE id = ? AND owner_id = ? AND visibility = ? AND version = ?",
        )
        .bind(&recipe_id)
        .bind(&principal.owner_id)
        .bind(&visibility)
        .bind(expected_version)
        .execute(&mut *transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(AppError::Conflict(
                "recipe changed; reload and retry".into(),
            ));
        }
        record_token_audit(
            &mut transaction,
            &principal,
            if direct_public_update && image_action == "replace" {
                "recipe.image.publish.replace"
            } else if direct_public_update {
                "recipe.image.publish.delete"
            } else if image_action == "replace" {
                "recipe.image.replace"
            } else {
                "recipe.image.delete"
            },
            Some(&recipe_id),
            Utc::now().timestamp(),
        )
        .await?;
        transaction.commit().await?;
        return Ok(AiRecipeMutation::Updated {
            recipe: load_owned_recipe(&state.pool, &principal.owner_id, &recipe_id).await?,
        });
    }
    let current = load_recipe_input_tx(&mut transaction, &recipe_id).await?;
    let revision_id = create_revision(
        &mut transaction,
        &principal,
        &recipe_id,
        expected_version,
        &current,
        image_action,
        image.as_ref(),
    )
    .await?;
    transaction.commit().await?;
    Ok(AiRecipeMutation::PendingReview {
        revision: load_revision(&state.pool, &principal.owner_id, &revision_id).await?,
    })
}

pub(crate) async fn list_revisions(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Query(query): Query<RevisionQuery>,
) -> AppResult<Json<Vec<RecipeRevision>>> {
    let ids: Vec<String> = if let Some(recipe_id) = query.recipe_id {
        sqlx::query_scalar(
            "SELECT id FROM recipe_revisions
             WHERE owner_id = ? AND recipe_id = ? AND status = 'pending'
             ORDER BY updated_at DESC",
        )
        .bind(&user.id)
        .bind(recipe_id)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT id FROM recipe_revisions
             WHERE owner_id = ? AND status = 'pending' ORDER BY updated_at DESC",
        )
        .bind(&user.id)
        .fetch_all(&state.pool)
        .await?
    };
    let mut revisions = Vec::with_capacity(ids.len());
    for id in ids {
        revisions.push(load_revision(&state.pool, &user.id, &id).await?);
    }
    Ok(Json(revisions))
}

pub(crate) async fn accept_revision(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(revision_id): Path<String>,
) -> AppResult<Json<Recipe>> {
    let mut transaction = state.pool.begin().await?;
    let row = sqlx::query(
        "SELECT recipe_id, expected_version, title, description, instructions, difficulty,
                estimated_calories, source_url, source_title, source_author, source_license,
                source_attribution, image_action,
                image_media_type, image_bytes, image_width, image_height, image_sha256,
                image_alt_text, image_source_url, image_source_license, image_source_attribution
         FROM recipe_revisions WHERE id = ? AND owner_id = ? AND status = 'pending'",
    )
    .bind(&revision_id)
    .bind(&user.id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or(AppError::NotFound)?;
    let recipe_id: String = row.get("recipe_id");
    let expected_version: i64 = row.get("expected_version");
    let updated = sqlx::query(
        "UPDATE recipes SET title = ?, description = ?, instructions = ?,
           difficulty = ?, estimated_calories = ?, source_url = ?, source_title = ?,
           source_author = ?, source_license = ?, source_attribution = ?,
           version = version + 1,
           updated_at = CURRENT_TIMESTAMP
         WHERE id = ? AND owner_id = ? AND version = ?",
    )
    .bind(row.get::<String, _>("title"))
    .bind(row.get::<String, _>("description"))
    .bind(row.get::<String, _>("instructions"))
    .bind(row.get::<Option<i64>, _>("difficulty"))
    .bind(row.get::<Option<i64>, _>("estimated_calories"))
    .bind(row.get::<Option<String>, _>("source_url"))
    .bind(row.get::<Option<String>, _>("source_title"))
    .bind(row.get::<Option<String>, _>("source_author"))
    .bind(row.get::<Option<String>, _>("source_license"))
    .bind(row.get::<Option<String>, _>("source_attribution"))
    .bind(&recipe_id)
    .bind(&user.id)
    .bind(expected_version)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "live recipe changed after this revision was created".into(),
        ));
    }
    let ingredients = load_revision_ingredients_tx(&mut transaction, &revision_id).await?;
    replace_ingredients(&mut transaction, &recipe_id, &user.id, &ingredients).await?;
    match row.get::<String, _>("image_action").as_str() {
        "keep" => {}
        "delete" => delete_image(&mut transaction, &recipe_id).await?,
        "replace" => {
            let bytes: Vec<u8> = row
                .get::<Option<Vec<u8>>, _>("image_bytes")
                .ok_or_else(|| AppError::Data("revision image is missing".into()))?;
            let sha: Vec<u8> = row
                .get::<Option<Vec<u8>>, _>("image_sha256")
                .ok_or_else(|| AppError::Data("revision image digest is missing".into()))?;
            let image =
                ProcessedImage {
                    bytes,
                    width: u32::try_from(row.get::<i64, _>("image_width"))
                        .map_err(|_| AppError::Data("invalid revision image width".into()))?,
                    height: u32::try_from(row.get::<i64, _>("image_height"))
                        .map_err(|_| AppError::Data("invalid revision image height".into()))?,
                    sha256: sha
                        .try_into()
                        .map_err(|_| AppError::Data("invalid revision image digest".into()))?,
                    metadata: crate::images::ImageMetadataInput {
                        alt_text: row.get::<Option<String>, _>("image_alt_text").ok_or_else(
                            || AppError::Data("revision image alt text is missing".into()),
                        )?,
                        source_url: row.get("image_source_url"),
                        source_license: row.get("image_source_license"),
                        source_attribution: row.get("image_source_attribution"),
                    },
                };
            upsert_image(&mut transaction, &recipe_id, &image).await?;
        }
        _ => return Err(AppError::Data("invalid revision image action".into())),
    }
    sqlx::query(
        "UPDATE recipe_revisions SET status = 'accepted', resolved_at = ?, updated_at = ?
         WHERE id = ? AND owner_id = ? AND status = 'pending'",
    )
    .bind(Utc::now().timestamp())
    .bind(Utc::now().timestamp())
    .bind(&revision_id)
    .bind(&user.id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Json(
        load_owned_recipe(&state.pool, &user.id, &recipe_id).await?,
    ))
}

pub(crate) async fn reject_revision(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(revision_id): Path<String>,
) -> AppResult<StatusCode> {
    let result = sqlx::query(
        "UPDATE recipe_revisions SET status = 'rejected', resolved_at = ?, updated_at = ?
         WHERE id = ? AND owner_id = ? AND status = 'pending'",
    )
    .bind(Utc::now().timestamp())
    .bind(Utc::now().timestamp())
    .bind(revision_id)
    .bind(user.id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn get_revision_image(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(revision_id): Path<String>,
) -> AppResult<Response<Body>> {
    let row = sqlx::query(
        "SELECT image_bytes, image_sha256
         FROM recipe_revisions
         WHERE id = ? AND owner_id = ? AND status = 'pending' AND image_action = 'replace'",
    )
    .bind(&revision_id)
    .bind(&user.id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let bytes: Vec<u8> = row
        .get::<Option<Vec<u8>>, _>("image_bytes")
        .ok_or(AppError::NotFound)?;
    let sha: Vec<u8> = row
        .get::<Option<Vec<u8>>, _>("image_sha256")
        .ok_or(AppError::NotFound)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "image/jpeg")
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .header(ETAG, format!("\"{}\"", hex(&sha)))
        .header(CACHE_CONTROL, "private, no-store")
        .header(
            "x-content-type-options",
            HeaderValue::from_static("nosniff"),
        )
        .body(Body::from(bytes))
        .map_err(|_| AppError::Data("invalid revision image response".into()))
}

async fn update_recipe_direct(
    transaction: &mut Transaction<'_, Sqlite>,
    principal: &TokenPrincipal,
    recipe_id: &str,
    expected_version: i64,
    current_visibility: &str,
    input: &RecipeInput,
) -> AppResult<()> {
    let result = sqlx::query(
        "UPDATE recipes SET title = ?, description = ?, instructions = ?,
           visibility = ?, difficulty = ?, estimated_calories = ?,
           source_url = ?, source_title = ?, source_author = ?, source_license = ?,
           source_attribution = ?,
           version = version + 1, updated_at = CURRENT_TIMESTAMP
         WHERE id = ? AND owner_id = ? AND visibility = ? AND version = ?",
    )
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(input.instructions.trim())
    .bind(&input.visibility)
    .bind(input.difficulty)
    .bind(input.estimated_calories)
    .bind(crate::routes::optional_text(&input.source_url))
    .bind(crate::routes::optional_text(&input.source_title))
    .bind(crate::routes::optional_text(&input.source_author))
    .bind(crate::routes::optional_text(&input.source_license))
    .bind(crate::routes::optional_text(&input.source_attribution))
    .bind(recipe_id)
    .bind(&principal.owner_id)
    .bind(current_visibility)
    .bind(expected_version)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() != 1 {
        return Err(AppError::Conflict(
            "recipe changed; reload and retry".into(),
        ));
    }
    replace_ingredients(
        transaction,
        recipe_id,
        &principal.owner_id,
        &input.ingredients,
    )
    .await?;
    record_token_audit(
        transaction,
        principal,
        if input.visibility == "public" {
            if current_visibility == "private" {
                "recipe.publish"
            } else {
                "recipe.publish.update"
            }
        } else {
            "recipe.update"
        },
        Some(recipe_id),
        Utc::now().timestamp(),
    )
    .await
}

async fn create_revision(
    transaction: &mut Transaction<'_, Sqlite>,
    principal: &TokenPrincipal,
    recipe_id: &str,
    expected_version: i64,
    input: &RecipeInput,
    image_action: &str,
    image: Option<&ProcessedImage>,
) -> AppResult<String> {
    let pending: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM recipe_revisions WHERE recipe_id = ? AND status = 'pending'",
    )
    .bind(recipe_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if pending.is_some() {
        return Err(AppError::Conflict(
            "this public recipe already has a pending AI revision".into(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().timestamp();
    validate_ingredient_selections(transaction, &principal.owner_id, &input.ingredients).await?;
    sqlx::query(
        "INSERT INTO recipe_revisions
         (id, recipe_id, owner_id, token_id, expected_version, title, description,
          instructions, difficulty, estimated_calories, source_url, source_title, source_author,
          source_license, source_attribution, image_action, image_media_type, image_bytes, image_width,
          image_height, image_sha256, image_alt_text, image_source_url,
          image_source_license, image_source_attribution, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(recipe_id)
    .bind(&principal.owner_id)
    .bind(&principal.token_id)
    .bind(expected_version)
    .bind(input.title.trim())
    .bind(input.description.trim())
    .bind(input.instructions.trim())
    .bind(input.difficulty)
    .bind(input.estimated_calories)
    .bind(crate::routes::optional_text(&input.source_url))
    .bind(crate::routes::optional_text(&input.source_title))
    .bind(crate::routes::optional_text(&input.source_author))
    .bind(crate::routes::optional_text(&input.source_license))
    .bind(crate::routes::optional_text(&input.source_attribution))
    .bind(image_action)
    .bind(image.map(|_| "image/jpeg"))
    .bind(image.map(|value| value.bytes.as_slice()))
    .bind(image.map(|value| i64::from(value.width)))
    .bind(image.map(|value| i64::from(value.height)))
    .bind(image.map(|value| value.sha256.as_slice()))
    .bind(image.map(|value| value.metadata.alt_text.as_str()))
    .bind(image.and_then(|value| value.metadata.source_url.as_deref()))
    .bind(image.and_then(|value| value.metadata.source_license.as_deref()))
    .bind(image.and_then(|value| value.metadata.source_attribution.as_deref()))
    .bind(now)
    .bind(now)
    .execute(&mut **transaction)
    .await?;
    insert_revision_ingredients(transaction, &id, &input.ingredients).await?;
    record_token_audit(
        transaction,
        principal,
        if image_action == "keep" {
            "recipe.revision.create"
        } else {
            "recipe.image.revision.create"
        },
        Some(recipe_id),
        now,
    )
    .await?;
    Ok(id)
}

async fn owned_recipe_visibility(
    transaction: &mut Transaction<'_, Sqlite>,
    owner_id: &str,
    recipe_id: &str,
    expected_version: i64,
) -> AppResult<String> {
    let row = sqlx::query("SELECT visibility, version FROM recipes WHERE id = ? AND owner_id = ?")
        .bind(recipe_id)
        .bind(owner_id)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(AppError::NotFound)?;
    if row.get::<i64, _>("version") != expected_version {
        return Err(AppError::Conflict(
            "recipe changed; reload and retry".into(),
        ));
    }
    Ok(row.get("visibility"))
}

async fn load_owned_recipe(pool: &SqlitePool, owner_id: &str, id: &str) -> AppResult<Recipe> {
    load_visible_recipes(pool, owner_id, Some(id))
        .await?
        .into_iter()
        .find(|recipe| recipe.is_owner)
        .ok_or(AppError::NotFound)
}

async fn load_recipe_input_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
) -> AppResult<RecipeInput> {
    let row = sqlx::query(
        "SELECT title, description, instructions, difficulty, estimated_calories,
                source_url, source_title, source_author, source_license, source_attribution
         FROM recipes WHERE id = ?",
    )
    .bind(recipe_id)
    .fetch_one(&mut **transaction)
    .await?;
    let ingredients = load_ingredients_tx(transaction, recipe_id).await?;
    Ok(RecipeInput {
        title: row.get("title"),
        description: row.get("description"),
        instructions: row.get("instructions"),
        visibility: "private".into(),
        difficulty: row.get("difficulty"),
        estimated_calories: row.get("estimated_calories"),
        source_url: row.get("source_url"),
        source_title: row.get("source_title"),
        source_author: row.get("source_author"),
        source_license: row.get("source_license"),
        source_attribution: row.get("source_attribution"),
        ingredients,
    })
}

fn recipe_input(input: &AiRecipeInput, visibility: &str) -> RecipeInput {
    RecipeInput {
        title: input.title.clone(),
        description: input.description.clone(),
        instructions: input.instructions.clone(),
        visibility: visibility.into(),
        difficulty: input.difficulty,
        estimated_calories: input.estimated_calories,
        source_url: input.source_url.clone(),
        source_title: input.source_title.clone(),
        source_author: input.source_author.clone(),
        source_license: input.source_license.clone(),
        source_attribution: input.source_attribution.clone(),
        ingredients: input.ingredients.clone(),
    }
}

fn validate_ai_visibility(visibility: &str) -> AppResult<&str> {
    match visibility {
        "private" | "public" => Ok(visibility),
        _ => Err(AppError::Validation(
            "visibility must be private or public".into(),
        )),
    }
}

fn principal_has_scope(principal: &TokenPrincipal, scope: &str) -> bool {
    principal.scopes.iter().any(|candidate| candidate == scope)
}

fn principal_can_publish(principal: &TokenPrincipal) -> bool {
    principal_has_scope(principal, SCOPE_RECIPES_READ)
        && principal_has_scope(principal, SCOPE_RECIPES_WRITE)
        && principal_has_scope(principal, SCOPE_RECIPES_PUBLISH)
}

async fn insert_revision_ingredients(
    transaction: &mut Transaction<'_, Sqlite>,
    revision_id: &str,
    ingredients: &[IngredientInput],
) -> AppResult<()> {
    for (index, ingredient) in ingredients.iter().enumerate() {
        sqlx::query(
            "INSERT INTO recipe_revision_ingredients
             (id, revision_id, name, quantity_note, canonical_ingredient_id, sort_order)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(revision_id)
        .bind(ingredient.name.trim())
        .bind(ingredient.quantity_note.trim())
        .bind(&ingredient.canonical_ingredient_id)
        .bind(
            i64::try_from(index)
                .map_err(|_| AppError::Validation("too many ingredients".into()))?,
        )
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn load_revision_ingredients_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    revision_id: &str,
) -> AppResult<Vec<IngredientInput>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, quantity_note, canonical_ingredient_id
         FROM recipe_revision_ingredients WHERE revision_id = ? ORDER BY sort_order",
    )
    .bind(revision_id)
    .fetch_all(&mut **transaction)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(name, quantity_note, canonical_ingredient_id)| IngredientInput {
                name,
                quantity_note,
                canonical_ingredient_id,
            },
        )
        .collect())
}

async fn load_revision(
    pool: &SqlitePool,
    owner_id: &str,
    revision_id: &str,
) -> AppResult<RecipeRevision> {
    let row = sqlx::query(
        "SELECT id, recipe_id, expected_version, title, description, instructions,
                difficulty, estimated_calories, source_url, source_title, source_author,
                source_license, source_attribution,
                image_action, image_alt_text, image_source_url, image_source_license,
                image_source_attribution, status, created_at, updated_at
         FROM recipe_revisions WHERE id = ? AND owner_id = ?",
    )
    .bind(revision_id)
    .bind(owner_id)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let ingredients: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, quantity_note, canonical_ingredient_id
         FROM recipe_revision_ingredients WHERE revision_id = ? ORDER BY sort_order",
    )
    .bind(revision_id)
    .fetch_all(pool)
    .await?;
    Ok(RecipeRevision {
        id: row.get("id"),
        recipe_id: row.get("recipe_id"),
        expected_version: row.get("expected_version"),
        title: row.get("title"),
        description: row.get("description"),
        instructions: row.get("instructions"),
        difficulty: row.get("difficulty"),
        estimated_calories: row.get("estimated_calories"),
        source_url: row.get("source_url"),
        source_title: row.get("source_title"),
        source_author: row.get("source_author"),
        source_license: row.get("source_license"),
        source_attribution: row.get("source_attribution"),
        ingredients: ingredients
            .into_iter()
            .map(
                |(name, quantity_note, canonical_ingredient_id)| IngredientInput {
                    name,
                    quantity_note,
                    canonical_ingredient_id,
                },
            )
            .collect(),
        image_action: row.get("image_action"),
        image_alt_text: row.get("image_alt_text"),
        image_url: (row.get::<String, _>("image_action") == "replace")
            .then(|| format!("/api/recipe-revisions/{revision_id}/cover")),
        image_source_url: row.get("image_source_url"),
        image_source_license: row.get("image_source_license"),
        image_source_attribution: row.get("image_source_attribution"),
        status: row.get("status"),
        created_at: timestamp(row.get("created_at")),
        updated_at: timestamp(row.get("updated_at")),
    })
}

async fn load_ingredients_tx(
    transaction: &mut Transaction<'_, Sqlite>,
    recipe_id: &str,
) -> AppResult<Vec<IngredientInput>> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT name, quantity_note, canonical_ingredient_id
         FROM recipe_ingredients WHERE recipe_id = ? ORDER BY sort_order",
    )
    .bind(recipe_id)
    .fetch_all(&mut **transaction)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(name, quantity_note, canonical_ingredient_id)| IngredientInput {
                name,
                quantity_note,
                canonical_ingredient_id,
            },
        )
        .collect())
}

async fn audit(
    pool: &SqlitePool,
    principal: &TokenPrincipal,
    action: &str,
    recipe_id: Option<&str>,
) -> AppResult<()> {
    let mut transaction = pool.begin().await?;
    record_token_audit(
        &mut transaction,
        principal,
        action,
        recipe_id,
        Utc::now().timestamp(),
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

fn idempotency_key(headers: &HeaderMap) -> AppResult<String> {
    let value = headers
        .get(IDEMPOTENCY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| (8..=128).contains(&value.len()))
        .filter(|value| value.bytes().all(|byte| byte.is_ascii_graphic()))
        .ok_or_else(|| {
            AppError::Validation(
                "Idempotency-Key must contain 8 to 128 visible ASCII characters".into(),
            )
        })?;
    Ok(value.to_string())
}

fn expected_version_header(headers: &HeaderMap) -> AppResult<i64> {
    let value = headers
        .get(VERSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("\"recipe-v"))
        .and_then(|value| value.strip_suffix('"'))
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            AppError::Validation("If-Match must use the form \"recipe-v<version>\"".into())
        })?;
    Ok(value)
}

fn timestamp(value: i64) -> String {
    chrono::DateTime::from_timestamp(value, 0)
        .unwrap_or(chrono::DateTime::<Utc>::UNIX_EPOCH)
        .to_rfc3339()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|value| format!("{value:02x}")).collect()
}
